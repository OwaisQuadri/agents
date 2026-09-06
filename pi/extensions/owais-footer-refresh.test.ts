import assert from "node:assert/strict";
import { mkdtemp, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { test, type TestContext } from "node:test";

const extensionDirectory = fileURLToPath(new URL(".", import.meta.url));
type Display = { dispose(): void; render(width: number): string[] };
type Result = { code: number; stdout: string };
type Handler = (...args: any[]) => unknown;

async function writeModule(root: string, name: string, source: string): Promise<void> {
	const directory = join(root, "node_modules", ...name.split("/"));
	await mkdir(directory, { recursive: true });
	await writeFile(join(directory, "package.json"), '{"type":"module","exports":"./index.js"}');
	await writeFile(join(directory, "index.js"), source);
}

async function settle(): Promise<void> {
	for (let index = 0; index < 6; index++) await new Promise((resolve) => setImmediate(resolve));
}

async function fixture(t: TestContext, isGit = false) {
	const root = await mkdtemp(join(tmpdir(), "owais-footer-refresh-"));
	t.after(() => rm(root, { recursive: true, force: true }));
	await writeFile(join(root, "owais-footer.ts"), await readFile(join(extensionDirectory, "owais-footer.ts")));
	await mkdir(join(root, "live-diff"), { recursive: true });
	await writeFile(join(root, "live-diff", "engine.ts"), await readFile(join(extensionDirectory, "live-diff", "engine.ts")));
	await writeModule(root, "@earendil-works/pi-tui", `
export const visibleWidth = (text) => text.replace(/\\x1b[^m]*m|\\x1b]8;;[^\\x1b]*\\x1b\\\\/g, "").length;
`);
	await writeModule(root, "@earendil-works/pi-coding-agent", "export const withFileMutationQueue = (_path, action) => action();");
	const extension = await import(pathToFileURL(join(root, "owais-footer.ts")).href);
	t.mock.timers.enable({ apis: ["setInterval", "setTimeout", "Date"], now: 1_000_000 });
	const handlers = new Map<string, Handler>();
	const events = new Map<string, Handler>();
	const displays: Display[] = [];
	const branchListeners = new Set<() => void>();
	const pending: Array<{ resolve(value: Result): void; reject(error: Error): void }> = [];
	const calls: string[] = [];
	let requests = 0;
	let branch = "feature-one";
	let footerFactory: Handler;
	let widgetFactory: Handler;
	let footer: Display;
	let widget: Display;
	const terminal = { requestRender() { requests++; } };
	const theme = { fg(_color: string, text: string) { return text; } };
	const footerData = {
		onBranchChange(callback: () => void) {
			branchListeners.add(callback);
			return () => branchListeners.delete(callback);
		},
	};
	const api = {
		on(name: string, handler: Handler) { handlers.set(name, handler); },
		events: {
			on(name: string, handler: Handler) {
				events.set(name, handler);
				return () => events.delete(name);
			},
		},
		async exec(command: string, args: string[]): Promise<Result> {
			const key = `${command} ${args.join(" ")}`;
			calls.push(key);
			if (key === "git rev-parse --show-toplevel") return { code: isGit ? 0 : 1, stdout: isGit ? root : "" };
			if (key === "git branch --show-current") return { code: 0, stdout: branch };
			if (key === "git remote -v") return { code: 0, stdout: "" };
			if (key === "git rev-parse --verify --quiet HEAD") return { code: 0, stdout: "head" };
			if (key === "git symbolic-ref --quiet refs/remotes/origin/HEAD") return { code: 0, stdout: "refs/remotes/origin/main" };
			if (key === "git rev-parse --verify --quiet origin/main^{commit}") return { code: 0, stdout: "base" };
			if (key === "git merge-base HEAD origin/main") return { code: 0, stdout: "base" };
			if (command === "git" && args[0] === "log") return { code: 0, stdout: "abc123 add footer refresh" };
			if (key === "git status --porcelain") return { code: 0, stdout: " M footer.ts" };
			if (key === "fm available --model system") return { code: 0, stdout: "System model available" };
			if (command === "fm" && args[0] === "respond") return new Promise((resolve, reject) => pending.push({ resolve, reject }));
			assert.fail(`unexpected command: ${key}`);
		},
	};
	const ctx = {
		mode: "tui",
		cwd: root,
		model: { provider: "test", id: "model", contextWindow: 1 },
		thinkingLevel: "off",
		getContextUsage() { return undefined; },
		sessionManager: { getEntries: () => [] },
		ui: {
			setWorkingVisible() {},
			setFooter(factory: Handler) {
				footer?.dispose();
				footerFactory = factory;
				footer = factory(terminal, theme, footerData);
				displays.push(footer);
			},
			setWidget(_key: string, factory: Handler) {
				widget?.dispose();
				widgetFactory = factory;
				widget = factory(terminal, theme);
				displays.push(widget);
			},
		},
	};
	extension.default(api);
	async function fire(name: string) {
		assert.ok(handlers.has(name), `registered ${name}`);
		await handlers.get(name)!({}, ctx);
		await settle();
	}
	t.after(async () => {
		await fire("session_shutdown");
		for (const display of displays) display.dispose();
		for (const response of pending) response.resolve({ code: 1, stdout: "" });
		await settle();
		t.mock.timers.reset();
	});
	return {
		ctx, calls, pending, fire,
		get requests() { return requests; },
		get footer() { return footer; },
		get widget() { return widget; },
		async event(name: string) {
			assert.ok(events.has(name), `registered ${name}`);
			await events.get(name)!({ id: "child-one" });
			await settle();
		},
		async changeBranch() {
			branch = "feature-two";
			for (const listener of branchListeners) listener();
			await settle();
		},
		replace(kind: "footer" | "widget", isDisposeFirst: boolean) {
			const old = kind === "footer" ? footer : widget;
			if (isDisposeFirst) old.dispose();
			const next = kind === "footer" ? footerFactory(terminal, theme, footerData) : widgetFactory(terminal, theme);
			displays.push(next);
			if (kind === "footer") footer = next;
			else widget = next;
			if (!isDisposeFirst) old.dispose();
		},
	};
}

function expectCadence(t: TestContext, state: { requests: number }, delay: number): void {
	for (let index = 0; index < 2; index++) {
		const before = state.requests;
		t.mock.timers.tick(delay - 1);
		assert.equal(state.requests - before, 0, `no periodic render before ${delay} ms`);
		t.mock.timers.tick(1);
		assert.equal(state.requests - before, 1, `one shared-terminal render every ${delay} ms`);
	}
}

test("idle requests one shared-terminal refresh every 1000 ms", async (t) => {
	const state = await fixture(t);
	await state.fire("session_start");
	const calls = state.calls.length;
	expectCadence(t, state, 1000);
	assert.equal(state.calls.length, calls, "render ticks do not execute subprocesses");
});

test("working requests one shared-terminal refresh every 120 ms", async (t) => {
	const state = await fixture(t);
	await state.fire("session_start");
	t.mock.timers.tick(50);
	await state.fire("agent_start");
	expectCadence(t, state, 120);
});

test("settling resets the deadline to the idle cadence", async (t) => {
	const state = await fixture(t);
	await state.fire("session_start");
	await state.fire("agent_start");
	t.mock.timers.tick(50);
	await state.fire("agent_settled");
	expectCadence(t, state, 1000);
});

test("activity, model, repository, and delegated changes refresh immediately", async (t) => {
	const state = await fixture(t);
	await state.fire("session_start");
	for (const name of ["agent_start", "model_select", "agent_settled"]) {
		const before = state.requests;
		await state.fire(name);
		assert.ok(state.requests > before, `${name} refreshes without advancing time`);
	}
	const before = state.requests;
	await state.changeBranch();
	assert.ok(state.requests > before, "repository result refreshes without advancing time");
	for (const name of ["subagents:started", "subagents:completed", "subagents:started", "subagents:failed"]) {
		const before = state.requests;
		await state.event(name);
		assert.ok(state.requests > before, `${name} refreshes without advancing time`);
	}
});

test("delegated activity keeps the idle cadence", async (t) => {
	const state = await fixture(t);
	await state.fire("session_start");
	await state.event("subagents:started");
	assert.match(state.widget.render(200).join("\n"), /Delegated/);
	expectCadence(t, state, 1000);
});

test("headline generation stays fast after the agent settles", async (t) => {
	const state = await fixture(t, true);
	await state.fire("session_start");
	assert.equal(state.pending.length, 1, "headline subprocess is pending");
	await state.fire("agent_start");
	await state.fire("agent_settled");
	assert.match(state.widget.render(200).join("\n"), /Generating headline/);
	expectCadence(t, state, 120);
});

for (const outcome of ["success", "failure", "rejection"] as const) {
	test(`headline ${outcome} refreshes immediately and restores idle cadence`, async (t) => {
		const state = await fixture(t, true);
		await state.fire("session_start");
		assert.equal(state.pending.length, 1);
		assert.match(state.widget.render(200).join("\n"), /Generating headline/);
		const before = state.requests;
		if (outcome === "rejection") state.pending[0].reject(new Error("headline unavailable"));
		else state.pending[0].resolve({ code: outcome === "success" ? 0 : 1, stdout: outcome === "success" ? "Footer refresh" : "" });
		await settle();
		assert.ok(state.requests > before, "headline completion refreshes without advancing time");
		assert.doesNotMatch(state.widget.render(200).join("\n"), /Generating headline/);
		if (outcome === "success") assert.match(state.widget.render(200).join("\n"), /Footer refresh/);
		expectCadence(t, state, 1000);
	});
}

for (const isStaleFirst of [true, false]) {
	test(`superseded headline completing ${isStaleFirst ? "first" : "last"} leaves the current generation's cadence`, async (t) => {
		const state = await fixture(t, true);
		await state.fire("session_start");
		await state.changeBranch();
		assert.equal(state.pending.length, 2);
		if (isStaleFirst) {
			state.pending[0].resolve({ code: 0, stdout: "Stale headline" });
			await settle();
			assert.match(state.widget.render(200).join("\n"), /Generating headline/);
			expectCadence(t, state, 120);
		}
		state.pending[1].resolve({ code: 0, stdout: "Current headline" });
		await settle();
		assert.match(state.widget.render(200).join("\n"), /Current headline/);
		expectCadence(t, state, 1000);
		if (!isStaleFirst) {
			state.pending[0].resolve({ code: 0, stdout: "Stale headline" });
			await settle();
			assert.match(state.widget.render(200).join("\n"), /Current headline/);
			assert.doesNotMatch(state.widget.render(200).join("\n"), /Stale headline/);
			expectCadence(t, state, 1000);
		}
	});
}

test("repeated session start does not accumulate periodic requests", async (t) => {
	const state = await fixture(t);
	await state.fire("session_start");
	await state.fire("session_start");
	await state.fire("session_start");
	expectCadence(t, state, 1000);
});

for (const kind of ["footer", "widget"] as const) {
	for (const isDisposeFirst of [true, false]) {
		test(`${kind} replacement with disposal ${isDisposeFirst ? "before" : "after"} creation keeps its refresh callback`, async (t) => {
			const state = await fixture(t);
			await state.fire("session_start");
			state.replace(kind, isDisposeFirst);
			if (kind === "footer") state.widget.dispose();
			else state.footer.dispose();
			const before = state.requests;
			await state.fire("model_select");
			assert.ok(state.requests > before, "remaining display receives immediate updates");
			expectCadence(t, state, 1000);
		});
	}
	test(`disposing ${kind} first keeps the other display alive and disposing both stops refreshes`, async (t) => {
		const state = await fixture(t);
		await state.fire("session_start");
		state[kind].dispose();
		expectCadence(t, state, 1000);
		state[kind === "footer" ? "widget" : "footer"].dispose();
		const before = state.requests;
		t.mock.timers.tick(2000);
		assert.equal(state.requests, before, "no refresh after both displays are disposed");
	});
}

test("disposing both displays stops periodic refreshes without shutdown", async (t) => {
	const state = await fixture(t);
	await state.fire("session_start");
	state.footer.dispose();
	state.widget.dispose();
	const before = state.requests;
	t.mock.timers.tick(2000);
	assert.equal(state.requests, before);
});

test("shutdown stops periodic refreshes before display disposal", async (t) => {
	const state = await fixture(t);
	await state.fire("session_start");
	await state.fire("session_shutdown");
	const before = state.requests;
	t.mock.timers.tick(2000);
	assert.equal(state.requests, before, "shutdown cancels display refresh scheduling");
});

test("late headline completion cannot restart refreshes after shutdown", async (t) => {
	const state = await fixture(t, true);
	await state.fire("session_start");
	assert.equal(state.pending.length, 1);
	await state.fire("session_shutdown");
	const before = state.requests;
	state.pending[0].resolve({ code: 0, stdout: "Late headline" });
	await settle();
	assert.equal(state.requests, before, "late completion does not request an immediate render");
	t.mock.timers.tick(2000);
	assert.equal(state.requests, before, "late completion does not restart a periodic render");
});

test("noninteractive start creates no display or periodic refresh", async (t) => {
	const state = await fixture(t);
	state.ctx.mode = "rpc";
	await state.fire("session_start");
	assert.equal(state.footer, undefined);
	assert.equal(state.widget, undefined);
	t.mock.timers.tick(30_000);
	await settle();
	assert.equal(state.requests, 0);
	assert.equal(state.calls.length, 0);
});
