import assert from "node:assert/strict";
import { test } from "node:test";

import herdrActivity from "./herdr-activity.ts";

function loadActivity() {
	const lifecycleHandlers = new Map<string, (event: unknown, context: unknown) => void>();
	const eventHandlers = new Map<string, (payload: unknown) => void>();
	const emitted: Array<{ channel: string; payload: unknown }> = [];
	const api = {
		on(event: string, handler: (event: unknown, context: unknown) => void) {
			lifecycleHandlers.set(event, handler);
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
	herdrActivity(api as any);
	return { emitted, eventHandlers, lifecycleHandlers };
}

test("reports aggregate background activity through the Herdr event boundary", () => {
	const activity = loadActivity();
	const started = activity.eventHandlers.get("subagents:started");
	const completed = activity.eventHandlers.get("subagents:completed");
	const failed = activity.eventHandlers.get("subagents:failed");

	started?.(null);
	started?.(42);
	started?.({});
	started?.({ id: "" });
	started?.({ id: "   " });
	assert.deepEqual(activity.emitted, []);

	started?.({ id: "child-1" });
	started?.({ id: "child-1" });
	started?.({ id: "child-2" });
	completed?.({ id: "child-1" });
	completed?.({ id: "missing" });
	assert.deepEqual(activity.emitted, [{ channel: "herdr:busy", payload: { active: true } }]);

	failed?.({ id: "child-2" });
	failed?.({ id: "child-2" });
	assert.deepEqual(activity.emitted, [
		{ channel: "herdr:busy", payload: { active: true } },
		{ channel: "herdr:busy", payload: { active: false } },
	]);
});

test("session boundaries clear active background work", () => {
	const activity = loadActivity();
	activity.eventHandlers.get("subagents:started")?.({ id: "child-1" });
	activity.lifecycleHandlers.get("session_start")?.({}, {});
	activity.eventHandlers.get("subagents:started")?.({ id: "child-2" });
	activity.lifecycleHandlers.get("session_shutdown")?.({}, {});

	assert.deepEqual(activity.emitted, [
		{ channel: "herdr:busy", payload: { active: true } },
		{ channel: "herdr:busy", payload: { active: false } },
		{ channel: "herdr:busy", payload: { active: true } },
		{ channel: "herdr:busy", payload: { active: false } },
	]);
});
