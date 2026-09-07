import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { test } from "node:test";
import { judgmentKey, readDecision, writeDecision } from "./decision-cache.ts";
import type { JudgmentInput } from "./protocol.ts";

const input: JudgmentInput = { comment: "comment", code_context: "code", language: "typescript", rule_document: "rules", prompt_version: 1, schema_version: 1, judgment_configuration: "config" };
for (const field of Object.keys(input) as (keyof JudgmentInput)[]) {
	test(`cache identity includes ${field}`, () => {
		const key = judgmentKey(input);
		writeDecision({ version: 1, key, decision: "pass", reason: "completed decision" });
		const changedKey = judgmentKey({ ...input, [field]: typeof input[field] === "number" ? 2 : `${input[field]} changed` });
		assert.notEqual(key, changedKey);
		assert.equal(readDecision(changedKey), undefined);
	});
}
test("completed pass and block records stay immutable and process-local", () => {
	const key = judgmentKey(input);
	for (const decision of ["pass", "block"] as const) {
		const entry = { version: 1 as const, key, decision, reason: "completed decision" };
		writeDecision(entry);
		assert.deepEqual(readDecision(key), entry);
		entry.reason = "mutated";
		assert.equal(readDecision(key)?.reason, "completed decision");
		assert.equal(Object.isFrozen(readDecision(key)), true);
		assert.throws(() => Object.assign(readDecision(key)!, { decision: "pass" }), TypeError);
	}
	const child = spawnSync(process.execPath, ["--input-type=module", "-e", `import assert from 'node:assert/strict'; import { readDecision } from ${JSON.stringify(new URL("./decision-cache.ts", import.meta.url).href)}; assert.equal(readDecision(${JSON.stringify(key)}), undefined);`], { encoding: "utf8", timeout: 5000 });
	assert.equal(child.status, 0, child.stderr);
});
test("cache evicts the oldest record beyond 256 entries", () => {
	for (let index = 0; index < 257; index++) writeDecision({ version: 1, key: `bounded-${index}`, decision: "block", reason: "completed decision" });
	assert.equal(readDecision("bounded-0"), undefined);
	assert.equal(readDecision("bounded-1")?.decision, "block");
	assert.equal(readDecision("bounded-256")?.decision, "block");
});
