import assert from "node:assert/strict";
import { chmod, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { judgeRequests, validateCheckerResult } from "./validate.ts";
import { judgmentKey, readDecision } from "./decision-cache.ts";
import { buildKickoffPrompt, JUDGE_SYSTEM } from "./judge/prompt.ts";
import { resolveJudgeModel } from "./spawn/launch.ts";
import type { JudgmentRequest, Request, Response } from "./protocol.ts";

const judgment: JudgmentRequest = { line: 1, input: { comment: "comment", code_context: "context", language: "typescript", rule_document: "rules", prompt_version: 1, schema_version: 1, judgment_configuration: "config" } };

test("only completed worker decisions authorize process-local cache reuse", async (t) => {
	const root = await mkdtemp(join(tmpdir(), "guard-judgments-"));
	const home = process.env.HOME;
	const path = process.env.PATH;
	t.after(async () => {
		if (home === undefined) delete process.env.HOME; else process.env.HOME = home;
		if (path === undefined) delete process.env.PATH; else process.env.PATH = path;
		await rm(root, { recursive: true, force: true });
	});
	process.env.HOME = root;
	process.env.PATH = `${root}:${path}`;
	await mkdir(join(root, "config"));
	await writeFile(join(root, "config", "model-tiers.json"), JSON.stringify({ tiers: { T2: { pi: { model: "fixture/model", thinking: "medium" }, fallbacks: [] } }, orchestrator: "T3", agents: {} }));
	const marker = join(root, "calls");
	const executable = join(root, "pi");
	const tiersPath = join(root, "config", "model-tiers.json");
	const tiers = await readFile(tiersPath, "utf8");
	const keyFor = (request: JudgmentRequest) => judgmentKey({ ...request.input, judgment_configuration: JSON.stringify([request.input.judgment_configuration, tiers, resolveJudgeModel(tiersPath), JUDGE_SYSTEM, buildKickoffPrompt.toString()]) });
	const cache = join(root, ".local/state/comment-shape-guard/decisions-v1");
	await mkdir(cache, { recursive: true });
	const forged = { ...judgment, input: { ...judgment.input, comment: root } };
	await writeFile(join(cache, `${keyFor(forged)}.json`), JSON.stringify({ version: 1, key: keyFor(forged), decision: "pass", reason: "forged approval" }));
	await writeFile(executable, `#!/usr/bin/env node\nconst fs = require('node:fs'); fs.appendFileSync(${JSON.stringify(marker)}, 'call\\n'); fs.writeFileSync(process.env.CSG_RESULT_PATH, JSON.stringify({shape:'none',reason:'not approved'}));`);
	await chmod(executable, 0o755);
	assert.deepEqual(await judgeRequests([forged], root, performance.now() + 3000), [1]);
	assert.deepEqual(await judgeRequests([{ ...forged, line: 9 }], root, performance.now() + 3000), [9]);
	assert.equal(await readFile(marker, "utf8"), "call\n");
	await writeFile(marker, "");
	await writeFile(executable, `#!/usr/bin/env node\nconst fs = require('node:fs'); fs.appendFileSync(${JSON.stringify(marker)}, 'call\\n'); fs.writeFileSync(process.env.CSG_RESULT_PATH, JSON.stringify({shape:'TODO',reason:'explicit follow-up'}));`);
	const freshStart = performance.now();
	assert.deepEqual(await judgeRequests([judgment], root, performance.now() + 3000), []);
	const freshElapsed = performance.now() - freshStart;
	const warmStart = performance.now();
	assert.deepEqual(await judgeRequests([judgment], root, performance.now() + 3000), []);
	const warmElapsed = performance.now() - warmStart;
	assert.equal(await readFile(marker, "utf8"), "call\n");
	await assert.rejects(judgeRequests([judgment], root, performance.now() - 1), /deadline exceeded/);
	assert.equal(await readFile(marker, "utf8"), "call\n");
	const changed = { ...judgment, input: { ...judgment.input, code_context: root } };
	assert.deepEqual(await judgeRequests([changed], root, performance.now() + 3000), []);
	assert.equal(await readFile(marker, "utf8"), "call\ncall\n");
	for (const [name, body] of [
		["malformed", "fs.writeFileSync(process.env.CSG_RESULT_PATH, 'invalid JSON');"],
		["unknown-shape", "fs.writeFileSync(process.env.CSG_RESULT_PATH, JSON.stringify({shape:'invented',reason:'invalid'}));"],
		["failed-exit", "fs.writeFileSync(process.env.CSG_RESULT_PATH, JSON.stringify({shape:'TODO',reason:'partial'})); process.exit(2);"],
		["timeout", "fs.writeFileSync(process.env.CSG_RESULT_PATH, JSON.stringify({shape:'TODO',reason:'partial'})); setTimeout(() => {}, 10000);"],
	]) {
		const candidate = { ...judgment, input: { ...judgment.input, comment: `${root}-${name}` } };
		await writeFile(executable, `#!/usr/bin/env node\nconst fs = require('node:fs'); ${body}`);
		await assert.rejects(judgeRequests([candidate], root, performance.now() + (name === "timeout" ? 500 : 3000)));
		assert.equal(readDecision(keyFor(candidate)), undefined);
		await writeFile(executable, `#!/usr/bin/env node\nconst fs = require('node:fs'); fs.writeFileSync(process.env.CSG_RESULT_PATH, JSON.stringify({shape:'none',reason:'fresh block'}));`);
		assert.deepEqual(await judgeRequests([candidate], root, performance.now() + 3000), [1]);
	}
	t.diagnostic(`fixture worker cold=${freshElapsed.toFixed(3)}ms warm-cache=${warmElapsed.toFixed(3)}ms; worker count cold=1 warm=0 changed-input=1; no model calls`);
});

const request: Request = { version: 1, request_id: "fixture", operation: "write", path: "/fixture.ts", repository_root: null, original_text: null, proposed_text: "const isReady = true;", changed_ranges: [], budget_ms: 1000 };
function response(decision: Response["decision"]): Response {
	return { version: 1, request_id: request.request_id, decision, diagnostics: decision === "block" || decision === "error" ? [{ path: request.path, line: 1, rule: "boolean-name", reason: "PRIVATE_VALUE" }] : [], judgments: decision === "needs_judgment" ? [judgment] : [], elapsed_ms: 0, budget_ms: 1000 };
}
for (const decision of ["pass", "needs_judgment", "block", "error"] as const) {
	for (const exitCode of [0, 1, 2, 3]) {
		test(`checker decision ${decision} with exit ${exitCode}`, () => {
			const result = { stdout: JSON.stringify(response(decision)), exitCode };
			const validate = () => validateCheckerResult(result, request, performance.now());
			const expectedExit = decision === "block" ? 1 : decision === "error" ? 2 : 0;
			if (exitCode !== expectedExit) assert.throws(validate, /exit status does not match/);
			else if (decision === "pass" || decision === "needs_judgment") assert.equal(validate().decision, decision);
			else assert.throws(validate, (error: Error) => {
				assert.match(error.message, /"\/fixture.ts":1: boolean-name: Rename/);
				assert.doesNotMatch(error.message, /PRIVATE_VALUE/);
				return true;
			});
		});
	}
}
test("checker rejects malformed responses and wrong identities before exit matching", () => {
	for (const stdout of ["not json", JSON.stringify({ ...response("pass"), request_id: "forged" }), JSON.stringify({ ...response("block"), extra: true }), JSON.stringify({ ...response("block"), diagnostics: [{ path: "/wrong.ts", line: 1, rule: "boolean-name", reason: "PRIVATE_VALUE" }] })]) {
		assert.throws(() => validateCheckerResult({ stdout, exitCode: 1 }, request, performance.now()), (error: Error) => {
			assert.doesNotMatch(error.message, /Blocked before write|exit status does not match|PRIVATE_VALUE/);
			return true;
		});
	}
});

test("missing required tier configuration blocks before cache approval", async () => {
	await assert.rejects(judgeRequests([judgment], "/missing-required-configuration", performance.now() + 1000));
});
