import { mkdtemp, readdir, rm, stat } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import type { DiffStats, FileChange, FileChangeKind, Hunk, Snapshot } from "./types.ts";

export const TEMP_INDEX_PREFIX = "live-diff-index-";

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
