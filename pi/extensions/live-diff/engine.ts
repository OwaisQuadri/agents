import type { DiffStats, Hunk, Snapshot } from "./types.ts";

export type Exec = (
	command: string,
	args: string[],
	options?: { cwd?: string; env?: Record<string, string> },
) => Promise<{ code: number; stdout: string; stderr: string }>;

/**
 * Capture an immutable tree of the full worktree (tracked + untracked,
 * .gitignore respected) without touching the real index or worktree.
 *
 * @param exec command runner
 * @param cwd repository worktree root
 * @returns Snapshot with the written tree sha and the HEAD tree sha as baseline
 * @throws Error when cwd is not a git worktree or git exits nonzero
 */
export function captureSnapshot(exec: Exec, cwd: string): Promise<Snapshot> {
	// TODO(AGNT-0015.T02): temp GIT_INDEX_FILE (absolute) read-tree/add -A/write-tree;
	// empty-tree baseline for zero-commit repos; delete temp index; no ref.
	throw new Error("unimplemented");
}

/**
 * Diff a snapshot tree against the current worktree state.
 *
 * @param exec command runner
 * @param cwd repository worktree root
 * @param baseTreeSha tree to diff from
 * @param maxFiles truncation cap for the row list
 * @returns per-file stats with renames (-M) and binary detection
 * @throws Error when git exits nonzero
 */
export function diffStats(
	exec: Exec,
	cwd: string,
	baseTreeSha: string,
	maxFiles: number,
): Promise<DiffStats> {
	// TODO(AGNT-0015.T03): writeWorktreeTree then diff-tree -r -M --numstat/-z +
	// --name-status join; binary "- -"; renamedFrom; maxFiles cap sets isTruncated.
	throw new Error("unimplemented");
}

/**
 * Unified-diff hunks for one file between a snapshot tree and the worktree.
 *
 * @param exec command runner
 * @param cwd repository worktree root
 * @param baseTreeSha tree to diff from
 * @param path file path relative to cwd
 * @returns parsed hunks; empty array for a binary file
 * @throws Error when git exits nonzero
 */
export function filePatch(
	exec: Exec,
	cwd: string,
	baseTreeSha: string,
	path: string,
): Promise<Hunk[]> {
	// TODO(AGNT-0015.T04): diff-tree -p single path → Hunk[]; binary → []; tolerate
	// "\ No newline at end of file".
	throw new Error("unimplemented");
}
