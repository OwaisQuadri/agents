import assert from "node:assert/strict";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

import { buildWorkerArgv, buildWorkerEnv, resolveJudgeModel, spawnWorker } from "./launch.ts";

test("buildWorkerArgv builds a minimal, fully scoped headless invocation", () => {
	const argv = buildWorkerArgv({
		model: { model: "anthropic/claude-haiku-4-5", thinking: "medium" },
		sessionName: "comment-shape-guard-judge-x",
		kickoffPrompt: "judge this comment",
	});
	assert.ok(argv.includes("--no-extensions"));
	assert.ok(argv.includes("--no-builtin-tools"));
	assert.ok(argv.includes("--no-skills"));
	assert.equal(argv[argv.indexOf("--model") + 1], "anthropic/claude-haiku-4-5");
	assert.equal(argv[argv.indexOf("--thinking") + 1], "medium");
	assert.equal(argv[argv.indexOf("-n") + 1], "comment-shape-guard-judge-x");
	assert.equal(argv[argv.indexOf("-p") + 1], "judge this comment");
	assert.ok(argv[argv.indexOf("-e") + 1].endsWith("comment-shape-guard/judge/agent/index.ts"));
});

test("buildWorkerEnv sets the result-path IPC variable and preserves the parent env", () => {
	const env = buildWorkerEnv("/tmp/comment-shape-guard/runs/r1.result.json");
	assert.equal(env.CSG_RESULT_PATH, "/tmp/comment-shape-guard/runs/r1.result.json");
	assert.equal(env.PATH, process.env.PATH);
});

test("resolveJudgeModel reads T2's primary model from a tiers file", () => {
	const dir = mkdtempSync(join(tmpdir(), "comment-shape-guard-tiers-"));
	const path = join(dir, "model-tiers.json");
	writeFileSync(
		path,
		JSON.stringify({
			tiers: { T2: { pi: { model: "anthropic/claude-haiku-4-5", thinking: "medium" }, fallbacks: [] } },
			orchestrator: "T3",
			agents: {},
		}),
	);
	const entry = resolveJudgeModel(path);
	assert.equal(entry.model, "anthropic/claude-haiku-4-5");
	assert.equal(entry.thinking, "medium");
	rmSync(dir, { recursive: true, force: true });
});

test("resolveJudgeModel falls back to a fixed model on a missing or malformed tiers file", () => {
	const entry = resolveJudgeModel("/nonexistent/model-tiers.json");
	assert.equal(entry.model, "anthropic/claude-haiku-4-5");
});

test("spawnWorker resolves with exit code 0 on success", async () => {
	const exit = await spawnWorker({ argv: ["/usr/bin/true"], cwd: tmpdir(), env: process.env });
	assert.equal(exit.code, 0);
});

test("spawnWorker resolves (never rejects) with a non-zero code on failure", async () => {
	const exit = await spawnWorker({ argv: ["/usr/bin/false"], cwd: tmpdir(), env: process.env });
	assert.notEqual(exit.code, 0);
});

test("spawnWorker resolves with an error code when the command does not exist", async () => {
	const exit = await spawnWorker({ argv: ["/nonexistent/binary-xyz"], cwd: tmpdir(), env: process.env });
	assert.notEqual(exit.code, 0);
});

test("spawnWorker kills the process and resolves when its AbortSignal fires", async () => {
	const controller = new AbortController();
	const promise = spawnWorker({ argv: ["/bin/sleep", "30"], cwd: tmpdir(), env: process.env, signal: controller.signal });
	controller.abort();
	const exit = await promise;
	assert.notEqual(exit.code, 0);
});
