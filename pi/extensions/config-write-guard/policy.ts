import { createHash } from "node:crypto";
import { readFileSync, realpathSync } from "node:fs";
import { homedir, userInfo } from "node:os";
import { relative, resolve } from "node:path";

import { bashCommandWritesProtectedPath, classifyCheckoutCommand, staticZshPayload } from "./bash-intent.ts";
import { isPathInsideRoot, isProtectedConfigPath } from "./paths.ts";

type FileToolInput = { path: string };
type BashToolInput = { command: string };
export type AgentToolInput = {
	subagent_type?: string;
	isolation?: "off" | "worktree";
};

export type GuardContext = {
	cwd: string;
	repositoryRoot: string;
	isRepositoryClean: () => boolean;
	worktreeRoots: () => string[];
};

const READ_ONLY_AGENT_TYPES = new Set([
	"Explore",
	"Plan",
	"anchor-verifier",
	"code-reviewer",
	"log-summarizer",
	"web-research-summarizer",
]);

const REVIEWED_SELECTOR_SHA256 = "4e1341d3824081cfa1f9fc0408167e686b621b984ec5380338e132ff8dc4d912";

function commandWithReadOnlySelector(command: string, home: string): string {
	const inner = staticZshPayload(command);
	const payload = inner ?? command;
	if (/[^A-Za-z0-9_/.\- \t\n;&]/.test(payload) || /(?<!&)&(?!&)/.test(payload)) return command;
	const classified = payload.split(/(&&|;|\n)/).map((segment) => {
		const match = /^\s*(?:(?:\/bin\/)?sh\s+)?(\/[^\s]+\/next-issue\.sh)\s*$/.exec(segment);
		if (match === null) return segment;
		try {
			const installed = realpathSync(resolve(home, ".agents/skills/task-graph/scripts/next-issue.sh"));
			if (realpathSync(match[1]) !== installed) return segment;
			if (createHash("sha256").update(readFileSync(installed)).digest("hex") !== REVIEWED_SELECTOR_SHA256) return segment;
			return `cat ${match[1]}`;
		} catch {
			return segment;
		}
	}).join("");
	return inner === undefined ? classified : `/bin/zsh -lc '${classified}'`;
}

function currentUsername(): string | undefined {
	try {
		return userInfo().username;
	} catch {
		return undefined;
	}
}

function escapeRegExp(literal: string): string {
	return literal.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

// `~<username>/...` and a doubled `/` name the same protected path as `home/...` —
// both must match too.
function pathReferencePattern(home: string, username = currentUsername()): RegExp {
	const escapedHome = escapeRegExp(home);
	const tildeForms = username !== undefined ? `~(?:${escapeRegExp(username)})?` : "~";
	return new RegExp(`(?:${escapedHome}|\\$HOME|\\$\\{HOME\\}|${tildeForms})/+(?:(?:\\.agents|\\.claude|\\.codex|\\.pi|\\.config/herdr)(?:/|\\b)|\\.config/simslim(?:/|(?![A-Za-z0-9._-])))`);
}

function shellPathPattern(path: string): string {
	return path.split("/").map(escapeRegExp).join("/+(?:\\./+)*");
}

function repositoryReferencePattern(repositoryRoot: string, cwd: string, home: string, username = currentUsername()): RegExp {
	const forms = [shellPathPattern(repositoryRoot)];
	const relativeRoot = relative(cwd, repositoryRoot);
	if (relativeRoot !== "") forms.push(shellPathPattern(relativeRoot));
	if (repositoryRoot.startsWith(`${home}/`)) {
		const relativeHomeRoot = shellPathPattern(repositoryRoot.slice(home.length + 1));
		const tildeForms = username === undefined ? "~" : `~(?:${escapeRegExp(username)})?`;
		forms.push(`(?:\\$HOME|\\$\\{HOME\\}|${tildeForms})["']?/+(?:\\./+)*${relativeHomeRoot}`);
	}
	return new RegExp(`(?:${forms.join("|")})(?=/+|[\\s'"|;&<>]|$)`);
}

function mainCheckoutBlockReason(): string {
	return "Blocked a write to the primary main checkout. Retry the tool call from an isolated worktree.";
}

function blocksChildAgent(input: AgentToolInput): boolean {
	if (input.isolation === "worktree") return false;
	return input.subagent_type === undefined || !READ_ONLY_AGENT_TYPES.has(input.subagent_type);
}

function pathsEqual(first: string, second: string): boolean {
	return isPathInsideRoot(first, second) && isPathInsideRoot(second, first);
}

function isPrimaryCheckoutPath(path: string, cwd: string, guard: GuardContext): boolean {
	if (!isPathInsideRoot(path, guard.repositoryRoot, cwd)) return false;
	return !guard.worktreeRoots().some((root) => !pathsEqual(root, guard.repositoryRoot) && isPathInsideRoot(path, root, cwd));
}

function shellPath(word: string, cwd: string, home: string, username: string | undefined, allowBare = false): string | undefined {
	let candidate = word.replace(/^[12&]*>>?/, "").replace(/^['"]|['"]$/g, "");
	const separator = candidate.indexOf("=");
	if (candidate.startsWith("-") && separator >= 0) candidate = candidate.slice(separator + 1);
	candidate = candidate.replace(/^\$HOME|^\$\{HOME\}/, home);
	if (candidate === "~" || candidate === `$HOME` || candidate === "${HOME}") candidate = home;
	if (candidate.startsWith("~/")) candidate = `${home}/${candidate.slice(2)}`;
	if (username !== undefined && candidate.startsWith(`~${username}/`)) candidate = `${home}/${candidate.slice(username.length + 2)}`;
	if (!allowBare && !candidate.includes("/") && candidate !== "." && candidate !== "..") return undefined;
	return resolve(cwd, candidate);
}

function shellCommandReferencesPrimaryCheckout(command: string, guard: GuardContext, home: string, username: string | undefined): boolean {
	let cwd = guard.cwd;
	for (const segment of (staticZshPayload(command) ?? command).split(/&&|\|\||[;\n|]/)) {
		const words = segment.match(/"[^"]*"|'[^']*'|[^\s<>]+/g) ?? [];
		const leading = words[0]?.split("/").pop();
		for (const word of words.slice(1)) {
			const path = shellPath(word, cwd, home, username);
			if (path !== undefined && isPrimaryCheckoutPath(path, cwd, guard)) return true;
		}
		if (leading === "cd" && words[1] !== undefined) {
			cwd = shellPath(words[1], cwd, home, username, true) ?? cwd;
			if (isPrimaryCheckoutPath(cwd, cwd, guard)) return true;
		}
	}
	return false;
}

function commandWithoutWorktreeReferences(command: string, guard: GuardContext, home: string, username: string | undefined): string {
	return guard.worktreeRoots().reduce((remaining, root) => {
		if (pathsEqual(root, guard.repositoryRoot)) return remaining;
		const pattern = repositoryReferencePattern(root, guard.cwd, home, username);
		const rootedPath = new RegExp(`(${pattern.source})([^\\s'"|;&<>]*)`, "g");
		return remaining.replace(rootedPath, (match, _reference: string, suffix: string) => {
			const target = resolve(root, suffix.replace(/^\/+/, ""));
			return isPathInsideRoot(target, root) ? "__worktree__" : match;
		});
	}, command);
}

function blockedMainCheckoutShell(
	command: string,
	isMainCheckoutCwd: boolean,
	guard: GuardContext,
	home: string,
	username: string | undefined,
): string | undefined {
	if (!isMainCheckoutCwd) {
		const protectedCommand = commandWithoutWorktreeReferences(command, guard, home, username);
		const repositoryPattern = repositoryReferencePattern(guard.repositoryRoot, guard.cwd, home, username);
		if (!repositoryPattern.test(protectedCommand) && !shellCommandReferencesPrimaryCheckout(command, guard, home, username)) return undefined;
		const classification = classifyCheckoutCommand(protectedCommand);
		return bashCommandWritesProtectedPath(protectedCommand, repositoryPattern) || classification !== "read"
			? mainCheckoutBlockReason()
			: undefined;
	}
	const classification = classifyCheckoutCommand(command);
	if (classification === "clean-fast-forward-pull") {
		return guard.isRepositoryClean() ? undefined : "Blocked `git pull --ff-only` because the primary main checkout is not clean.";
	}
	return classification === "write-or-unknown" ? mainCheckoutBlockReason() : undefined;
}

function blockedMainCheckoutToolCall(
	toolName: "edit" | "write" | "bash" | "Agent",
	input: unknown,
	guard: GuardContext,
	home: string,
	username: string | undefined,
): string | undefined {
	const isMainCheckoutCwd = isPrimaryCheckoutPath(guard.cwd, guard.cwd, guard);
	if ((toolName === "edit" || toolName === "write") && isPrimaryCheckoutPath((input as FileToolInput).path, guard.cwd, guard)) {
		return mainCheckoutBlockReason();
	}
	if (toolName === "Agent" && isMainCheckoutCwd && blocksChildAgent(input as AgentToolInput)) {
		return "Blocked a write-capable child agent in the primary main checkout. Dispatch it with worktree isolation.";
	}
	return toolName === "bash"
		? blockedMainCheckoutShell((input as BashToolInput).command, isMainCheckoutCwd, guard, home, username)
		: undefined;
}

/**
 * Returns the block reason for a managed-destination tool call.
 *
 * @param toolName The Pi tool name.
 * @param input The file path or shell command supplied to the tool.
 * @param home The home directory that contains the agent destinations.
 * @param username The current user's name, for the `~<username>/...` path form.
 * @returns A block reason, or undefined when the call is outside the policy.
 * @throws Never.
 */
export function blockedConfigToolCall(
	toolName: "edit" | "write" | "bash" | "Agent",
	input: unknown,
	home = homedir(),
	username = currentUsername(),
	guard?: GuardContext,
): string | undefined {
	if (toolName === "bash") input = { command: commandWithReadOnlySelector((input as BashToolInput).command, home) };
	const mainCheckoutReason = guard === undefined
		? undefined
		: blockedMainCheckoutToolCall(toolName, input, guard, home, username);
	if (mainCheckoutReason !== undefined) return mainCheckoutReason;

	if ((toolName === "edit" || toolName === "write") && isProtectedConfigPath((input as FileToolInput).path, home)) {
		return `Blocked a direct agent-config write to ${(input as FileToolInput).path}. Edit the source in the agents worktree, then run install.sh.`;
	}
	if (toolName === "bash" && bashCommandWritesProtectedPath((input as BashToolInput).command, pathReferencePattern(home, username))) {
		return "Blocked a shell command that writes an agent-config destination. Edit the source in the agents worktree, then run install.sh. Reading it (cat, grep, ls, ...) is fine.";
	}
	return undefined;
}
