import { isToolCallEventType, type ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { realpathSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { blockedConfigToolCall, type AgentToolInput } from "./config-write-guard/policy.ts";

type WorktreeRoots = (repositoryRoot: string, spawn: typeof import("node:child_process").spawnSync) => string[];

function repositoryWorktreeRoots(repositoryRoot: string, spawn: typeof import("node:child_process").spawnSync): string[] {
	const result = spawn("git", ["-C", repositoryRoot, "worktree", "list", "--porcelain"], { encoding: "utf8" });
	if (result.status !== 0) return [repositoryRoot];
	return result.stdout
		.split("\n")
		.filter((line) => line.startsWith("worktree "))
		.map((line) => line.slice("worktree ".length));
}

export default function configWriteGuard(pi: ExtensionAPI, getWorktreeRoots: WorktreeRoots = repositoryWorktreeRoots): void {
	const extensionPath = realpathSync(fileURLToPath(import.meta.url));
	const repositoryRoot = resolve(dirname(extensionPath), "../..");
	let roots: string[] | undefined;
	let spawn: typeof import("node:child_process").spawnSync | undefined;
	let sessionStartingDirectory: string | undefined;

	pi.on("resources_discover", () => ({
		skillPaths: [resolve(repositoryRoot, "skills")],
		themePaths: [resolve(repositoryRoot, "pi/themes")],
	}));

	pi.on("session_start", (_event, ctx) => {
		sessionStartingDirectory = ctx.cwd;
	});

	pi.on("session_shutdown", () => {
		sessionStartingDirectory = undefined;
	});

	pi.on("tool_call", async (event, ctx) => {
		if (
			isToolCallEventType("edit", event) ||
			isToolCallEventType("write", event) ||
			isToolCallEventType("bash", event) ||
			isToolCallEventType<"Agent", AgentToolInput>("Agent", event)
		) {
			const spawnSync = spawn ??= (await import("node:child_process")).spawnSync;
			const reason = blockedConfigToolCall(event.toolName, event.input, undefined, undefined, {
				cwd: ctx.cwd,
				repositoryRoot,
				sessionStartingDirectory,
				isRepositoryClean: () => {
					const result = spawnSync("git", ["-C", repositoryRoot, "status", "--porcelain"], { encoding: "utf8" });
					return result.status === 0 && result.stdout.trim() === "";
				},
				worktreeRoots: () => (roots ??= getWorktreeRoots(repositoryRoot, spawnSync)),
			});
			if (reason !== undefined) return { block: true, reason };
		}
	});
}
