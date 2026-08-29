import { mkdtemp, readdir, rm, stat } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import type {
	DiffStats,
	FileChange,
	FileChangeKind,
	Hunk,
	Snapshot,
} from "./types.ts";

export const TEMP_INDEX_PREFIX = "live-diff-index-";

const DEFAULT_BRANCH_CANDIDATES = [
	"origin/main",
	"origin/master",
	"main",
	"master",
];

const STALE_TEMP_AGE_MS = 60 * 60 * 1000;

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
export async function captureSnapshot(
	exec: Exec,
	cwd: string,
): Promise<Snapshot> {
	const baselineSha = await resolveBaselineTree(exec, cwd);
	const treeSha = await writeWorktreeTree(exec, cwd);
	return { treeSha, baselineSha, captureTs: Date.now() };
}

/**
 * Write a tree object covering the worktree's current tracked + untracked
 * content through a temporary index, leaving the real index untouched.
 *
 * @param exec command runner
 * @param cwd repository worktree root
 * @returns sha of the written tree
 * @throws Error when git exits nonzero
 */
export async function writeWorktreeTree(
	exec: Exec,
	cwd: string,
): Promise<string> {
	await sweepStaleTempDirs();
	const tempDir = await mkdtemp(join(tmpdir(), TEMP_INDEX_PREFIX));
	try {
		const env = { GIT_INDEX_FILE: join(tempDir, "index") };
		const isHeadPresent = await headExists(exec, cwd);
		if (isHeadPresent) {
			await run(exec, cwd, ["read-tree", "HEAD"], env);
		}
		await run(exec, cwd, ["add", "-A"], env);
		return (await run(exec, cwd, ["write-tree"], env)).trim();
	} finally {
		await rm(tempDir, { recursive: true, force: true });
	}
}

async function sweepStaleTempDirs(): Promise<void> {
	const root = tmpdir();
	const cutoff = Date.now() - STALE_TEMP_AGE_MS;
	let entries: string[];
	try {
		entries = await readdir(root);
	} catch {
		return;
	}
	for (const entry of entries) {
		if (!entry.startsWith(TEMP_INDEX_PREFIX)) {
			continue;
		}
		const target = join(root, entry);
		try {
			const info = await stat(target);
			if (!info.isDirectory() || info.mtimeMs >= cutoff) {
				continue;
			}
			await rm(target, { recursive: true, force: true });
		} catch {
			continue;
		}
	}
}

async function resolveBaselineTree(exec: Exec, cwd: string): Promise<string> {
	const head = await exec("git", ["rev-parse", "HEAD^{tree}"], { cwd });
	if (head.code === 0) {
		return head.stdout.trim();
	}
	return (await run(exec, cwd, ["hash-object", "-t", "tree", "/dev/null"])).trim();
}

async function headExists(exec: Exec, cwd: string): Promise<boolean> {
	const result = await exec("git", ["rev-parse", "--verify", "HEAD"], { cwd });
	return result.code === 0;
}

async function run(
	exec: Exec,
	cwd: string,
	args: string[],
	env?: Record<string, string>,
): Promise<string> {
	const result = await exec("git", args, env ? { cwd, env } : { cwd });
	if (result.code !== 0) {
		throw new Error(`git ${args[0]} failed: ${result.stderr}`);
	}
	return result.stdout;
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
export async function diffStats(
	exec: Exec,
	cwd: string,
	baseTreeSha: string,
	maxFiles: number,
): Promise<DiffStats> {
	const currentTreeSha = await writeWorktreeTree(exec, cwd);
	const numstatOut = await run(exec, cwd, [
		"diff-tree", "-r", "-M", "--numstat", "-z", baseTreeSha, currentTreeSha,
	]);
	const nameStatusOut = await run(exec, cwd, [
		"diff-tree", "-r", "-M", "--name-status", "-z", baseTreeSha, currentTreeSha,
	]);
	const statusByPath = parseNameStatus(nameStatusOut);
	const files: FileChange[] = [];
	let additions = 0;
	let deletions = 0;
	for (const row of parseNumstat(numstatOut)) {
		const status = statusByPath.get(row.path);
		const change: FileChange = {
			path: row.path,
			renamedFrom: status?.renamedFrom ?? row.renamedFrom,
			kind: row.isBinary ? "binary" : (status?.kind ?? "modified"),
			additions: row.additions,
			deletions: row.deletions,
			isBinary: row.isBinary,
			origin: "uncommitted",
		};
		if (!change.isBinary) {
			additions += change.additions;
			deletions += change.deletions;
		}
		files.push(change);
	}
	const isTruncated = files.length > maxFiles;
	return {
		files: isTruncated ? files.slice(0, maxFiles) : files,
		additions,
		deletions,
		isTruncated,
	};
}

/**
 * Resolve the tree of the merge-base between HEAD and the default branch.
 *
 * @param exec command runner
 * @param cwd repository worktree root
 * @returns the merge-base commit's tree sha, or null when no branch point
 *   applies — a detached HEAD, a repo sitting on its own default branch, a
 *   repo with no commits, or no resolvable default branch
 */
export async function resolveBranchPointTree(
	exec: Exec,
	cwd: string,
): Promise<string | null> {
	const commit = await resolveBranchPointMergeBase(exec, cwd);
	if (commit === null) {
		return null;
	}
	try {
		const tree = await exec("git", ["rev-parse", `${commit}^{tree}`], { cwd });
		return tree.code === 0 && tree.stdout.trim() !== "" ? tree.stdout.trim() : null;
	} catch {
		return null;
	}
}

/**
 * Resolve the commit sha of the merge-base between HEAD and the default
 * branch — the "branch point" commits after it belong to this branch alone.
 *
 * @param exec command runner
 * @param cwd repository worktree root
 * @returns the merge-base commit sha, or null when no branch point applies —
 *   a detached HEAD, a repo sitting on its own default branch, a repo with
 *   no commits, or no resolvable default branch
 */
export async function resolveBranchPointCommit(
	exec: Exec,
	cwd: string,
): Promise<string | null> {
	return resolveBranchPointMergeBase(exec, cwd);
}

async function resolveBranchPointMergeBase(
	exec: Exec,
	cwd: string,
): Promise<string | null> {
	try {
		const head = await exec("git", ["rev-parse", "--verify", "--quiet", "HEAD"], {
			cwd,
		});
		if (head.code !== 0) {
			return null;
		}
		const current = await exec("git", ["branch", "--show-current"], { cwd });
		const currentBranch = current.code === 0 ? current.stdout.trim() : "";
		if (currentBranch === "") {
			return null;
		}
		for (const candidate of await branchCandidates(exec, cwd)) {
			if (candidate === currentBranch || candidate === `origin/${currentBranch}`) {
				continue;
			}
			const exists = await exec(
				"git",
				["rev-parse", "--verify", "--quiet", `${candidate}^{commit}`],
				{ cwd },
			);
			if (exists.code !== 0) {
				continue;
			}
			const base = await exec("git", ["merge-base", "HEAD", candidate], { cwd });
			if (base.code !== 0 || base.stdout.trim() === "") {
				continue;
			}
			return base.stdout.trim();
		}
		return null;
	} catch {
		return null;
	}
}

async function branchCandidates(exec: Exec, cwd: string): Promise<string[]> {
	const symref = await exec(
		"git",
		["symbolic-ref", "--quiet", "refs/remotes/origin/HEAD"],
		{ cwd },
	);
	if (symref.code !== 0) {
		return DEFAULT_BRANCH_CANDIDATES;
	}
	const resolved = symref.stdout.trim().replace(/^refs\/remotes\//, "");
	if (resolved === "") {
		return DEFAULT_BRANCH_CANDIDATES;
	}
	return [resolved, ...DEFAULT_BRANCH_CANDIDATES];
}

/**
 * Diff the branch point against both HEAD and the worktree, merged per path.
 *
 * @param exec command runner
 * @param cwd repository worktree root
 * @param branchPointTree tree to diff from, from resolveBranchPointTree
 * @returns one row per path with origin committed, uncommitted, or both, and
 *   counts summed across the committed and uncommitted sides
 * @throws Error when git exits nonzero
 */
export async function branchStats(
	exec: Exec,
	cwd: string,
	branchPointTree: string,
	maxFiles: number,
): Promise<DiffStats> {
	const headTreeSha = await resolveBaselineTree(exec, cwd);
	const worktreeSha = await writeWorktreeTree(exec, cwd);
	const committed = await treeDiff(exec, cwd, branchPointTree, headTreeSha);
	const uncommitted = await treeDiff(exec, cwd, headTreeSha, worktreeSha);
	return mergeSides(committed, uncommitted, maxFiles);
}

async function treeDiff(
	exec: Exec,
	cwd: string,
	baseTreeSha: string,
	targetTreeSha: string,
): Promise<FileChange[]> {
	if (baseTreeSha === targetTreeSha) {
		return [];
	}
	const numstatOut = await run(exec, cwd, [
		"diff-tree", "-r", "-M", "--numstat", "-z", baseTreeSha, targetTreeSha,
	]);
	const nameStatusOut = await run(exec, cwd, [
		"diff-tree", "-r", "-M", "--name-status", "-z", baseTreeSha, targetTreeSha,
	]);
	const statusByPath = parseNameStatus(nameStatusOut);
	const changes: FileChange[] = [];
	for (const row of parseNumstat(numstatOut)) {
		const status = statusByPath.get(row.path);
		changes.push({
			path: row.path,
			renamedFrom: status?.renamedFrom ?? row.renamedFrom,
			kind: row.isBinary ? "binary" : (status?.kind ?? "modified"),
			additions: row.additions,
			deletions: row.deletions,
			isBinary: row.isBinary,
			origin: "uncommitted",
		});
	}
	return changes;
}

function mergeSides(
	committed: FileChange[],
	uncommitted: FileChange[],
	maxFiles: number,
): DiffStats {
	const byPath = new Map<string, FileChange>();
	for (const change of committed) {
		byPath.set(change.path, { ...change, origin: "committed" });
	}
	for (const change of uncommitted) {
		const existing = byPath.get(change.path);
		if (existing === undefined) {
			byPath.set(change.path, { ...change, origin: "uncommitted" });
			continue;
		}
		byPath.set(change.path, {
			path: change.path,
			renamedFrom: existing.renamedFrom ?? change.renamedFrom,
			kind: existing.isBinary || change.isBinary ? "binary" : change.kind,
			additions: existing.additions + change.additions,
			deletions: existing.deletions + change.deletions,
			isBinary: existing.isBinary || change.isBinary,
			origin: "both",
		});
	}
	const files = [...byPath.values()];
	let additions = 0;
	let deletions = 0;
	for (const change of files) {
		if (!change.isBinary) {
			additions += change.additions;
			deletions += change.deletions;
		}
	}
	const isTruncated = files.length > maxFiles;
	return {
		files: isTruncated ? files.slice(0, maxFiles) : files,
		additions,
		deletions,
		isTruncated,
	};
}

interface NumstatRow {
	path: string;
	renamedFrom: string | null;
	additions: number;
	deletions: number;
	isBinary: boolean;
}

function parseNumstat(output: string): NumstatRow[] {
	const fields = output.split("\0");
	const rows: NumstatRow[] = [];
	let i = 0;
	while (i < fields.length) {
		const field = fields[i];
		if (field === "") {
			i += 1;
			continue;
		}
		const match = /^(-|\d+)\t(-|\d+)\t(.*)$/s.exec(field);
		if (!match) {
			i += 1;
			continue;
		}
		const isBinary = match[1] === "-" && match[2] === "-";
		const additions = isBinary ? 0 : Number(match[1]);
		const deletions = isBinary ? 0 : Number(match[2]);
		if (match[3] === "") {
			rows.push({
				path: fields[i + 2] ?? "",
				renamedFrom: fields[i + 1] ?? null,
				additions,
				deletions,
				isBinary,
			});
			i += 3;
		} else {
			rows.push({
				path: match[3],
				renamedFrom: null,
				additions,
				deletions,
				isBinary,
			});
			i += 1;
		}
	}
	return rows;
}

interface NameStatusEntry {
	kind: FileChangeKind;
	renamedFrom: string | null;
}

function parseNameStatus(output: string): Map<string, NameStatusEntry> {
	const fields = output.split("\0");
	const byPath = new Map<string, NameStatusEntry>();
	let i = 0;
	while (i < fields.length) {
		const status = fields[i];
		if (status === "") {
			i += 1;
			continue;
		}
		if (status.startsWith("R") || status.startsWith("C")) {
			const from = fields[i + 1] ?? "";
			const to = fields[i + 2] ?? "";
			byPath.set(to, { kind: "renamed", renamedFrom: from });
			i += 3;
			continue;
		}
		const path = fields[i + 1] ?? "";
		const kind: FileChangeKind =
			status === "A" ? "added" : status === "D" ? "deleted" : "modified";
		byPath.set(path, { kind, renamedFrom: null });
		i += 2;
	}
	return byPath;
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
export async function filePatch(
	exec: Exec,
	cwd: string,
	baseTreeSha: string,
	path: string,
): Promise<Hunk[]> {
	const currentTreeSha = await writeWorktreeTree(exec, cwd);
	const output = await run(exec, cwd, [
		"diff-tree", "-r", "-M", "-p", baseTreeSha, currentTreeSha, "--", path,
	]);
	return parsePatch(output);
}

function parsePatch(output: string): Hunk[] {
	const hunks: Hunk[] = [];
	let current: Hunk | null = null;
	for (const line of output.split("\n")) {
		if (line.startsWith("Binary files ") || line === "GIT binary patch") {
			return [];
		}
		if (line.startsWith("@@")) {
			current = { header: line, lines: [] };
			hunks.push(current);
			continue;
		}
		if (!current || line.startsWith("\\")) {
			continue;
		}
		const origin = line[0];
		if (origin === " " || origin === "+" || origin === "-") {
			current.lines.push({ origin, text: line.slice(1) });
		}
	}
	return hunks;
}
