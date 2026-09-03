import assert from "node:assert/strict";
import { test } from "node:test";

import herdrActivity from "./herdr-activity.ts";
import { resetHerdrActivityState, settleHerdrAgent, startHerdrAgent, setHerdrBlocked } from "./herdr-activity/state.ts";

test("reports working, needs-attention, done, and idle states in one sequence", async () => {
	const previous = {
		env: process.env.HERDR_ENV,
		paneId: process.env.HERDR_PANE_ID,
		socket: process.env.HERDR_SOCKET_PATH,
		binary: process.env.HERDR_BIN_PATH,
	};
	process.env.HERDR_ENV = "1";
	process.env.HERDR_PANE_ID = "pane-1";
	process.env.HERDR_SOCKET_PATH = "/tmp/herdr.sock";
	process.env.HERDR_BIN_PATH = "herdr-test";

	const handlers = new Map<string, (event: unknown, ctx: any) => Promise<void> | void>();
	const calls: Array<{ command: string; args: string[] }> = [];
	const api = {
		on(event: string, handler: (event: unknown, ctx: any) => Promise<void> | void) {
			handlers.set(event, handler);
		},
		events: {
			on() {
				return () => {};
			},
			emit() {},
		},
		async exec(command: string, args: string[]) {
			calls.push({ command, args });
			return { code: 0, stdout: "", stderr: "" };
		},
	};
	const ctx = {
		mode: "tui",
		sessionManager: { getSessionId: () => "session-1" },
	};

	try {
		resetHerdrActivityState();
		herdrActivity(api as any);
		await handlers.get("session_start")?.({}, ctx);
		await handlers.get("agent_start")?.({}, ctx);
		await setHerdrBlocked(api as any, ctx, true);
		await setHerdrBlocked(api as any, ctx, false);
		await handlers.get("agent_settled")?.({}, ctx);

		assert.equal(calls.length, 5);
		assert.deepEqual(calls.map(({ command }) => command), Array(5).fill("herdr-test"));
		assert.deepEqual(calls.map(({ args }) => args[8]), ["idle", "working", "blocked", "working", "idle"]);
		assert.deepEqual(calls.map(({ args }) => args[10]), ["Idle", "Working", "Needs attention", "Working", "Done"]);
		assert.ok(calls.every(({ args }) => args.includes("session-1")));
		assert.deepEqual(calls.map(({ args }) => Number(args[12])).sort((a, b) => a - b), calls.map(({ args }) => Number(args[12])));
	} finally {
		resetHerdrActivityState();
		process.env.HERDR_ENV = previous.env;
		process.env.HERDR_PANE_ID = previous.paneId;
		process.env.HERDR_SOCKET_PATH = previous.socket;
		process.env.HERDR_BIN_PATH = previous.binary;
	}
});

test("reports aggregate background activity through the Herdr event boundary", () => {
	const eventHandlers = new Map<string, (payload: unknown) => void>();
	const emitted: Array<{ channel: string; payload: unknown }> = [];
	const api = {
		on() {},
		events: {
			on(channel: string, handler: (payload: unknown) => void) {
				eventHandlers.set(channel, handler);
				return () => {};
			},
			emit(channel: string, payload: unknown) {
				emitted.push({ channel, payload });
			},
		},
	};

	herdrActivity(api as any);
	const started = eventHandlers.get("subagents:started");
	const completed = eventHandlers.get("subagents:completed");
	const failed = eventHandlers.get("subagents:failed");

	started?.(null);
	started?.(42);
	started?.({});
	started?.({ id: "" });
	started?.({ id: "   " });
	assert.deepEqual(emitted, []);

	started?.({ id: "child-1" });
	started?.({ id: "child-1" });
	started?.({ id: "child-2" });
	completed?.({ id: "child-1" });
	completed?.({ id: "missing" });
	assert.deepEqual(emitted, [{ channel: "herdr:busy", payload: { active: true } }]);

	failed?.({ id: "child-2" });
	failed?.({ id: "child-2" });
	assert.deepEqual(emitted, [
		{ channel: "herdr:busy", payload: { active: true } },
		{ channel: "herdr:busy", payload: { active: false } },
	]);

	started?.({ id: "child-3" });
	completed?.({ id: "child-3" });
	assert.deepEqual(emitted.slice(-2), [
		{ channel: "herdr:busy", payload: { active: true } },
		{ channel: "herdr:busy", payload: { active: false } },
	]);
});

test("session boundaries clear active background work", async () => {
	const handlers = new Map<string, (event: unknown, ctx: any) => Promise<void> | void>();
	const eventHandlers = new Map<string, (payload: unknown) => void>();
	const emitted: Array<{ channel: string; payload: unknown }> = [];
	const api = {
		on(event: string, handler: (event: unknown, ctx: any) => Promise<void> | void) {
			handlers.set(event, handler);
		},
		events: {
			on(channel: string, handler: (payload: unknown) => void) {
				eventHandlers.set(channel, handler);
				return () => {};
			},
			emit(channel: string, payload: unknown) {
				emitted.push({ channel, payload });
			},
		},
	};
	const context = {
		mode: "rpc",
		sessionManager: { getSessionId: () => "session-1" },
	};

	herdrActivity(api as any);
	eventHandlers.get("subagents:started")?.({ id: "child-1" });
	await handlers.get("session_start")?.({}, context);
	eventHandlers.get("subagents:started")?.({ id: "child-2" });
	await handlers.get("session_shutdown")?.({}, context);

	assert.deepEqual(emitted, [
		{ channel: "herdr:busy", payload: { active: true } },
		{ channel: "herdr:busy", payload: { active: false } },
		{ channel: "herdr:busy", payload: { active: true } },
		{ channel: "herdr:busy", payload: { active: false } },
	]);
});

test("one settle after several starts still reports idle, not a leaked working state", async () => {
	// agent_settled fires once per run after every retry/compaction/queued continuation
	// finishes, but agent_start can fire once per retry/continuation within that same
	// run. Two starts and one settle must still land on idle, or the reported state
	// gets pinned on "working" for the rest of the process's life.
	const previous = {
		env: process.env.HERDR_ENV,
		paneId: process.env.HERDR_PANE_ID,
		socket: process.env.HERDR_SOCKET_PATH,
		binary: process.env.HERDR_BIN_PATH,
	};
	process.env.HERDR_ENV = "1";
	process.env.HERDR_PANE_ID = "pane-1";
	process.env.HERDR_SOCKET_PATH = "/tmp/herdr.sock";
	process.env.HERDR_BIN_PATH = "herdr-test";

	const calls: Array<{ args: string[] }> = [];
	const api = {
		on() {},
		async exec(_command: string, args: string[]) {
			calls.push({ args });
			return { code: 0, stdout: "", stderr: "" };
		},
	};
	const ctx = {
		mode: "tui",
		sessionManager: { getSessionId: () => "session-1" },
	};

	try {
		resetHerdrActivityState();
		await startHerdrAgent(api as any, ctx as any);
		await startHerdrAgent(api as any, ctx as any);
		await settleHerdrAgent(api as any, ctx as any);

		assert.deepEqual(calls.map(({ args }) => args[8]), ["working", "working", "idle"]);
	} finally {
		resetHerdrActivityState();
		process.env.HERDR_ENV = previous.env;
		process.env.HERDR_PANE_ID = previous.paneId;
		process.env.HERDR_SOCKET_PATH = previous.socket;
		process.env.HERDR_BIN_PATH = previous.binary;
	}
});
