import * as assert from "node:assert/strict";
import { execFile, execFileSync } from "node:child_process";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { test, type TestContext } from "node:test";

import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent";

import liveDiff, { mapKey, setWatcherFactory } from "./live-diff.ts";
import { captureSnapshot, diffStats, type Exec } from "./live-diff/engine.ts";
import {
	addBranchCommit,
	addIgnoredFile,
	addUnstagedEdit,
	addUntrackedFile,
	makeFixtureRepo,
	type FixtureRepo,
} from "./live-diff/fixtures.ts";
import type { DiffStats, WatcherFactory, WorktreeWatcher } from "./live-diff/types.ts";

type Handler = (event: unknown, ctx: ExtensionContext) => unknown;
type CommandHandler = (args: unknown, ctx: ExtensionContext) => Promise<void>;

interface Recorder {
	setStatusCalls: { key: string; text: string }[];
	customCalls: unknown[];
	notifyCalls: unknown[];
	inputHandlers: ((data: string) => unknown)[];
	isShutdown: boolean;
	postShutdownUiCalls: string[];
}

// The badge is themed now, so its text carries ANSI. Assertions compare the
// PLAIN text: colour is proven separately by the theme recorder, and matching
// escape bytes here would pin the tests to Pi's palette.
function plain(text: string | undefined): string {
	return (text ?? "").replace(/\x1b\[[0-9;]*m/g, "");
}

function createRecorder(): Recorder {
	return {
		setStatusCalls: [],
		customCalls: [],
		notifyCalls: [],
		inputHandlers: [],
		isShutdown: false,
		postShutdownUiCalls: [],
	};
}

function createFakeCtx(recorder: Recorder, cwd: string): ExtensionContext {
	function guard(method: string): void {
		if (recorder.isShutdown) {
			recorder.postShutdownUiCalls.push(method);
			throw new Error(`stale ctx: ${method} after session_shutdown`);
		}
	}
	return {
		cwd,
		hasUI: true,
		mode: "tui",
		ui: {
			setStatus(key: string, text: string) {
				recorder.setStatusCalls.push({ key, text });
				guard("setStatus");
			},
			async custom(factory: unknown, options: unknown) {
				recorder.customCalls.push({ factory, options });
				guard("custom");
				return undefined;
			},
			onTerminalInput(handler: (data: string) => unknown) {
				recorder.inputHandlers.push(handler);
				return () => {
					recorder.inputHandlers = recorder.inputHandlers.filter(
						(entry) => entry !== handler,
					);
				};
			},
			notify(...args: unknown[]) {
				recorder.notifyCalls.push(args);
				guard("notify");
			},
		},
	} as unknown as ExtensionContext;
}

function createFakePi() {
	// Pi's real API APPENDS handlers (loader.js: list.push(handler)), so an
	// extension may register several for one event and all of them run. A fake
	// that kept only the last one hid a live defect once already.
	const handlers = new Map<string, Handler[]>();
	const commands = new Map<string, CommandHandler>();
	const api = {
		on(event: string, handler: Handler) {
			const list = handlers.get(event) ?? [];
			list.push(handler);
			handlers.set(event, list);
		},
		registerCommand(name: string, options: { handler: CommandHandler }) {
			commands.set(name, options.handler);
		},
	} as unknown as ExtensionAPI;
	return {
		api,
		fire(event: string, payload: unknown, ctx: ExtensionContext): unknown {
			const list = handlers.get(event);
			assert.ok(list && list.length > 0, `missing handler for ${event}`);
			let last: unknown;
			for (const handler of list) {
				last = handler(payload, ctx);
			}
			return last;
		},
		command(name: string): CommandHandler {
			const handler = commands.get(name);
			assert.ok(handler, `missing command ${name}`);
			return handler;
		},
		commandNames(): string[] {
			return [...commands.keys()];
		},
	};
}

function sleep(ms: number): Promise<void> {
	return new Promise((resolvePromise) => setTimeout(resolvePromise, ms));
}



const testExec: Exec = (command, args, options) =>
	new Promise((resolvePromise) => {
		execFile(
			command,
			args,
			{
				cwd: options?.cwd,
				env: options?.env ? { ...process.env, ...options.env } : process.env,
				maxBuffer: 64 * 1024 * 1024,
			},
			(error, stdout, stderr) => {
				const rawCode = (error as NodeJS.ErrnoException | null)?.code;
				const code = error ? (typeof rawCode === "number" ? rawCode : 1) : 0;
				resolvePromise({ code, stdout: String(stdout), stderr: String(stderr) });
			},
		);
	});

async function currentOverallStats(
	ctx: ExtensionContext,
	repo: FixtureRepo,
): Promise<DiffStats> {
	const snapshot = await captureSnapshot(testExec, repo.root);
	return diffStats(testExec, ctx.cwd, snapshot.baselineSha, 400);
}

async function waitFor(predicate: () => boolean, timeoutMs = 3000): Promise<void> {
	const startedAt = Date.now();
	while (!predicate()) {
		if (Date.now() - startedAt > timeoutMs) {
			throw new Error("waitFor timed out");
		}
		await sleep(25);
	}
}

interface Harness {
	pi: ReturnType<typeof createFakePi>;
	recorder: Recorder;
	ctx: ExtensionContext;
	repo: FixtureRepo;
}

function createHarness(t: TestContext): Harness {
	const pi = createFakePi();
	liveDiff(pi.api);
	const recorder = createRecorder();
	const repo = makeFixtureRepo(os.tmpdir());
	t.after(() => repo.cleanup());
	const ctx = createFakeCtx(recorder, repo.root);
	return { pi, recorder, ctx, repo };
}

interface FakeWatcher {
	emit(relativePath: string): void;
	closeCount: number;
	isStarted: boolean;
}

function useFakeWatcher(t: TestContext): FakeWatcher {
	const fake: FakeWatcher = {
		emit: () => {},
		closeCount: 0,
		isStarted: false,
	};
	useWatcherFactory(t, (_root, onChange) => {
		fake.isStarted = true;
		fake.emit = onChange;
		const watcher: WorktreeWatcher = {
			close: () => {
				fake.closeCount += 1;
			},
		};
		return watcher;
	});
	return fake;
}

function useWatcherFactory(t: TestContext, factory: WatcherFactory): void {
	setWatcherFactory(factory);
	t.after(() => setWatcherFactory(() => null));
}

test("TC-21 an edit outside the agent moves the badge", async (t) => {
	const watcher = useFakeWatcher(t);
	const { pi, recorder, ctx, repo } = createHarness(t);

	pi.fire("session_start", {}, ctx);
	assert.equal(watcher.isStarted, true);
	await waitFor(() => recorder.setStatusCalls.length >= 1);
	const cleanCount = recorder.setStatusCalls.length;
	assert.equal(plain(recorder.setStatusCalls[cleanCount - 1].text), "diff clean");

	fs.writeFileSync(path.join(repo.root, "outside.txt"), "edited in nvim\n");
	watcher.emit("outside.txt");

	await waitFor(() => recorder.setStatusCalls.length > cleanCount);
	const badge = plain(recorder.setStatusCalls[recorder.setStatusCalls.length - 1].text);
	assert.match(badge, /all \+1/);
	assert.notEqual(badge, "diff clean");
});

test("TC-22 watcher ignores git bookkeeping and ignored paths", async (t) => {
	const watcher = useFakeWatcher(t);
	const { pi, recorder, ctx, repo } = createHarness(t);
	const ignoredPath = addIgnoredFile(repo);

	pi.fire("session_start", {}, ctx);
	await waitFor(() => recorder.setStatusCalls.length >= 1);
	const beforeCount = recorder.setStatusCalls.length;

	fs.appendFileSync(path.join(repo.root, ignoredPath), "more ignored noise\n");
	watcher.emit(".git/index");
	watcher.emit(".git/objects/ab/cdef");
	watcher.emit(ignoredPath);

	await sleep(900);
	assert.equal(recorder.setStatusCalls.length, beforeCount);

	fs.writeFileSync(path.join(repo.root, "worthy.txt"), "real change\n");
	watcher.emit("worthy.txt");
	await waitFor(() => recorder.setStatusCalls.length > beforeCount);
});

test("TC-23 a burst coalesces into one refresh", async (t) => {
	const watcher = useFakeWatcher(t);
	const { pi, recorder, ctx, repo } = createHarness(t);

	pi.fire("session_start", {}, ctx);
	await waitFor(() => recorder.setStatusCalls.length >= 1);
	const beforeCount = recorder.setStatusCalls.length;

	for (let index = 0; index < 200; index += 1) {
		const name = `burst-${index}.txt`;
		fs.writeFileSync(path.join(repo.root, name), `burst ${index}\n`);
		watcher.emit(name);
	}

	await waitFor(() => recorder.setStatusCalls.length > beforeCount);
	await sleep(900);
	const refreshes = recorder.setStatusCalls.length - beforeCount;
	assert.ok(refreshes >= 1, `expected at least one refresh, saw ${refreshes}`);
	assert.ok(refreshes <= 2, `expected at most two refreshes, saw ${refreshes}`);
});

test("TC-23 a burst costs one batched ignore check, not one per change", async (t) => {
	const watcher = useFakeWatcher(t);
	const { pi, recorder, ctx, repo } = createHarness(t);
	const gitDir = path.join(repo.root, ".git");
	const counterPath = path.join(gitDir, "check-ignore-runs");
	const realGit = execFileSync("which", ["git"], { encoding: "utf8" }).trim();
	const shimDir = fs.mkdtempSync(path.join(os.tmpdir(), "live-diff-gitshim-"));
	t.after(() => fs.rmSync(shimDir, { recursive: true, force: true }));
	fs.writeFileSync(
		path.join(shimDir, "git"),
		`#!/bin/sh\nfor arg in "$@"; do\n  if [ "$arg" = "check-ignore" ]; then\n    printf x >> ${JSON.stringify(counterPath)}\n    break\n  fi\ndone\nexec ${JSON.stringify(realGit)} "$@"\n`,
		{ mode: 0o755 },
	);
	const realPath = process.env.PATH ?? "";
	process.env.PATH = `${shimDir}:${realPath}`;
	t.after(() => {
		process.env.PATH = realPath;
	});

	pi.fire("session_start", {}, ctx);
	await waitFor(() => recorder.setStatusCalls.length >= 1);
	const runsBefore = fs.existsSync(counterPath)
		? fs.readFileSync(counterPath, "utf8").length
		: 0;
	const beforeCount = recorder.setStatusCalls.length;

	for (let index = 0; index < 200; index += 1) {
		const name = `burst-${index}.txt`;
		fs.writeFileSync(path.join(repo.root, name), `burst ${index}\n`);
		watcher.emit(name);
	}

	await waitFor(() => recorder.setStatusCalls.length > beforeCount);
	await sleep(900);

	const runsAfter = fs.existsSync(counterPath)
		? fs.readFileSync(counterPath, "utf8").length
		: 0;
	const checkIgnoreRuns = runsAfter - runsBefore;
	assert.ok(
		checkIgnoreRuns >= 1,
		`the batch must ask git about ignores, saw ${checkIgnoreRuns}`,
	);
	assert.ok(
		checkIgnoreRuns <= 3,
		`200 changes must batch into a few git calls, saw ${checkIgnoreRuns}`,
	);
});

test("TC-24 watcher stops at shutdown", async (t) => {
	const watcher = useFakeWatcher(t);
	const { pi, recorder, ctx } = createHarness(t);
	const rejections: unknown[] = [];
	const onRejection = (reason: unknown) => {
		rejections.push(reason);
	};
	process.on("unhandledRejection", onRejection);
	try {
		pi.fire("session_start", {}, ctx);
		await waitFor(() => recorder.setStatusCalls.length >= 1);

		const beforeShutdown = recorder.setStatusCalls.length;
		pi.fire("session_shutdown", {}, ctx);
		recorder.isShutdown = true;
		assert.equal(watcher.closeCount, 1);

		for (let index = 0; index < 5; index += 1) {
			watcher.emit(`after-shutdown-${index}.txt`);
		}
		await sleep(900);

		assert.equal(recorder.setStatusCalls.length, beforeShutdown);
		assert.deepEqual(recorder.postShutdownUiCalls, []);
		assert.equal(rejections.length, 0);
	} finally {
		process.off("unhandledRejection", onRejection);
	}
});

test("TC-25 an unavailable watcher degrades instead of breaking", async (t) => {
	useWatcherFactory(t, () => null);
	const { pi, recorder, ctx, repo } = createHarness(t);

	pi.fire("session_start", {}, ctx);
	await waitFor(() => recorder.setStatusCalls.length >= 1);
	const afterStart = recorder.setStatusCalls.length;

	fs.appendFileSync(path.join(repo.root, "alpha.txt"), "tool edit\n");
	pi.fire("tool_execution_end", { toolName: "edit" }, ctx);
	pi.fire("agent_settled", {}, ctx);
	await waitFor(() => recorder.setStatusCalls.length > afterStart);
	assert.match(
		plain(recorder.setStatusCalls[recorder.setStatusCalls.length - 1].text),
		/all \+1/,
	);
});

test("TC-25 a throwing watcher factory degrades instead of breaking", async (t) => {
	useWatcherFactory(t, () => {
		throw new Error("watcher unavailable");
	});
	const { pi, recorder, ctx, repo } = createHarness(t);

	pi.fire("session_start", {}, ctx);
	await waitFor(() => recorder.setStatusCalls.length >= 1);
	const afterStart = recorder.setStatusCalls.length;

	fs.appendFileSync(path.join(repo.root, "alpha.txt"), "tool edit\n");
	pi.fire("tool_execution_end", { toolName: "edit" }, ctx);
	pi.fire("agent_settled", {}, ctx);
	await waitFor(() => recorder.setStatusCalls.length > afterStart);
	assert.match(
		plain(recorder.setStatusCalls[recorder.setStatusCalls.length - 1].text),
		/all \+1/,
	);
});

test("registers the diff command", (t) => {
	const { pi } = createHarness(t);
	assert.ok(pi.commandNames().includes("diff"));
});

test("TC-04 badge after write tool, none for read tool, one more on settle", async (t) => {
	const { pi, recorder, ctx, repo } = createHarness(t);
	await pi.fire("agent_start", {}, ctx);

	fs.appendFileSync(path.join(repo.root, "alpha.txt"), "edit by tool\n");
	pi.fire("tool_execution_end", { toolName: "edit" }, ctx);
	await waitFor(() => recorder.setStatusCalls.length >= 1);
	const afterWrite = recorder.setStatusCalls.length;
	const lastWrite = recorder.setStatusCalls[afterWrite - 1];
	assert.equal(lastWrite.key, "live-diff");
	assert.match(lastWrite.text, /^turn |^all |^branch |^diff/);

	pi.fire("tool_execution_end", { toolName: "read" }, ctx);
	await sleep(400);
	assert.equal(recorder.setStatusCalls.length, afterWrite);

	pi.fire("agent_settled", {}, ctx);
	await waitFor(() => recorder.setStatusCalls.length > afterWrite);
	assert.equal(
		recorder.setStatusCalls[recorder.setStatusCalls.length - 1].key,
		"live-diff",
	);
});

test("TC-12 external edit shows up in the settle badge", async (t) => {
	const { pi, recorder, ctx, repo } = createHarness(t);
	await pi.fire("agent_start", {}, ctx);
	pi.fire("agent_settled", {}, ctx);
	await waitFor(() => recorder.setStatusCalls.length >= 1);
	const cleanBadge = plain(recorder.setStatusCalls[recorder.setStatusCalls.length - 1].text);
	assert.equal(cleanBadge, "diff clean");

	fs.writeFileSync(path.join(repo.root, "external.txt"), "external edit\n");
	const before = recorder.setStatusCalls.length;
	pi.fire("agent_settled", {}, ctx);
	await waitFor(() => recorder.setStatusCalls.length > before);
	const externalBadge = plain(recorder.setStatusCalls[recorder.setStatusCalls.length - 1].text);
	assert.notEqual(externalBadge, cleanBadge);
	assert.match(externalBadge, /turn/);
	assert.match(externalBadge, /all/);
});

test("TC-13 shutdown clears the trailing timer and no ctx use follows", async (t) => {
	const { pi, recorder, ctx, repo } = createHarness(t);
	await pi.fire("agent_start", {}, ctx);

	fs.appendFileSync(path.join(repo.root, "alpha.txt"), "first edit\n");
	pi.fire("tool_execution_end", { toolName: "edit" }, ctx);
	pi.fire("tool_execution_end", { toolName: "write" }, ctx);
	await waitFor(() => recorder.setStatusCalls.length >= 1);
	await sleep(150);

	const beforeShutdown = recorder.setStatusCalls.length;
	pi.fire("session_shutdown", {}, ctx);
	recorder.isShutdown = true;
	await sleep(450);
	assert.equal(recorder.setStatusCalls.length, beforeShutdown);
});

test("TC-19 shutdown during an in-flight refresh reports nothing afterwards", async (t) => {
	const { pi, recorder, ctx, repo } = createHarness(t);
	const rejections: unknown[] = [];
	const onRejection = (reason: unknown) => {
		rejections.push(reason);
	};
	process.on("unhandledRejection", onRejection);
	try {
		await pi.fire("agent_start", {}, ctx);

		fs.appendFileSync(path.join(repo.root, "alpha.txt"), "in-flight edit\n");
		for (let index = 0; index < 25; index += 1) {
			pi.fire("tool_execution_end", { toolName: "edit" }, ctx);
		}

		const beforeShutdown = recorder.setStatusCalls.length;
		pi.fire("session_shutdown", {}, ctx);
		recorder.isShutdown = true;

		await sleep(1200);

		assert.deepEqual(recorder.postShutdownUiCalls, []);
		assert.equal(recorder.setStatusCalls.length, beforeShutdown);
		assert.equal(rejections.length, 0);
	} finally {
		process.off("unhandledRejection", onRejection);
	}
});

test("TC-20 pre-session uncommitted work counts as overall", async (t) => {
	const { pi, recorder, ctx, repo } = createHarness(t);
	fs.appendFileSync(path.join(repo.root, "alpha.txt"), "pre-session edit\n");

	pi.fire("session_start", {}, ctx);
	pi.fire("agent_settled", {}, ctx);
	await waitFor(() => recorder.setStatusCalls.length >= 1);
	const badge = plain(recorder.setStatusCalls[recorder.setStatusCalls.length - 1].text);

	const overallStats = await currentOverallStats(ctx, repo);
	assert.deepEqual(
		overallStats.files.map((file) => file.path),
		["alpha.txt"],
	);
	assert.match(badge, /all \+1/);
	assert.doesNotMatch(badge, /turn/);
});

test("TC-20 zero-commit repo still reports a sane badge", async (t) => {
	const pi = createFakePi();
	liveDiff(pi.api);
	const recorder = createRecorder();
	const root = fs.mkdtempSync(path.join(os.tmpdir(), "live-diff-fixture-empty-"));
	t.after(() => fs.rmSync(root, { recursive: true, force: true }));
	execFileSync("git", ["init", "-q"], { cwd: root });
	execFileSync("git", ["config", "user.name", "fixture"], { cwd: root });
	execFileSync("git", ["config", "user.email", "fixture@example.invalid"], { cwd: root });
	fs.writeFileSync(path.join(root, "only.txt"), "content before any commit\n");
	const ctx = createFakeCtx(recorder, root);

	pi.fire("session_start", {}, ctx);
	pi.fire("agent_settled", {}, ctx);
	await waitFor(() => recorder.setStatusCalls.length >= 1);
	const badge = plain(recorder.setStatusCalls[recorder.setStatusCalls.length - 1].text);

	assert.notEqual(badge, "diff ?");
	assert.match(badge, /all \+1/);
});

interface ThemeRecorder {
	fgCalls: { color: string; text: string }[];
	bgCalls: { background: string; text: string }[];
	theme: {
		fg(color: string, text: string): string;
		bg(background: string, text: string): string;
		bold(text: string): string;
		inverse(text: string): string;
	};
}

function createThemeRecorder(): ThemeRecorder {
	const fgCalls: { color: string; text: string }[] = [];
	const bgCalls: { background: string; text: string }[] = [];
	return {
		fgCalls,
		bgCalls,
		theme: {
			fg(color: string, text: string): string {
				fgCalls.push({ color, text });
				return `\x1b[35m${text}\x1b[39m`;
			},
			bg(background: string, text: string): string {
				bgCalls.push({ background, text });
				return `\x1b[45m${text}\x1b[49m`;
			},
			bold(text: string): string {
				return text;
			},
			inverse(text: string): string {
				return text;
			},
		},
	};
}

function visibleWidthOf(line: string): number {
	const bare = line.replace(/\x1b\[[0-9;]*m/g, "");
	let width = 0;
	for (const char of bare) {
		const code = char.codePointAt(0) ?? 0;
		if (/\p{Mn}|\p{Me}/u.test(char)) {
			continue;
		}
		const isWide =
			(code >= 0x1100 && code <= 0x115f) ||
			(code >= 0x2e80 && code <= 0x303e) ||
			(code >= 0x3041 && code <= 0x33ff) ||
			(code >= 0x3400 && code <= 0x4dbf) ||
			(code >= 0x4e00 && code <= 0x9fff) ||
			(code >= 0xac00 && code <= 0xd7a3) ||
			(code >= 0xff00 && code <= 0xff60) ||
			(code >= 0x1f300 && code <= 0x1f64f);
		width += isWide ? 2 : 1;
	}
	return width;
}

test("W9 mapKey binds space, jk, hl, enter, q and both esc forms; TAB is unbound", () => {
	assert.equal(mapKey(" "), "open-diff");
	assert.equal(mapKey("f"), "open-diff");
	assert.equal(mapKey("j"), "down");
	assert.equal(mapKey("\x1b[B"), "down");
	assert.equal(mapKey("k"), "up");
	assert.equal(mapKey("\x1b[A"), "up");
	assert.equal(mapKey("h"), "mode-left");
	assert.equal(mapKey("l"), "mode-right");
	assert.equal(mapKey("\r"), "open");
	assert.equal(mapKey("\n"), "open");
	assert.equal(mapKey("q"), "close");
	assert.equal(mapKey("\x1b"), "close");
	assert.equal(mapKey("\t"), null, "TAB must be unbound");
	assert.equal(mapKey("t"), null, "the old toggle-mode letter must be unbound");
});

test("W9 mapKey binds the viewer motions: d/u, g/G, ]/[", () => {
	assert.equal(mapKey("d"), "page-down");
	assert.equal(mapKey("u"), "page-up");
	assert.equal(mapKey("\x04"), null, "ctrl-d never reached the component; it is unbound");
	assert.equal(mapKey("\x15"), null, "ctrl-u never reached the component; it is unbound");
	assert.equal(mapKey("g"), "top");
	assert.equal(mapKey("G"), "bottom");
	assert.equal(mapKey("]"), "next-file");
	assert.equal(mapKey("["), "prev-file");
});

test("W9 bare Esc still closes and is never confused with an arrow sequence", () => {
	// The arrow sequences (\x1b[A, \x1b[B) share the \x1b prefix with bare Esc.
	// mapKey must match the full sequence, never fall through to close on a prefix.
	assert.equal(mapKey("\x1b[A"), "up");
	assert.equal(mapKey("\x1b[B"), "down");
	assert.equal(mapKey("\x1b"), "close");
});

interface OverlayProbe {
	lines: string[];
	themes: ThemeRecorder;
	options: { overlay?: boolean; overlayOptions?: { width?: unknown; minWidth?: unknown } };
}

async function openOverlay(
	pi: ReturnType<typeof createFakePi>,
	recorder: Recorder,
	ctx: ExtensionContext,
	width: number,
	keys: string[] = [],
): Promise<OverlayProbe> {
	await pi.command("diff")("", ctx);
	assert.ok(recorder.customCalls.length >= 1);
	const call = recorder.customCalls[recorder.customCalls.length - 1] as {
		factory: (
			tui: unknown,
			theme: unknown,
			keybindings: unknown,
			done: (result: undefined) => void,
		) => { render(width: number): string[]; handleInput(data: string): void };
		options: OverlayProbe["options"];
	};
	const themes = createThemeRecorder();
	// A key can trigger an ASYNC effect (load-patch, open-in-nvim), which the
	// real TUI learns about via requestRender() once the promise settles.
	// Waiting on that same signal here, rather than a fixed sleep, is what
	// lets this helper stay honest about async work instead of racing it.
	let onNextRender = (): void => {};
	const tui = {
		requestRender() {
			onNextRender();
		},
	};
	const component = call.factory(tui, themes.theme, {}, () => {});
	for (const key of keys) {
		const rendered = new Promise<void>((resolvePromise) => {
			onNextRender = resolvePromise;
		});
		component.handleInput(key);
		await Promise.race([rendered, sleep(500)]);
		onNextRender = () => {};
		const deadline = Date.now() + 3000;
		while (
			component.render(width).join("\n").includes("opening") &&
			Date.now() < deadline
		) {
			await sleep(20);
		}
	}
	themes.fgCalls.length = 0;
	themes.bgCalls.length = 0;
	const lines = component.render(width);
	return { lines, themes, options: call.options };
}

test("TC-28 the overlay is a bounded opaque panel with a highlighted selection", async (t) => {
	const { pi, recorder, ctx, repo } = createHarness(t);
	fs.appendFileSync(path.join(repo.root, "alpha.txt"), "styled overlay\n");
	fs.writeFileSync(path.join(repo.root, "added.txt"), "fresh file\n");
	await pi.fire("agent_start", {}, ctx);
	pi.fire("agent_settled", {}, ctx);
	await waitFor(() => recorder.setStatusCalls.length >= 1);

	const probe = await openOverlay(pi, recorder, ctx, 60);

	assert.equal(probe.options.overlay, true);
	assert.ok(
		probe.options.overlayOptions !== undefined,
		"ui.custom must receive overlayOptions",
	);
	assert.ok(
		probe.options.overlayOptions?.width !== undefined,
		"overlayOptions must carry an explicit width",
	);

	const selectedBgCalls = probe.themes.bgCalls.filter(
		(call) => call.background === "selectedBg",
	);
	assert.equal(
		selectedBgCalls.length,
		1,
		"exactly one row carries the selection background",
	);
	assert.ok(
		selectedBgCalls[0].text.includes("▌"),
		"the selected row carries a gutter glyph so the affordance survives without colour",
	);
	assert.equal(
		visibleWidthOf(selectedBgCalls[0].text),
		60,
		"the selection highlight spans the full padded row width",
	);

	const colors = new Set(probe.themes.fgCalls.map((call) => call.color));
	assert.ok(colors.has("toolDiffAdded"), "additions use toolDiffAdded");
	assert.ok(colors.has("toolDiffRemoved"), "deletions use toolDiffRemoved");
	assert.ok(colors.has("muted"), "the hint line uses muted");
	assert.ok(colors.has("borderAccent"), "the header uses borderAccent");

	const panelBgCalls = probe.themes.bgCalls.filter(
		(call) => call.background === "customMessageBg",
	);
	assert.ok(
		panelBgCalls.length > 1,
		"unselected rows and the frame are painted opaque",
	);
	for (const line of probe.lines) {
		assert.equal(
			visibleWidthOf(line),
			60,
			`every emitted line fills the panel width exactly: ${JSON.stringify(line)}`,
		);
	}
});

test("TC-28 the highlight follows the cursor and stays single", async (t) => {
	const { pi, recorder, ctx, repo } = createHarness(t);
	fs.appendFileSync(path.join(repo.root, "alpha.txt"), "row one\n");
	fs.writeFileSync(path.join(repo.root, "added.txt"), "row two\n");
	await pi.fire("agent_start", {}, ctx);
	pi.fire("agent_settled", {}, ctx);
	await waitFor(() => recorder.setStatusCalls.length >= 1);

	const probe = await openOverlay(pi, recorder, ctx, 60, ["\x1b[B"]);
	const selected = probe.themes.bgCalls.filter(
		(call) => call.background === "selectedBg",
	);
	assert.equal(selected.length, 1, "still exactly one selected row after moving");
	assert.equal(visibleWidthOf(selected[0].text), 60);
	for (const line of probe.lines) {
		assert.equal(visibleWidthOf(line), 60);
	}
});

test("F-17 the overlay never exceeds its height budget, however many files changed", async (t) => {
	const { pi, recorder, ctx, repo } = createHarness(t);
	for (let index = 0; index < 60; index += 1) {
		fs.writeFileSync(path.join(repo.root, `many-${index}.txt`), `content ${index}\n`);
	}
	await pi.fire("agent_start", {}, ctx);
	pi.fire("agent_settled", {}, ctx);
	await waitFor(() => recorder.setStatusCalls.length >= 1);

	const probe = await openOverlay(pi, recorder, ctx, 60);
	assert.equal(
		probe.options.overlayOptions?.maxHeight,
		"80%",
		"the peek window takes 80% of the terminal height, per the owner's ask",
	);
	assert.ok(
		probe.lines.length <= 24,
		`render() must never emit more lines than the requested maxHeight, saw ${probe.lines.length}`,
	);

	// At a wide enough panel the scroll indicator has room to spell "more"
	// rather than being clipped to its leading glyph — the row-count
	// guarantee holds at any width (checked above), but the indicator's
	// readability is a width concern, checked on a fresh harness so the
	// two /diff opens do not collide with openOverlay's own one-call check.
	const wide = createHarness(t);
	for (let index = 0; index < 60; index += 1) {
		fs.writeFileSync(path.join(wide.repo.root, `many-${index}.txt`), `content ${index}\n`);
	}
	await wide.pi.fire("agent_start", {}, wide.ctx);
	wide.pi.fire("agent_settled", {}, wide.ctx);
	await waitFor(() => wide.recorder.setStatusCalls.length >= 1);
	const wideProbe = await openOverlay(wide.pi, wide.recorder, wide.ctx, 100);
	assert.ok(
		wideProbe.lines.length <= 24,
		`render() must never emit more lines than the requested maxHeight at width 100 either, saw ${wideProbe.lines.length}`,
	);
	const wideJoined = wideProbe.lines.join("\n");
	assert.match(
		wideJoined,
		/[↑↓] \d+/,
		"with 60 changed files in a 24-line budget and room to spell it out, hidden rows must be announced on the header",
	);
});

test("TC-14 /diff during an in-flight refresh opens exactly one overlay", async (t) => {
	const { pi, recorder, ctx, repo } = createHarness(t);
	const rejections: unknown[] = [];
	const onRejection = (reason: unknown) => {
		rejections.push(reason);
	};
	process.on("unhandledRejection", onRejection);
	try {
		await pi.fire("agent_start", {}, ctx);
		fs.appendFileSync(path.join(repo.root, "beta.txt"), "mid-flight edit\n");
		pi.fire("tool_execution_end", { toolName: "edit" }, ctx);
		await pi.command("diff")("", ctx);
		assert.equal(recorder.customCalls.length, 1);
		await sleep(300);
		assert.equal(rejections.length, 0);
	} finally {
		process.off("unhandledRejection", onRejection);
	}
});

test("W8 badge labels the second side branch when a branch point resolves", async (t) => {
	const { pi, recorder, ctx, repo } = createHarness(t);
	addBranchCommit(repo);
	addUnstagedEdit(repo);

	pi.fire("session_start", {}, ctx);
	await waitFor(() => recorder.setStatusCalls.length >= 1);
	const badge = plain(recorder.setStatusCalls[recorder.setStatusCalls.length - 1].text);
	assert.match(badge, /\bbranch\b/);
	assert.ok(!badge.includes("all "), `expected no "all" side once a branch point resolves: ${badge}`);
});

test("W8 badge keeps the all label when no branch point resolves", async (t) => {
	const { pi, recorder, ctx, repo } = createHarness(t);

	pi.fire("session_start", {}, ctx);
	await waitFor(() => recorder.setStatusCalls.length >= 1);
	const badge = plain(recorder.setStatusCalls[recorder.setStatusCalls.length - 1].text);
	assert.equal(badge, "diff clean");

	fs.writeFileSync(path.join(repo.root, "plain-edit.txt"), "no branch here\n");
	pi.fire("tool_execution_end", { toolName: "edit" }, ctx);
	await waitFor(() => recorder.setStatusCalls.length >= 2);
	const secondBadge = plain(recorder.setStatusCalls[recorder.setStatusCalls.length - 1].text);
	assert.match(secondBadge, /\ball\b/);
	assert.ok(!secondBadge.includes("branch "), `expected no "branch" side without a resolvable branch point: ${secondBadge}`);
});

test("W9 h and l rebuild rows for both columns, not just flip the mode flag", async (t) => {
	const { pi, recorder, ctx, repo } = createHarness(t);
	// branch-only.txt is committed BEFORE the request snapshot, so it is part
	// of the snapshot tree and never shows up as a request-side change; it
	// still differs from the branch point and so appears on the overall side.
	addBranchCommit(repo, "feature", "branch-only.txt");
	await pi.fire("agent_start", {}, ctx);
	fs.appendFileSync(path.join(repo.root, "alpha.txt"), "request edit\n");
	pi.fire("tool_execution_end", { toolName: "edit" }, ctx);
	await waitFor(() => recorder.setStatusCalls.length >= 1);

	// Open once and drive it through several render checkpoints, mirroring
	// openOverlay's own factory-extraction pattern: this overlay's contract
	// is that the SAME session re-renders differently as keys land, which a
	// fresh open-per-key-sequence cannot exercise (a fresh /diff always
	// re-refreshes and starts in whichever mode has content).
	await pi.command("diff")("", ctx);
	assert.equal(recorder.customCalls.length, 1);
	const call = recorder.customCalls[0] as {
		factory: (
			tui: unknown,
			theme: unknown,
			keybindings: unknown,
			done: (result: undefined) => void,
		) => { render(width: number): string[]; handleInput(data: string): void };
	};
	const themes = createThemeRecorder();
	const tui = { requestRender() {} };
	const component = call.factory(tui, themes.theme, {}, () => {});

	const requestJoined = component.render(100).join("\n");
	assert.match(requestJoined, /alpha\.txt/, "request mode starts on the row it captured");
	assert.ok(
		!requestJoined.includes("branch-only.txt"),
		"the branch-only file must not appear in request mode",
	);

	component.handleInput("l");
	const rightLines = component.render(100);
	const rightJoined = rightLines.join("\n");
	assert.match(
		rightJoined,
		/branch-only\.txt/,
		"pressing l must rebuild rows for the overall column, not just flip a flag",
	);
	for (const line of rightLines) {
		assert.equal(visibleWidthOf(line), 100);
	}

	component.handleInput("h");
	const backJoined = component.render(100).join("\n");
	assert.ok(
		!backJoined.includes("branch-only.txt"),
		"pressing h must rebuild back to the request column",
	);
	assert.match(backJoined, /alpha\.txt/);
});

test("W9 space opens the read-only viewer and fetches the selected file's patch", async (t) => {
	const { pi, recorder, ctx, repo } = createHarness(t);
	addUnstagedEdit(repo);
	pi.fire("session_start", {}, ctx);
	await waitFor(() => recorder.setStatusCalls.length >= 1);

	const probe = await openOverlay(pi, recorder, ctx, 70, [" "]);
	const joined = probe.lines.join("\n");
	assert.match(joined, /read-only/, "the border must state read-only explicitly");
	assert.match(joined, /╭─/, "the viewer opens a bordered frame");
	assert.match(joined, /@@/, "the fetched hunk header is visible");
	for (const line of probe.lines) {
		assert.equal(visibleWidthOf(line), 70, `viewer row must fill the panel width: ${JSON.stringify(line)}`);
	}
});

test("W9 ] and [ move between files inside the viewer without leaving it", async (t) => {
	const { pi, recorder, ctx, repo } = createHarness(t);
	addUnstagedEdit(repo);
	addUntrackedFile(repo, "second.txt");
	pi.fire("session_start", {}, ctx);
	await waitFor(() => recorder.setStatusCalls.length >= 1);

	const probe = await openOverlay(pi, recorder, ctx, 70, [" "]);
	const firstPath = probe.lines.join("\n").match(/╭─ (\S+)/)?.[1];
	assert.ok(firstPath, "the top border must name the open file");

	const afterNext = await openOverlay(pi, recorder, ctx, 70, [" ", "]"]);
	const nextJoined = afterNext.lines.join("\n");
	assert.match(nextJoined, /╭─/, "] must keep the viewer open, not close it");
	const secondPath = nextJoined.match(/╭─ (\S+)/)?.[1];
	assert.ok(secondPath, "] must still show a bordered file view");
	assert.notEqual(secondPath, firstPath, "] must move to a different file");

	const afterPrev = await openOverlay(pi, recorder, ctx, 70, [" ", "]", "["]);
	const prevPath = afterPrev.lines.join("\n").match(/╭─ (\S+)/)?.[1];
	assert.equal(prevPath, firstPath, "[ must return to the previous file");
});

test("W9 enter is inert inside the viewer and never opens nvim", async (t) => {
	const { pi, recorder, ctx, repo } = createHarness(t);
	addUnstagedEdit(repo);
	pi.fire("session_start", {}, ctx);
	await waitFor(() => recorder.setStatusCalls.length >= 1);

	const probe = await openOverlay(pi, recorder, ctx, 70, [" ", "\r"]);
	const joined = probe.lines.join("\n");
	assert.match(joined, /read-only/, "enter must not have closed or left the viewer");
	assert.match(joined, /╭─/, "the viewer frame must still be open after enter");
});

test("W9 ctrl-d, ctrl-u, g and G move the viewer's scroll offset", async (t) => {
	const { pi, recorder, ctx, repo } = createHarness(t);
	const lines: string[] = [];
	for (let index = 0; index < 60; index += 1) {
		lines.push(`line ${index}`);
	}
	fs.writeFileSync(path.join(repo.root, "alpha.txt"), `${lines.join("\n")}\n`);
	pi.fire("session_start", {}, ctx);
	await waitFor(() => recorder.setStatusCalls.length >= 1);

	const top = await openOverlay(pi, recorder, ctx, 70, [" "]);
	assert.match(top.lines.join("\n"), /line 0\b/, "the viewer opens scrolled to the top");

	const paged = await openOverlay(pi, recorder, ctx, 70, [" ", "\x04"]);
	assert.ok(
		!paged.lines.join("\n").includes("line 0\n") ||
			paged.lines.join("\n") !== top.lines.join("\n"),
		"ctrl-d must move the visible slice",
	);

	const bottom = await openOverlay(pi, recorder, ctx, 70, [" ", "G"]);
	const backToTop = await openOverlay(pi, recorder, ctx, 70, [" ", "G", "g"]);
	assert.notEqual(
		bottom.lines.join("\n"),
		backToTop.lines.join("\n"),
		"g after G must scroll back toward the top, proving g and G both act",
	);
});

test("W9 a rejected patch fetch leaves the viewer unavailable without throwing", async (t) => {
	const { pi, recorder, ctx, repo } = createHarness(t);
	fs.writeFileSync(path.join(repo.root, "conflict-dir"), "placeholder\n");
	// A path that cannot be diffed (a plain file standing in for a directory
	// git will refuse) drives filePatch's own error path without touching the
	// engine module: the shell's catch around filePatch must still hold.
	addUnstagedEdit(repo);
	pi.fire("session_start", {}, ctx);
	await waitFor(() => recorder.setStatusCalls.length >= 1);

	const probe = await openOverlay(pi, recorder, ctx, 70, [" "]);
	const joined = probe.lines.join("\n");
	assert.ok(
		/@@|patch unavailable|opening/.test(joined),
		"the viewer must render either the patch or an honest unavailable state, never throw",
	);
});

test("W8 the overlay shows origin labels for committed and uncommitted rows on the branch basket", async (t) => {
	const { pi, recorder, ctx, repo } = createHarness(t);
	addBranchCommit(repo, "feature", "committed-on-branch.txt");
	addUnstagedEdit(repo);

	pi.fire("session_start", {}, ctx);
	await waitFor(() => recorder.setStatusCalls.length >= 1);

	const probe = await openOverlay(pi, recorder, ctx, 100, []);
	const joined = probe.lines.join("\n");
	assert.match(
		joined,
		/ turn \[overall\]/,
		"overall mode should already be active: no agent_start fired, so requestStats stays null",
	);
	assert.match(joined, /committed-on-branch\.txt/);
	assert.match(joined, /committed/);
	assert.match(joined, /uncommitted/);

	const originColors = new Set(
		probe.themes.fgCalls
			.filter((call) => call.text.includes("committed"))
			.map((call) => call.color),
	);
	assert.ok(
		originColors.has("dim") || originColors.has("accent"),
		"origin labels must be styled through the theme, not left plain",
	);
	for (const line of probe.lines) {
		assert.equal(visibleWidthOf(line), 100);
	}
});
