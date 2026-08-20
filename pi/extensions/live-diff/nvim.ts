import type { Exec } from "./engine.ts";

/**
 * Focus the herdr editor tab of the workspace owning cwd and open the file
 * in its nvim pane.
 *
 * @param exec command runner
 * @param cwd repository worktree root
 * @param path file path relative to cwd
 * @returns true when a workspace, editor tab, and nvim pane were all found
 *   and the open sequence was sent; false otherwise (never throws)
 */
export function openInNvim(
	exec: Exec,
	cwd: string,
	path: string,
): Promise<boolean> {
	throw new Error("unimplemented");
}
