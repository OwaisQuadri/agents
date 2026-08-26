import { homedir } from "node:os";

import { isProtectedConfigPath } from "./paths.ts";

type FileToolInput = { path: string };
type BashToolInput = { command: string };

function pathReferencePattern(home: string): RegExp {
	const escapedHome = home.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
	return new RegExp(`(?:${escapedHome}|\\$HOME|\\$\\{HOME\\}|~)/(?:\\.agents|\\.claude|\\.codex|\\.pi)(?:/|\\b)`);
}

/**
 * Returns the block reason for a managed-destination tool call.
 *
 * @param toolName The Pi tool name.
 * @param input The file path or shell command supplied to the tool.
 * @param home The home directory that contains the agent destinations.
 * @returns A block reason, or undefined when the call is outside the policy.
 * @throws Never.
 */
export function blockedConfigToolCall(
	toolName: "edit" | "write" | "bash",
	input: FileToolInput | BashToolInput,
	home = homedir(),
): string | undefined {
	if ((toolName === "edit" || toolName === "write") && isProtectedConfigPath((input as FileToolInput).path, home)) {
		return `Blocked a direct agent-config write to ${(input as FileToolInput).path}. Edit the source in the agents worktree, then run install.sh.`;
	}
	if (toolName === "bash" && pathReferencePattern(home).test((input as BashToolInput).command)) {
		return "Blocked a shell command that references an agent-config destination. Edit the source in the agents worktree, then run install.sh.";
	}
	return undefined;
}
