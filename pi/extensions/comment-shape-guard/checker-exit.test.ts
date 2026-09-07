import assert from "node:assert/strict";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "node:test";
import { createOperations } from "./direct.ts";
import { runProcess } from "./process.ts";
import type { Request } from "./protocol.ts";
import { validateCheckerResult, validateProposal } from "./validate.ts";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
const command = join(root, "tools/edit-time-check/target/release/edit-time-check");

test("real checker block reaches safe rule diagnostic before mutation", async (t) => {
	const directory = await mkdtemp(join(tmpdir(), "checker-exit-"));
	t.after(() => rm(directory, { recursive: true, force: true }));
	const path = join(directory, "candidate.ts");
	const original = "const isReady = true;\n";
	await writeFile(path, original);
	const start = performance.now();
	const deadline = start + 3000;
	const operations = createOperations({ operation: "write", deadline, validate: async (proposal) => { await validateProposal(proposal, start, deadline); } });
	await assert.rejects(operations.writeFile(path, "const ready: boolean = true;\n"), (error: Error) => {
		assert.match(error.message, /candidate\.ts":1: boolean-name: Rename/);
		assert.doesNotMatch(error.message, /const ready|required validation failed/);
		return true;
	});
	assert.equal(await readFile(path, "utf8"), original);
	t.diagnostic(`real block end-to-end=${(performance.now() - start).toFixed(3)}ms; deadline=3000ms; checker processes=1; retries=0`);
});

test("expired original request deadlines cannot be reset", async () => {
	const proposal = { operation: "write" as const, path: "/fixture.ts", repository_root: null, original_text: null, proposed_text: "const isReady = true;" };
	const now = performance.now();
	await assert.rejects(validateProposal(proposal, now - 20_001, now + 3000), /deadline exceeded/);
	await assert.rejects(validateProposal(proposal, now, now - 1), /deadline exceeded/);
	const controller = new AbortController();
	controller.abort();
	await assert.rejects(validateProposal(proposal, performance.now(), performance.now() + 3000, controller.signal), /aborted before commit/);
});

test("real checker error exit two rejects without mutation", async (t) => {
	const directory = await mkdtemp(join(tmpdir(), "checker-error-"));
	t.after(() => rm(directory, { recursive: true, force: true }));
	const path = join(directory, "candidate.ts");
	const original = "const isReady = true;\n";
	await writeFile(path, original);
	const start = performance.now();
	const deadline = start + 3000;
	const operations = createOperations({ operation: "write", deadline, validate: async (proposal) => {
		const request: Request = { version: 1, request_id: "error-fixture", ...proposal, changed_ranges: [], budget_ms: 3000 };
		const result = await runProcess({ command, args: ["--config", join(directory, "missing.toml")], input: JSON.stringify(request), deadline, resultMode: "exit-status" });
		assert.equal(result.exitCode, 2);
		assert.equal(JSON.parse(result.stdout).decision, "error");
		validateCheckerResult(result, request, start);
	} });
	await assert.rejects(operations.writeFile(path, "const isDone = true;\n"), (error: Error) => {
		assert.match(error.message, /candidate\.ts":1: checker: Repair/);
		assert.doesNotMatch(error.message, /privacy|missing.toml|const isDone/);
		return true;
	});
	assert.equal(await readFile(path, "utf8"), original);
	t.diagnostic(`real error end-to-end=${(performance.now() - start).toFixed(3)}ms; deadline=3000ms; checker processes=1; retries=0`);
});
