import assert from "node:assert/strict";
import { appendFileSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, test } from "node:test";

import { appendUnverified, appendVerdict, hashCommentText, readVerdict } from "./cache.ts";

let dir: string;

function freshPaths() {
	dir = mkdtempSync(join(tmpdir(), "comment-shape-guard-cache-"));
	return { verdicts: join(dir, "verdicts.jsonl"), unverified: join(dir, "unverified.jsonl") };
}

afterEach(() => {
	if (dir) rmSync(dir, { recursive: true, force: true });
});

test("hashCommentText is stable and ignores surrounding whitespace", () => {
	const a = hashCommentText("// a note\n");
	const b = hashCommentText("  // a note  ");
	assert.equal(a, b);
	assert.notEqual(a, hashCommentText("// a different note"));
});

test("a miss returns undefined", () => {
	const { verdicts } = freshPaths();
	assert.equal(readVerdict(hashCommentText("// x"), verdicts), undefined);
});

test("a hit round-trips the exact verdict written", () => {
	const { verdicts } = freshPaths();
	const hash = hashCommentText("// invariant: x never exceeds y");
	appendVerdict({ hash, shape: "inexpressible concept or architecture", reason: "states a cross-component invariant", judgedAt: "2026-09-03T00:00:00Z" }, verdicts);
	const found = readVerdict(hash, verdicts);
	assert.equal(found?.shape, "inexpressible concept or architecture");
	assert.equal(found?.reason, "states a cross-component invariant");
});

test("a later verdict for the same hash wins (last-wins, append-only)", () => {
	const { verdicts } = freshPaths();
	const hash = hashCommentText("// same text");
	appendVerdict({ hash, shape: "none", reason: "first pass", judgedAt: "2026-09-03T00:00:00Z" }, verdicts);
	appendVerdict({ hash, shape: "TODO", reason: "re-judged after whitelist update", judgedAt: "2026-09-04T00:00:00Z" }, verdicts);
	assert.equal(readVerdict(hash, verdicts)?.shape, "TODO");
});

test("different comments hash to different cache entries and never collide", () => {
	const { verdicts } = freshPaths();
	const hashA = hashCommentText("// comment A");
	const hashB = hashCommentText("// comment B");
	appendVerdict({ hash: hashA, shape: "none", reason: "a", judgedAt: "2026-09-03T00:00:00Z" }, verdicts);
	appendVerdict({ hash: hashB, shape: "TODO", reason: "b", judgedAt: "2026-09-03T00:00:00Z" }, verdicts);
	assert.equal(readVerdict(hashA, verdicts)?.shape, "none");
	assert.equal(readVerdict(hashB, verdicts)?.shape, "TODO");
});

test("a malformed line in the cache file is skipped, not fatal", () => {
	const { verdicts } = freshPaths();
	const hash = hashCommentText("// ok");
	appendVerdict({ hash, shape: "TODO", reason: "fine", judgedAt: "2026-09-03T00:00:00Z" }, verdicts);
	// simulate a torn write landing between two good lines
	appendFileSync(verdicts, "{not valid json\n");
	assert.equal(readVerdict(hash, verdicts)?.shape, "TODO");
});

test("appendUnverified writes a readable jsonl entry distinct from the verdict cache", () => {
	const { unverified } = freshPaths();
	appendUnverified({ hash: hashCommentText("// x"), reason: "worker spawn timed out", at: "2026-09-03T00:00:00Z" }, unverified);
	const lines = readFileSync(unverified, "utf-8").trim().split("\n");
	assert.equal(lines.length, 1);
	const parsed = JSON.parse(lines[0]);
	assert.equal(parsed.reason, "worker spawn timed out");
});
