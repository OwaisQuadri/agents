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
		{ input: "vendor/lib/.git/index", expected: false },
		{ input: "a/.git/b", expected: false },
		{ input: "x/.git", expected: false },
		{ input: "x/.gitignore", expected: true },
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
	assert.equal(isRefreshWorthy("notgit/x", isIgnored), true);
	assert.equal(isRefreshWorthy("src/git/handler.ts", isIgnored), true);
});

test("isRefreshWorthy rejects a nested repository's bookkeeping", () => {
	assert.equal(isRefreshWorthy("a/.git/config", isIgnored), false);
	assert.equal(isRefreshWorthy("vendor/lib/.git/objects/ab/cd", isIgnored), false);
	assert.equal(isRefreshWorthy("deep/nest/vendor/.git", isIgnored), false);
	assert.equal(isRefreshWorthy("vendor\\lib\\.git\\index", isIgnored), false);
});

function makeTempRoot(t: { after: (fn: () => void) => void }): string {
	const root = fs.realpathSync(
		fs.mkdtempSync(path.join(os.tmpdir(), "live-diff-watch-")),
	);
	t.after(() => {
		fs.rmSync(root, { recursive: true, force: true });
	});
	return root;
}

const WATCH_DEADLINE_MS = 15_000;

// fs.watch(recursive) is FSEvents on macOS, and FSEvents arms ASYNCHRONOUSLY:
// a write issued immediately after watch() returns is missed outright, not
// merely delivered late. Every test here writes as soon as it has a watcher, so
// each one must first prove the watcher is live, then start from a clean slate.
async function armWatcher(root: string, seen: string[]): Promise<void> {
	const deadline = Date.now() + WATCH_DEADLINE_MS;
	for (let probe = 0; Date.now() < deadline; probe += 1) {
		const sentinel = path.join(root, `.arm-${probe}`);
		fs.writeFileSync(sentinel, "arm");
		await new Promise((resolve) => setTimeout(resolve, 50));
		if (seen.length > 0) {
			for (const entry of fs.readdirSync(root)) {
				if (entry.startsWith(".arm-")) {
					fs.rmSync(path.join(root, entry), { force: true });
				}
			}
			await quiesce(seen);
			return;
		}
	}
	throw new Error(`watcher never armed within ${WATCH_DEADLINE_MS}ms`);
}

// Wait for the event stream to go quiet, then drop everything the arming
// probes produced, so a test's assertions see only its own writes.
async function quiesce(seen: string[]): Promise<void> {
	let previous = -1;
	while (previous !== seen.length) {
		previous = seen.length;
		await new Promise((resolve) => setTimeout(resolve, 150));
	}
	seen.length = 0;
}

async function waitForChanges(
	seen: string[],
	minimum: number,
): Promise<void> {
	const deadline = Date.now() + WATCH_DEADLINE_MS;
	while (Date.now() < deadline) {
		if (seen.length >= minimum) {
			return;
		}
		await new Promise((resolve) => setTimeout(resolve, 20));
	}
	// A filesystem event that never arrives is a failed test, never a passed
	// one. Falling through silently turned FSEvents latency into an assertion
	// failure somewhere further down, which read as a defect in the watcher.
	throw new Error(
		`waited ${WATCH_DEADLINE_MS}ms for ${minimum} change(s); saw ${JSON.stringify(seen)}`,
	);
}

async function waitForPath(seen: string[], expected: string): Promise<void> {
	const deadline = Date.now() + WATCH_DEADLINE_MS;
	while (Date.now() < deadline) {
		if (seen.includes(expected)) {
			return;
		}
		await new Promise((resolve) => setTimeout(resolve, 20));
	}
	throw new Error(
		`waited ${WATCH_DEADLINE_MS}ms for ${JSON.stringify(expected)}; saw ${JSON.stringify(seen)}`,
	);
}

test("createWatcher reports a written file relative to root", async (t) => {
	const root = makeTempRoot(t);
	const seen: string[] = [];
	const watcher = createWatcher(root, (relativePath) => {
		seen.push(relativePath);
	});
	assert.notEqual(watcher, null);
	t.after(() => watcher?.close());
	await armWatcher(root, seen);

	fs.writeFileSync(path.join(root, "alpha.txt"), "one");
	await waitForPath(seen, "alpha.txt");

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
	await armWatcher(root, seen);

	fs.mkdirSync(path.join(root, "nested"));
	fs.writeFileSync(path.join(root, "nested", "beta.txt"), "two");
	await waitForPath(seen, path.join("nested", "beta.txt"));

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
	t.after(() => watcher?.close());
	await armWatcher(root, seen);

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
	// The self-close is what this test measures, so it must not fire during
	// arming: the arming probe is an event like any other and would close the
	// watcher before the test writes anything.
	let isArmed = false;
	const watcher = createWatcher(root, (relativePath) => {
		seen.push(relativePath);
		if (isArmed) {
			watcher?.close();
		}
	});
	assert.notEqual(watcher, null);
	t.after(() => watcher?.close());
	await armWatcher(root, seen);
	isArmed = true;

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
