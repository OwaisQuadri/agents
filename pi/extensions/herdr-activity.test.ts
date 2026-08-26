import assert from "node:assert/strict";
import { test } from "node:test";

import herdrActivity from "./herdr-activity.ts";
import { resetHerdrActivityState, setHerdrBlocked } from "./herdr-activity/state.ts";

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
