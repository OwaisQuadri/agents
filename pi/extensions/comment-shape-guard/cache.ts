/**
 * Verdict cache for comment-shape-guard: a content-hash-keyed, append-only, machine-global
 * jsonl file. A comment's exact trimmed text hashes the same everywhere it appears, so a
 * span already judged once — anywhere, in any project — resolves instantly here with no
 * subprocess spawned. This is what gives the guard its diff-only, incremental-cost
 * behavior: an unmodified comment carried through a whole-file rewrite hashes identically
 * to its last verdict and never re-pays the model call.
 *
 * Machine-global (not per-project) on purpose, matching this repo's other cross-project
 * state (`~/.pi/agent/sessions`): identical boilerplate comments recur across repos, and a
 * shared cache maximizes the hit rate.
 */
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join } from "node:path";

export type Verdict = {
	hash: string;
	shape: string; // a whitelist shape name, or "none"
	reason: string;
	judgedAt: string; // ISO 8601
};

export type UnverifiedEntry = {
	hash: string;
	reason: string;
	at: string; // ISO 8601
};

export function stateDir(home = homedir()): string {
	return join(home, ".local", "state", "comment-shape-guard");
}

export function verdictsPath(home = homedir()): string {
	return join(stateDir(home), "verdicts.jsonl");
}

export function unverifiedPath(home = homedir()): string {
	return join(stateDir(home), "unverified.jsonl");
}

/** sha256 of the trimmed comment text — the cache key. Trimmed so trailing-whitespace-only
 * edits, which do not change what a comment says, still hit the same cache entry. */
export function hashCommentText(text: string): string {
	return createHash("sha256").update(text.trim()).digest("hex");
}

/** Append-only (temp + rename) so a concurrent reader never sees a half-written line. */
function appendLine(path: string, line: string): void {
	mkdirSync(dirname(path), { recursive: true });
	const existing = existsSync(path) ? readFileSync(path, "utf-8") : "";
	const tmp = `${path}.tmp-${process.pid}-${Date.now()}`;
	writeFileSync(tmp, `${existing}${line}\n`, "utf-8");
	renameSync(tmp, path);
}

function parseLines<T>(path: string, isValid: (v: unknown) => v is T): T[] {
	if (!existsSync(path)) return [];
	const raw = readFileSync(path, "utf-8");
	const out: T[] = [];
	for (const line of raw.split("\n")) {
		if (!line.trim()) continue;
		try {
			const parsed = JSON.parse(line) as unknown;
			if (isValid(parsed)) out.push(parsed);
		} catch {
			// malformed line — skip rather than fail the whole read
		}
	}
	return out;
}

function isVerdict(v: unknown): v is Verdict {
	if (!v || typeof v !== "object") return false;
	const r = v as Record<string, unknown>;
	return typeof r.hash === "string" && typeof r.shape === "string" && typeof r.reason === "string" && typeof r.judgedAt === "string";
}

function isUnverifiedEntry(v: unknown): v is UnverifiedEntry {
	if (!v || typeof v !== "object") return false;
	const r = v as Record<string, unknown>;
	return typeof r.hash === "string" && typeof r.reason === "string" && typeof r.at === "string";
}

/** The most recent verdict for `hash`, or undefined on a cache miss. Append-only + last-wins
 * lets a re-judge (e.g. the whitelist itself changed) override an earlier entry without
 * ever deleting history. */
export function readVerdict(hash: string, path = verdictsPath()): Verdict | undefined {
	const entries = parseLines(path, isVerdict).filter((v) => v.hash === hash);
	return entries.at(-1);
}

export function appendVerdict(verdict: Verdict, path = verdictsPath()): void {
	appendLine(path, JSON.stringify(verdict));
}

export function appendUnverified(entry: UnverifiedEntry, path = unverifiedPath()): void {
	appendLine(path, JSON.stringify(entry));
}
