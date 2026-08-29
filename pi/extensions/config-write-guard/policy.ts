import { homedir, userInfo } from "node:os";

import { bashCommandWritesProtectedPath } from "./bash-intent.ts";
import { isProtectedConfigPath } from "./paths.ts";

type FileToolInput = { path: string };
type BashToolInput = { command: string };

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
	return new RegExp(`(?:${escapedHome}|\\$HOME|\\$\\{HOME\\}|${tildeForms})/+(?:\\.agents|\\.claude|\\.codex|\\.pi|\\.config/herdr)(?:/|\\b)`);
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
	toolName: "edit" | "write" | "bash",
	input: FileToolInput | BashToolInput,
	home = homedir(),
	username = currentUsername(),
): string | undefined {
	if ((toolName === "edit" || toolName === "write") && isProtectedConfigPath((input as FileToolInput).path, home)) {
		return `Blocked a direct agent-config write to ${(input as FileToolInput).path}. Edit the source in the agents worktree, then run install.sh.`;
	}
	if (toolName === "bash" && bashCommandWritesProtectedPath((input as BashToolInput).command, pathReferencePattern(home, username))) {
		return "Blocked a shell command that writes an agent-config destination. Edit the source in the agents worktree, then run install.sh. Reading it (cat, grep, ls, ...) is fine.";
	}
	return undefined;
}
