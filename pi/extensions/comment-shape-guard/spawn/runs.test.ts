import assert from "node:assert/strict";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, test } from "node:test";

import { readWorkerVerdict, runResultPath, runsDir, writeWorkerVerdict } from "./runs.ts";

let dir: string;

afterEach(() => {
	if (dir) rmSync(dir, { recursive: true, force: true });
});

test("runsDir and runResultPath nest under the given home's state dir", () => {
	dir = mkdtempSync(join(tmpdir(), "comment-shape-guard-runs-"));
	assert.equal(runsDir(dir), join(dir, ".local", "state", "comment-shape-guard", "runs"));
	assert.equal(runResultPath("run-1", dir), join(dir, ".local", "state", "comment-shape-guard", "runs", "run-1.result.json"));
});

test("a written verdict round-trips exactly", () => {
	dir = mkdtempSync(join(tmpdir(), "comment-shape-guard-runs-"));
	const path = runResultPath("run-1", dir);
	writeWorkerVerdict(path, { shape: "TODO", reason: "explicit deferred follow-up" });
	assert.deepEqual(readWorkerVerdict(path), { shape: "TODO", reason: "explicit deferred follow-up" });
});

test("a missing result file reads as undefined, not a thrown error", () => {
	dir = mkdtempSync(join(tmpdir(), "comment-shape-guard-runs-"));
	assert.equal(readWorkerVerdict(runResultPath("never-ran", dir)), undefined);
});

test("a malformed result file reads as undefined", () => {
	dir = mkdtempSync(join(tmpdir(), "comment-shape-guard-runs-"));
	const path = runResultPath("bad", dir);
	writeWorkerVerdict(path, { shape: "TODO", reason: "placeholder, overwritten below" });
	writeFileSync(path, "{not valid json");
	assert.equal(readWorkerVerdict(path), undefined);
});

test("a result file missing a required field reads as undefined", () => {
	dir = mkdtempSync(join(tmpdir(), "comment-shape-guard-runs-"));
	const path = runResultPath("partial", dir);
	writeWorkerVerdict(path, { shape: "TODO", reason: "placeholder, overwritten below" });
	writeFileSync(path, JSON.stringify({ shape: "TODO" }));
	assert.equal(readWorkerVerdict(path), undefined);
});
