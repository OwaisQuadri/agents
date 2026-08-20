import { test } from "node:test";
import assert from "node:assert/strict";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";

import { createWatcher, isRefreshWorthy } from "./watch.ts";

const IGNORED = new Set(["node_modules/x.js"]);
const isIgnored = (candidate: string): boolean => IGNORED.has(candidate);

test("isRefreshWorthy rejects git bookkeeping and ignored paths", () => {
	const table: Array<{ input: string; expected: boolean }> = [
		{ input: ".git/index", expected: false },
		{ input: ".git/objects/ab/cd", expected: false },
		{ input: ".git", expected: false },
		{ input: "node_modules/x.js", expected: false },
		{ input: "src/a.ts", expected: true },
		{ input: "src/gitlab.ts", expected: true },
	];
	for (const row of table) {
		assert.equal(
			isRefreshWorthy(row.input, isIgnored),
			row.expected,
			`${row.input} should be ${row.expected}`,
		);
	}
});

test("isRefreshWorthy keeps paths whose names merely contain git", () => {
	assert.equal(isRefreshWorthy("gitignore-notes.md", isIgnored), true);
	assert.equal(isRefreshWorthy("src/.gitkeep", isIgnored), true);
	assert.equal(isRefreshWorthy("a/.git/config", isIgnored), true);
});

function makeTempRoot(t: { after: (fn: () => void) => void }): string {
	const root = fs.mkdtempSync(path.join(os.tmpdir(), "live-diff-watch-"));
	t.after(() => {
		fs.rmSync(root, { recursive: true, force: true });
	});
	return root;
}

async function waitForChanges(
	seen: string[],
	minimum: number,
): Promise<void> {
	for (let attempt = 0; attempt < 100; attempt += 1) {
		if (seen.length >= minimum) {
			return;
		}
		await new Promise((resolve) => setTimeout(resolve, 20));
	}
}

test("createWatcher reports a written file relative to root", async (t) => {
	const root = makeTempRoot(t);
	const seen: string[] = [];
	const watcher = createWatcher(root, (relativePath) => {
		seen.push(relativePath);
	});
	assert.notEqual(watcher, null);
	t.after(() => watcher?.close());

	fs.writeFileSync(path.join(root, "alpha.txt"), "one");
	await waitForChanges(seen, 1);

	assert.ok(seen.includes("alpha.txt"), `saw ${JSON.stringify(seen)}`);
	assert.ok(
		seen.every((entry) => !path.isAbsolute(entry)),
		"paths must be relative to root",
	);
});

test("createWatcher reports nested paths relative to root", async (t) => {
	const root = makeTempRoot(t);
	const seen: string[] = [];
	const watcher = createWatcher(root, (relativePath) => {
		seen.push(relativePath);
	});
	assert.notEqual(watcher, null);
	t.after(() => watcher?.close());

	fs.mkdirSync(path.join(root, "nested"));
	fs.writeFileSync(path.join(root, "nested", "beta.txt"), "two");
	await waitForChanges(seen, 1);

	assert.ok(
		seen.some((entry) => entry === path.join("nested", "beta.txt")),
		`saw ${JSON.stringify(seen)}`,
	);
});

test("close stops further callbacks and is idempotent", async (t) => {
	const root = makeTempRoot(t);
	const seen: string[] = [];
	const watcher = createWatcher(root, (relativePath) => {
		seen.push(relativePath);
	});
	assert.notEqual(watcher, null);

	fs.writeFileSync(path.join(root, "before.txt"), "one");
	await waitForChanges(seen, 1);
	assert.ok(seen.length > 0, "watcher should report before close");

	watcher?.close();
	const countAtClose = seen.length;

	fs.writeFileSync(path.join(root, "after.txt"), "two");
	await new Promise((resolve) => setTimeout(resolve, 200));
	assert.equal(seen.length, countAtClose, "no callbacks after close");

	assert.doesNotThrow(() => watcher?.close());
	assert.doesNotThrow(() => watcher?.close());
});

test("close guards the callback itself, not only the handle", async (t) => {
	const root = makeTempRoot(t);
	const seen: string[] = [];
	const watcher = createWatcher(root, (relativePath) => {
		seen.push(relativePath);
		watcher?.close();
	});
	assert.notEqual(watcher, null);
	t.after(() => watcher?.close());

	fs.writeFileSync(path.join(root, "first.txt"), "one");
	fs.writeFileSync(path.join(root, "second.txt"), "two");
	fs.writeFileSync(path.join(root, "third.txt"), "three");
	await waitForChanges(seen, 1);
	await new Promise((resolve) => setTimeout(resolve, 200));

	assert.equal(seen.length, 1, `closing inside the callback must stop the rest, saw ${JSON.stringify(seen)}`);
});

test("a nonexistent root returns null instead of throwing", (t) => {
	const root = makeTempRoot(t);
	const missing = path.join(root, "no-such-directory");
	let watcher: ReturnType<typeof createWatcher> = null;
	assert.doesNotThrow(() => {
		watcher = createWatcher(missing, () => {});
	});
	assert.equal(watcher, null);
});
