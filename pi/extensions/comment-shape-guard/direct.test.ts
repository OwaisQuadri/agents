import assert from "node:assert/strict";
import { mkdtemp, mkdir, readFile, realpath, writeFile, lstat, symlink, link, rm, access } from "node:fs/promises";
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

test("one or several missing parents are created before a validated destination write", async (t) => {
	const { root } = await fixture(t);
	for (const path of [join(root, "one", "new.ts"), join(root, "several", "nested", "new.ts")]) {
		let candidate = "";
		const operations = createOperations({ operation: "write", deadline: performance.now() + 20_000, validate: async (proposal) => {
			candidate = proposal.proposed_text;
			await assert.rejects(lstat(path), { code: "ENOENT" });
		} });
		await createWriteToolDefinition(root, { operations }).execute("id", { path, content: "x" });
		assert.equal(candidate, "x");
		assert.equal(await readFile(path, "utf8"), candidate);
	}
});

for (const mode of ["expired", "aborted"] as const) {
	test(`${mode} requests create no missing parent`, async (t) => {
		const { root } = await fixture(t);
		const parent = join(root, mode, "nested");
		const path = join(parent, "new.ts");
		const controller = new AbortController();
		if (mode === "aborted") controller.abort();
		let calls = 0;
		const operations = createOperations({
			operation: "write",
			signal: controller.signal,
			deadline: mode === "expired" ? performance.now() - 1 : performance.now() + 20_000,
			validate: async () => { calls++; },
		});
		const execution = mode === "expired"
			? createWriteToolDefinition(root, { operations }).execute("id", { path, content: "x" })
			: operations.mkdir(parent);
		await assert.rejects(execution, mode === "expired" ? /deadline/ : /aborted before commit/);
		assert.equal(calls, 0);
		await assert.rejects(lstat(parent), { code: "ENOENT" });
	});
}

test("validation rejection leaves created parents but no destination file", async (t) => {
	const { root } = await fixture(t);
	const parent = join(root, "created");
	const path = join(parent, "new.ts");
	const operations = createOperations({ operation: "write", deadline: performance.now() + 20_000, validate: async () => { throw new Error("rule rejected"); } });
	await assert.rejects(createWriteToolDefinition(root, { operations }).execute("id", { path, content: "x" }), /rule rejected/);
	await access(parent);
	await assert.rejects(lstat(path), { code: "ENOENT" });
});

test("a linked parent follows the installed Pi write behavior", async (t) => {
	const { root } = await fixture(t);
	const target = join(root, "target-parent");
	const linked = join(root, "linked-parent");
	await mkdir(target);
	await symlink(target, linked);
	const path = join(linked, "nested", "new.ts");
	let canonicalPath = "";
	const operations = createOperations({ operation: "write", deadline: performance.now() + 20_000, validate: async (proposal) => { canonicalPath = proposal.path; } });
	await createWriteToolDefinition(root, { operations }).execute("id", { path, content: "x" });
	assert.equal(canonicalPath, join(await realpath(target), "nested", "new.ts"));
	assert.equal(await readFile(join(target, "nested", "new.ts"), "utf8"), "x");
});

test("existing parents succeed while links and invalid parent components reject", async (t) => {
	const { root, path } = await fixture(t);
	const existing = join(root, "new.ts");
	const symbolic = join(root, "symbolic.ts");
	const hard = join(root, "hard.ts");
	await symlink(path, symbolic);
	await link(path, hard);
	let candidate = "";
	const operations = createOperations({ operation: "write", deadline: performance.now() + 20_000, validate: async (proposal) => { candidate = proposal.proposed_text; } });
	await createWriteToolDefinition(root, { operations }).execute("id", { path: existing, content: "x" });
	assert.equal(await readFile(existing, "utf8"), candidate);
	for (const target of [symbolic, hard, join(path, "new.ts")]) {
		let calls = 0;
		const rejecting = createOperations({ operation: "write", deadline: performance.now() + 20_000, validate: async () => { calls++; } });
		await assert.rejects(createWriteToolDefinition(root, { operations: rejecting }).execute("id", { path: target, content: "x" }));
		assert.equal(calls, 0);
	}
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
