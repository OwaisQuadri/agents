import assert from "node:assert/strict";
import { mkdtemp, readFile, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { createOperations } from "./direct.ts";

const packageRoot = process.env.PI_PACKAGE_ROOT ?? "/opt/homebrew/lib/node_modules/@earendil-works/pi-coding-agent";
const { createEditToolDefinition } = await import(`${packageRoot}/dist/core/tools/edit.js`);
const { createWriteToolDefinition } = await import(`${packageRoot}/dist/core/tools/write.js`);

test("completed write remains successful when cancellation follows commit", async (t) => {
	const root = await mkdtemp(join(tmpdir(), "guard-commit-"));
	t.after(() => rm(root, { recursive: true, force: true }));
	const path = join(root, "new.ts");
	const controller = new AbortController();
	const base = createOperations({ operation: "write", signal: controller.signal, deadline: performance.now() + 20_000, validate: async () => {} });
	const operations = { ...base, writeFile: async (path: string, content: string) => { await base.writeFile(path, content); controller.abort(); } };
	const result = await createWriteToolDefinition(root, { operations }).execute("fixture", { path, content: "complete" });
	assert.equal(controller.signal.aborted, true);
	assert.equal(await readFile(path, "utf8"), "complete");
	assert.match(result.content[0].text, /Successfully wrote/);
});

test("native per-path queue preserves both concurrent edits", async (t) => {
	const root = await mkdtemp(join(tmpdir(), "guard-queue-"));
	t.after(() => rm(root, { recursive: true, force: true }));
	const path = join(root, "file.ts");
	await writeFile(path, "alpha\nbeta\n");
	const candidates: string[] = [];
	await Promise.all(["alpha", "beta"].map(async (text) => {
		const operations = createOperations({ operation: "edit", deadline: performance.now() + 20_000, validate: async (proposal) => { candidates.push(proposal.proposed_text); } });
		return createEditToolDefinition(root, { operations }).execute(text, { path, edits: [{ oldText: text, newText: `${text}!` }] });
	}));
	assert.equal(candidates.length, 2);
	assert.equal(await readFile(path, "utf8"), "alpha!\nbeta!\n");
	assert.equal(candidates[1], "alpha!\nbeta!\n");
});

test("native deletion retains complete unicode candidate for validation", async (t) => {
	const root = await mkdtemp(join(tmpdir(), "guard-deletion-"));
	t.after(() => rm(root, { recursive: true, force: true }));
	const path = join(root, "file.ts");
	await writeFile(path, "λ\nremove\n// comment\nexport function example() {}\n");
	let candidate = "";
	const operations = createOperations({ operation: "edit", deadline: performance.now() + 20_000, validate: async (proposal) => { candidate = proposal.proposed_text; } });
	await createEditToolDefinition(root, { operations }).execute("delete", { path, edits: [{ oldText: "remove\n", newText: "" }] });
	assert.equal(candidate, "λ\n// comment\nexport function example() {}\n");
	assert.equal(await readFile(path, "utf8"), candidate);
});

test("file snapshot and commit measurements across bounded input sizes", async (t) => {
	const root = await mkdtemp(join(tmpdir(), "guard-timing-"));
	t.after(() => rm(root, { recursive: true, force: true }));
	for (const bytes of [1024, 64 * 1024, 1024 * 1024]) {
		const times: number[] = [];
		for (let trial = 0; trial < 5; trial++) {
			const path = join(root, `${bytes}-${trial}.txt`);
			await writeFile(path, "x".repeat(bytes));
			const operations = createOperations({ operation: "write", deadline: performance.now() + 20_000, validate: async () => {} });
			const start = performance.now();
			await createWriteToolDefinition(root, { operations }).execute("timing", { path, content: "y".repeat(bytes) });
			times.push(performance.now() - start);
			assert.equal((await readFile(path)).length, bytes);
		}
		times.sort((a, b) => a - b);
		t.diagnostic(`snapshot+recheck+write bytes=${bytes} trials=5 median=${times[2].toFixed(3)}ms max=${times[4].toFixed(3)}ms; checker/judge excluded`);
	}
});
