import assert from "node:assert/strict";
import { mkdtemp, readFile, writeFile, lstat, symlink, link, rm, access } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import type { TestContext } from "node:test";
import { createOperations } from "./direct.ts";

const packageRoot = process.env.PI_PACKAGE_ROOT ?? "/opt/homebrew/lib/node_modules/@earendil-works/pi-coding-agent";
const { createEditToolDefinition } = await import(`${packageRoot}/dist/core/tools/edit.js`);
const { createWriteToolDefinition } = await import(`${packageRoot}/dist/core/tools/write.js`);

async function fixture(t: TestContext, original = "alpha\nbeta\n") {
	const root = await mkdtemp(join(tmpdir(), "direct-guard-"));
	t.after(() => rm(root, { recursive: true, force: true }));
	const path = join(root, "sample.ts");
	await writeFile(path, original);
	return { root, path };
}

for (const operation of ["edit", "write"] as const) {
	test(`${operation}: rejection preserves bytes and modification metadata`, async (t) => {
		const { root, path } = await fixture(t);
		const before = await lstat(path);
		const operations = createOperations({ operation, deadline: performance.now() + 20_000, validate: async () => { throw new Error("rule rejected"); } });
		const tool = operation === "edit" ? createEditToolDefinition(root, { operations }) : createWriteToolDefinition(root, { operations });
		await assert.rejects(tool.execute("id", { path, content: "new", edits: [{ oldText: "alpha", newText: "new" }] }), /rule rejected/);
		assert.equal(await readFile(path, "utf8"), "alpha\nbeta\n");
		const after = await lstat(path);
		for (const field of ["ino", "mode", "nlink", "size", "mtimeMs", "ctimeMs"] as const) assert.equal(after[field], before[field]);
	});
}

test("native edit reconstructs BOM, CRLF, unicode, and disjoint original matches", async (t) => {
	const { root, path } = await fixture(t, "\ufeffalpha\r\nbeta λ\r\n");
	let candidate = "";
	const operations = createOperations({ operation: "edit", deadline: performance.now() + 20_000, validate: async (proposal) => { candidate = proposal.proposed_text; } });
	const result = await createEditToolDefinition(root, { operations }).execute("id", { path, edits: [{ oldText: "alpha", newText: "beta" }, { oldText: "beta λ", newText: "gamma λ" }] });
	assert.equal(candidate, "\ufeffbeta\r\ngamma λ\r\n");
	assert.equal(await readFile(path, "utf8"), candidate);
	assert.equal(typeof result.details.diff, "string");
});

for (const edits of [[{ oldText: "missing", newText: "x" }], [{ oldText: "a", newText: "x" }], [{ oldText: "alpha", newText: "x" }, { oldText: "lph", newText: "y" }]]) {
	test("native unmatched, ambiguous, or overlapping edit never validates", async (t) => {
		const { root, path } = await fixture(t);
		let calls = 0;
		const operations = createOperations({ operation: "edit", deadline: performance.now() + 20_000, validate: async () => { calls++; } });
		await assert.rejects(createEditToolDefinition(root, { operations }).execute("id", { path, edits }));
		assert.equal(calls, 0);
		assert.equal(await readFile(path, "utf8"), "alpha\nbeta\n");
	});
}

test("missing parents, symbolic links and hard links are rejected", async (t) => {
	const { root, path } = await fixture(t);
	const symbolic = join(root, "symbolic.ts");
	const hard = join(root, "hard.ts");
	await symlink(path, symbolic);
	await link(path, hard);
	for (const target of [symbolic, hard, join(root, "absent", "new.ts")]) {
		const operations = createOperations({ operation: "write", deadline: performance.now() + 20_000, validate: async () => assert.fail("must not validate") });
		await assert.rejects(createWriteToolDefinition(root, { operations }).execute("id", { path: target, content: "x" }));
	}
	await assert.rejects(access(join(root, "absent")));
	assert.equal(await readFile(path, "utf8"), "alpha\nbeta\n");
});

test("cancellation in validation prevents commit", async (t) => {
	const { root, path } = await fixture(t);
	const controller = new AbortController();
	const operations = createOperations({ operation: "write", signal: controller.signal, deadline: performance.now() + 20_000, validate: async () => { controller.abort(); } });
	await assert.rejects(createWriteToolDefinition(root, { operations }).execute("id", { path, content: "x" }), /aborted/);
	assert.equal(await readFile(path, "utf8"), "alpha\nbeta\n");
});

test("conflicting external write is detected before final write", async (t) => {
	const { root, path } = await fixture(t);
	const operations = createOperations({ operation: "write", deadline: performance.now() + 20_000, validate: async () => { await writeFile(path, "external"); } });
	await assert.rejects(createWriteToolDefinition(root, { operations }).execute("id", { path, content: "x" }), /changed/);
	assert.equal(await readFile(path, "utf8"), "external");
});

test("invalid UTF-8 original and unpaired surrogate candidate block", async (t) => {
	const { root, path } = await fixture(t);
	await writeFile(path, Buffer.from([0xff]));
	const operations = createOperations({ operation: "write", deadline: performance.now() + 20_000, validate: async () => assert.fail("invalid encoding") });
	await assert.rejects(createWriteToolDefinition(root, { operations }).execute("id", { path, content: "x" }));
	await writeFile(path, "valid");
	await assert.rejects(createWriteToolDefinition(root, { operations }).execute("id", { path, content: "\ud800" }));
});
