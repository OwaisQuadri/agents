import assert from "node:assert/strict";
import { test } from "node:test";

import eventTimestamps from "./event-timestamps.ts";

test("records timestamps for messages, tool calls, and tool results", () => {
	const handlers = new Map<string, (event: any) => void>();
	const entries: Array<{ type: string; data: { at: string; label: string } }> = [];
	const api = {
		appendEntry(type: string, data: { at: string; label: string }) {
			entries.push({ type, data });
		},
		on(event: string, handler: (event: any) => void) {
			handlers.set(event, handler);
		},
		registerEntryRenderer() {},
	};

	eventTimestamps(api as any);
	handlers.get("message_start")?.({ message: { role: "user", timestamp: Date.UTC(2026, 8, 5, 12, 30, 0) } });
	handlers.get("tool_call")?.({ toolName: "read" });
	handlers.get("message_start")?.({
		message: { role: "toolResult", toolName: "read", timestamp: Date.UTC(2026, 8, 5, 12, 30, 1) },
	});

	assert.equal(entries.length, 3);
	assert.deepEqual(entries.map(({ type }) => type), ["event-timestamp", "event-timestamp", "event-timestamp"]);
	assert.deepEqual(entries.map(({ data }) => data.label), ["user message", "tool call · read", "tool result · read"]);
	assert.equal(entries[0]?.data.at, "2026-09-05T12:30:00.000Z");
	assert.match(entries[1]?.data.at ?? "", /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$/);
	assert.equal(entries[2]?.data.at, "2026-09-05T12:30:01.000Z");
});

test("renders the timestamp without changing message content", () => {
	let renderer: ((entry: any, options: any, theme: any) => { render(width: number): string[] }) | undefined;
	const api = {
		appendEntry() {},
		on() {},
		registerEntryRenderer(_type: string, nextRenderer: typeof renderer) {
			renderer = nextRenderer;
		},
	};

	eventTimestamps(api as any);
	const component = renderer?.(
		{ data: { at: "2026-09-05T12:30:00.000Z", label: "assistant message" } },
		{},
		{ fg: (_style: string, text: string) => text },
	);

	assert.deepEqual(component?.render(120), ["2026-09-05T12:30:00.000Z · assistant message"]);
});
