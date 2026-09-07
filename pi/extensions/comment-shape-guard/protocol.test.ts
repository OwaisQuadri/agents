import assert from "node:assert/strict";
import { test } from "node:test";
import { blockingMessage, validateResponse, type Request } from "./protocol.ts";

const request: Request = { version: 1, request_id: "fixture", operation: "write", path: "/fixture.ts", repository_root: null, original_text: null, proposed_text: "λ\n", changed_ranges: [{ start_byte: 0, end_byte: 3 }], budget_ms: 3000 };
const pass = { version: 1, request_id: "fixture", decision: "pass", diagnostics: [], judgments: [], elapsed_ms: 1, budget_ms: 3000 };
const input = { comment: "example", code_context: "export function example() {}", language: "typescript", rule_document: "rules", prompt_version: 1, schema_version: 1, judgment_configuration: "tiers" };

test("accepts a single matching completed pass", () => assert.deepEqual(validateResponse(JSON.stringify(pass), request), pass));
test("accepts snake_case judgment inputs with explicit source line", () => {
	const response = { ...pass, decision: "needs_judgment", judgments: [{ line: 2, input }] };
	assert.deepEqual(validateResponse(JSON.stringify(response), request), response);
});
for (const value of ["{}", JSON.stringify(pass) + JSON.stringify(pass), JSON.stringify({ ...pass, request_id: "wrong" }), JSON.stringify({ ...pass, extra: true }), JSON.stringify({ ...pass, elapsed_ms: -1 }), JSON.stringify({ ...pass, decision: "needs_judgment" }), JSON.stringify({ ...pass, judgments: [{ line: 2, input }] }), JSON.stringify({ ...pass, decision: "block" }), JSON.stringify({ ...pass, decision: "needs_judgment", judgments: [{ startLine: 1, input }] }), JSON.stringify({ ...pass, decision: "needs_judgment", judgments: [{ line: 0, input }] })]) {
	test(`rejects invalid terminal response ${value.slice(0, 60)}`, () => assert.throws(() => validateResponse(value, request)));
}

for (const budget_ms of [undefined, null, 0, -1, 1.5, "3000", Number.MAX_SAFE_INTEGER + 1, 3001, 20_001]) {
	test(`rejects missing or invalid effective budget ${budget_ms}`, () => assert.throws(() => validateResponse(JSON.stringify({ ...pass, budget_ms }), request)));
}
test("accepts a reduced total and rejects totals above the absolute ceiling", () => {
	assert.equal(validateResponse(JSON.stringify({ ...pass, budget_ms: 1 }), request).budget_ms, 1);
	assert.throws(() => validateResponse(JSON.stringify({ ...pass, budget_ms: 20_001 }), { ...request, budget_ms: 30_000 }));
});

test("combined diagnostics remain bounded with complete omitted accounting", () => {
	const diagnostics = Array.from({ length: 1000 }, (_, index) => ({ line: index + 1, rule: "privacy" as const, reason: "sensitive-fixture-value", path: "/fixture.ts" }));
	const message = blockingMessage(diagnostics, 3);
	assert.ok(message.length <= 2000);
	assert.ok(!message.includes("sensitive-fixture-value"));
	const displayed = message.match(/: privacy:/g)?.length ?? 0;
	assert.ok(message.endsWith(`${1000 - displayed} diagnostics omitted.`));
});

for (const decision of ["error", "block", "pass", "needs_judgment"]) {
	test(`checker category is diagnostic-only: ${decision}`, () => {
		const response = { ...pass, decision, diagnostics: [{ path: request.path, line: 1, rule: "checker", reason: "PRIVATE_VALUE" }] };
		if (decision === "error") assert.equal(validateResponse(JSON.stringify(response), request).diagnostics[0].rule, "checker");
		else assert.throws(() => validateResponse(JSON.stringify(response), request));
	});
}

test("controlled guidance renders validated location without checker reason text", () => {
	for (const rule of ["privacy", "comment-length", "comment-shape", "boolean-name", "checker"] as const) {
		const response = validateResponse(JSON.stringify({ ...pass, decision: "error", diagnostics: [{ path: request.path, line: 2, rule, reason: "PRIVATE_VALUE\u001b[31m" }] }), request);
		const message = blockingMessage(response.diagnostics, 1);
		assert.ok(message.includes(`"/fixture.ts":2: ${rule}: `));
		assert.doesNotMatch(message, /PRIVATE_VALUE|\u001b|check failed/);
	}
});

test("long and escaped target paths retain exact omitted counts within the combined cap", () => {
	for (const path of ["/" + "x".repeat(1800), "/" + "x".repeat(2001), "/" + "\n".repeat(1000), "/quoted\"\nfile.ts"]) {
		const diagnostics = Array.from({ length: 1000 }, () => ({ path, line: 1, rule: "privacy" as const }));
		const message = blockingMessage(diagnostics, 1);
		assert.ok(message.length <= 2000);
		const displayed = message.match(/: privacy:/g)?.length ?? 0;
		assert.ok(message.endsWith(`${1000 - displayed} diagnostics omitted.`));
		if (displayed) assert.ok(message.includes(JSON.stringify(path)));
	}
});
