import { test } from "node:test";
import assert from "node:assert/strict";

import { HerdrCommandClient, type HerdrCommandResult, type HerdrTransport } from "./client.ts";
import {
	makeMalformedWorkspaceUpdatedEvent,
	makePaneUpdatedEvent,
	makeSnapshotResponse,
	makeSnapshotResponseMissingId,
	makeSnapshotResponseMissingSnapshot,
	makeSnapshotResponseWrongType,
	makeUnknownEvent,
	makeWorkspaceUpdatedEvent,
	SELF_PANE_ID,
	SELF_WORKSPACE_ID,
} from "./fixtures.ts";

interface RecordedCall {
	args: string[];
	signal?: AbortSignal;
}

function makeFakeTransport(options: {
	snapshotResult?: (signal?: AbortSignal) => Promise<HerdrCommandResult>;
	paneReadResult?: (signal?: AbortSignal) => Promise<HerdrCommandResult>;
	eventLines?: (signal?: AbortSignal) => AsyncIterable<string>;
}): { transport: HerdrTransport; calls: RecordedCall[] } {
	const calls: RecordedCall[] = [];
	const transport: HerdrTransport = {
		runCommand: async (args: string[], signal?: AbortSignal) => {
			calls.push(signal === undefined ? { args } : { args, signal });
			if (args[0] === "api" && args[1] === "snapshot") {
				if (options.snapshotResult === undefined) {
					throw new Error("fake transport: no snapshot result configured");
				}
				return options.snapshotResult(signal);
			}
			if (args[0] === "pane" && args[1] === "read") {
				if (options.paneReadResult === undefined) {
					throw new Error("fake transport: no pane read result configured");
				}
				return options.paneReadResult(signal);
			}
			throw new Error(`fake transport: unexpected command ${JSON.stringify(args)}`);
		},
		subscribeEvents: (signal?: AbortSignal) => {
			if (options.eventLines === undefined) {
				throw new Error("fake transport: no event lines configured");
			}
			return options.eventLines(signal);
		},
	};
	return { transport, calls };
}

async function collectEvents(
	client: HerdrCommandClient,
	limit: number,
	signal?: AbortSignal,
): Promise<unknown[]> {
	const collected: unknown[] = [];
	for await (const event of client.events(signal)) {
		collected.push(event);
		if (collected.length >= limit) {
			break;
		}
	}
	return collected;
}

async function* linesFrom(lines: string[]): AsyncIterable<string> {
	for (const line of lines) {
		yield line;
	}
}

function acknowledgement(overrides: Record<string, unknown> = {}): string {
	return JSON.stringify({
		id: "pi-herdr-state-events",
		result: { type: "subscription_started" },
		...overrides,
	});
}

function eventEnvelope(data: unknown, event?: string): string {
	const dataType =
		data !== null && typeof data === "object"
			? (data as { type?: unknown }).type
			: undefined;
	return JSON.stringify({ event: event ?? dataType, data });
}

function subscriptionLines(...events: string[]): AsyncIterable<string> {
	return linesFrom([acknowledgement(), ...events]);
}

test("TC-01 snapshot returns the raw envelope on success", async () => {
	const response = makeSnapshotResponse();
	const { transport, calls } = makeFakeTransport({
		snapshotResult: async () => ({ code: 0, stdout: JSON.stringify(response), stderr: "" }),
	});
	const client = new HerdrCommandClient(transport);

	const result = await client.snapshot();

	assert.deepEqual(result, response);
	assert.deepEqual(calls, [{ args: ["api", "snapshot"] }]);
});

test("TC-06 snapshot reports unavailable when the command fails to run", async () => {
	const { transport } = makeFakeTransport({
		snapshotResult: async () => {
			throw new Error("spawn ENOENT");
		},
	});
	const client = new HerdrCommandClient(transport);

	const result = await client.snapshot();

	assert.deepEqual(result, {
		code: "unavailable",
		message: "Herdr snapshot command failed to run: spawn ENOENT",
	});
});

test("TC-06 snapshot reports unavailable when the command exits nonzero", async () => {
	const { transport } = makeFakeTransport({
		snapshotResult: async () => ({ code: 1, stdout: "", stderr: "herdr: no running session" }),
	});
	const client = new HerdrCommandClient(transport);

	const result = await client.snapshot();

	assert.equal((result as { code: string }).code, "unavailable");
	assert.match((result as { message: string }).message, /no running session/);
});

test("TC-06 snapshot reports invalid-response for output that is not JSON", async () => {
	const { transport } = makeFakeTransport({
		snapshotResult: async () => ({ code: 0, stdout: "not json", stderr: "" }),
	});
	const client = new HerdrCommandClient(transport);

	const result = await client.snapshot();

	assert.equal((result as { code: string }).code, "invalid-response");
});

test("TC-06 snapshot reports invalid-response for each malformed envelope shape", async () => {
	for (const malformed of [
		makeSnapshotResponseMissingId(),
		makeSnapshotResponseMissingSnapshot(),
		makeSnapshotResponseWrongType(),
	]) {
		const { transport } = makeFakeTransport({
			snapshotResult: async () => ({ code: 0, stdout: JSON.stringify(malformed), stderr: "" }),
		});
		const client = new HerdrCommandClient(transport);

		const result = await client.snapshot();

		assert.equal(
			(result as { code: string }).code,
			"invalid-response",
			`expected invalid-response for ${JSON.stringify(malformed)}`,
		);
	}
});

test("TC-03 readPane returns bounded, unmarked output when the source fits the limit", async () => {
	const { transport, calls } = makeFakeTransport({
		paneReadResult: async () => ({ code: 0, stdout: "line1\nline2\nline3\n", stderr: "" }),
	});
	const client = new HerdrCommandClient(transport);

	const result = await client.readPane(SELF_PANE_ID, 5);

	assert.deepEqual(result, { paneId: SELF_PANE_ID, text: "line1\nline2\nline3", isTruncated: false });
	assert.deepEqual(calls, [
		{ args: ["pane", "read", SELF_PANE_ID, "--source", "recent", "--lines", "6"] },
	]);
});

test("TC-03 readPane bounds output and reports truncation when the source exceeds the limit", async () => {
	const lines = Array.from({ length: 6 }, (_value, index) => `line${index + 1}`);
	const { transport } = makeFakeTransport({
		paneReadResult: async () => ({ code: 0, stdout: lines.join("\n") + "\n", stderr: "" }),
	});
	const client = new HerdrCommandClient(transport);

	const result = await client.readPane(SELF_PANE_ID, 5);

	assert.deepEqual(result, {
		paneId: SELF_PANE_ID,
		text: "line2\nline3\nline4\nline5\nline6",
		isTruncated: true,
	});
});

test("TC-08 readPane reports not-found for an absent pane", async () => {
	const { transport } = makeFakeTransport({
		paneReadResult: async () => ({
			code: 1,
			stdout: "",
			stderr: JSON.stringify({
				error: { code: "pane_not_found", message: "pane bogus-pane-id not found" },
				id: "cli:pane:read",
			}),
		}),
	});
	const client = new HerdrCommandClient(transport);

	const result = await client.readPane("bogus-pane-id", 5);

	assert.deepEqual(result, {
		code: "not-found",
		message: "Herdr pane bogus-pane-id was not found: pane bogus-pane-id not found",
	});
});

test("readPane reports unavailable for a nonzero exit that is not a not-found error", async () => {
	const { transport } = makeFakeTransport({
		paneReadResult: async () => ({ code: 1, stdout: "", stderr: "herdr: no running session" }),
	});
	const client = new HerdrCommandClient(transport);

	const result = await client.readPane(SELF_PANE_ID, 5);

	assert.equal((result as { code: string }).code, "unavailable");
});

test("readPane reports unavailable when the command fails to run", async () => {
	const { transport } = makeFakeTransport({
		paneReadResult: async () => {
			throw new Error("spawn ENOENT");
		},
	});
	const client = new HerdrCommandClient(transport);

	const result = await client.readPane(SELF_PANE_ID, 5);

	assert.equal((result as { code: string }).code, "unavailable");
});

test("TC-22 readPane accepts the direct-client line-limit boundaries", async () => {
	const { transport, calls } = makeFakeTransport({
		paneReadResult: async () => ({ code: 0, stdout: "", stderr: "" }),
	});
	const client = new HerdrCommandClient(transport);

	assert.deepEqual(await client.readPane(SELF_PANE_ID, 1), {
		paneId: SELF_PANE_ID,
		text: "",
		isTruncated: false,
	});
	assert.deepEqual(await client.readPane(SELF_PANE_ID, 10_000), {
		paneId: SELF_PANE_ID,
		text: "",
		isTruncated: false,
	});
	assert.deepEqual(calls, [
		{ args: ["pane", "read", SELF_PANE_ID, "--source", "recent", "--lines", "2"] },
		{ args: ["pane", "read", SELF_PANE_ID, "--source", "recent", "--lines", "10001"] },
	]);
});

test("TC-22 readPane rejects out-of-range direct-client line limits before transport", async (t) => {
	for (const lineLimit of [0, 10_001, Number.MAX_SAFE_INTEGER, Number.MAX_VALUE]) {
		await t.test(String(lineLimit), async () => {
			const { transport, calls } = makeFakeTransport({});
			const client = new HerdrCommandClient(transport);

			const result = await client.readPane(SELF_PANE_ID, lineLimit);

			assert.equal((result as { code: string }).code, "invalid-response");
			assert.deepEqual(calls, [], "an invalid request must never reach the transport");
		});
	}
});

test("TC-23 readPane rejects empty, control-bearing, and option-shaped pane identifiers before transport", async (t) => {
	const rejectedPaneIds = [
		"pane-ok\n--lines\n999999",
		"",
		"-h",
		"--help",
		"--lines",
		"p\u0000",
		"p\u001b",
		"p\u007f",
		"p\u0085",
	];

	for (const paneId of rejectedPaneIds) {
		await t.test(JSON.stringify(paneId), async () => {
			const { transport, calls } = makeFakeTransport({});
			const client = new HerdrCommandClient(transport);

			const result = await client.readPane(paneId, 1);

			assert.equal((result as { code: string }).code, "invalid-response");
			assert.deepEqual(calls, [], "an invalid pane identifier must never reach the transport");
		});
	}
});

test("TC-23 readPane passes a printable Unicode pane identifier as one exact argument", async () => {
	const { transport, calls } = makeFakeTransport({
		paneReadResult: async () => ({ code: 0, stdout: "", stderr: "" }),
	});
	const client = new HerdrCommandClient(transport);

	const result = await client.readPane("pane-雪", 1);

	assert.deepEqual(result, { paneId: "pane-雪", text: "", isTruncated: false });
	assert.deepEqual(calls, [
		{ args: ["pane", "read", "pane-雪", "--source", "recent", "--lines", "2"] },
	]);
});

test("readPane reports invalid-response for a malformed line limit", async () => {
	const { transport, calls } = makeFakeTransport({});
	const client = new HerdrCommandClient(transport);

	assert.equal((await client.readPane(SELF_PANE_ID, -1) as { code: string }).code, "invalid-response");
	assert.equal((await client.readPane(SELF_PANE_ID, 1.5) as { code: string }).code, "invalid-response");
	assert.equal((await client.readPane(SELF_PANE_ID, Number.MIN_VALUE) as { code: string }).code, "invalid-response");
	assert.deepEqual(calls, [], "an invalid request must never reach the transport");
});

test("TC-05 events requests snapshot replacement for an unknown event and continues", async () => {
	const { transport } = makeFakeTransport({
		eventLines: () =>
			subscriptionLines(
				eventEnvelope(makeUnknownEvent()),
				eventEnvelope(makePaneUpdatedEvent()),
			),
	});
	const client = new HerdrCommandClient(transport);

	const events = await collectEvents(client, 2);

	assert.equal((events[0] as { code: string }).code, "invalid-response");
	assert.match((events[0] as { message: string }).message, /replace the snapshot/);
	assert.equal((events[1] as { type: string }).type, "pane-changed");
});

test("events reports invalid-response for a line that is not JSON and continues", async () => {
	const { transport } = makeFakeTransport({
		eventLines: () =>
			subscriptionLines("not json", eventEnvelope(makeWorkspaceUpdatedEvent())),
	});
	const client = new HerdrCommandClient(transport);

	const events = await collectEvents(client, 2);

	assert.equal((events[0] as { code: string }).code, "invalid-response");
	assert.match((events[0] as { message: string }).message, /replace the snapshot/);
	assert.equal((events[1] as { type: string }).type, "workspace-changed");
});

test("events reports invalid-response for a line that does not decode to an object and continues", async () => {
	const { transport } = makeFakeTransport({
		eventLines: () => subscriptionLines("42", eventEnvelope(makeWorkspaceUpdatedEvent())),
	});
	const client = new HerdrCommandClient(transport);

	const events = await collectEvents(client, 2);

	assert.equal((events[0] as { code: string }).code, "invalid-response");
	assert.match((events[0] as { message: string }).message, /replace the snapshot/);
	assert.equal((events[1] as { type: string }).type, "workspace-changed");
});

test("events reports invalid-response for a malformed recognized event and continues", async () => {
	const { transport } = makeFakeTransport({
		eventLines: () =>
			subscriptionLines(
				eventEnvelope(makeMalformedWorkspaceUpdatedEvent()),
				eventEnvelope(makePaneUpdatedEvent()),
			),
	});
	const client = new HerdrCommandClient(transport);

	const events = await collectEvents(client, 2);

	assert.equal((events[0] as { code: string }).code, "invalid-response");
	assert.match((events[0] as { message: string }).message, /replace the snapshot/);
	assert.equal((events[1] as { type: string }).type, "pane-changed");
});

test("TC-18 snapshot forwards cancellation and resolves unavailable with the abort reason", async () => {
	const controller = new AbortController();
	const reason = new Error("required abort reason");
	let receivedSignal: AbortSignal | undefined;
	const { transport, calls } = makeFakeTransport({
		snapshotResult: async (signal) => {
			receivedSignal = signal;
			return new Promise<never>((_resolve, reject) => {
				signal?.addEventListener("abort", () => reject(new Error("transport AbortError")), {
					once: true,
				});
			});
		},
	});
	const client = new HerdrCommandClient(transport);

	const pending = client.snapshot(controller.signal);
	controller.abort(reason);

	const result = await pending;

	assert.equal((result as { code: string }).code, "unavailable");
	assert.match((result as { message: string }).message, /required abort reason/);
	assert.equal(receivedSignal, controller.signal);
	assert.equal(calls[0]?.signal, controller.signal);
});

test("TC-20 events requires the exact acknowledgement and ends after recovery guidance", async (t) => {
	const invalidAcknowledgements = [
		"not json",
		JSON.stringify({ id: "wrong-id", result: { type: "subscription_started" } }),
		JSON.stringify({ id: "pi-herdr-state-events", result: { type: "wrong_type" } }),
		JSON.stringify({
			id: "pi-herdr-state-events",
			result: { type: "subscription_started", extra: true },
		}),
	];

	for (const invalidAcknowledgement of invalidAcknowledgements) {
		await t.test(invalidAcknowledgement, async () => {
			const { transport } = makeFakeTransport({
				eventLines: () =>
					linesFrom([
						invalidAcknowledgement,
						eventEnvelope(makeWorkspaceUpdatedEvent()),
					]),
			});
			const client = new HerdrCommandClient(transport);

			const events: unknown[] = [];
			for await (const event of client.events()) {
				events.push(event);
			}

			assert.equal(events.length, 1);
			assert.equal((events[0] as { code: string }).code, "invalid-response");
			assert.match((events[0] as { message: string }).message, /replace the snapshot/);
		});
	}
});

test("TC-20 events validates schema-complete envelopes and continues after invalid data", async () => {
	const { transport } = makeFakeTransport({
		eventLines: () =>
			subscriptionLines(
				eventEnvelope(makeWorkspaceUpdatedEvent(), "pane_updated"),
				JSON.stringify({
					event: "pane.output_matched",
					data: { pane_id: SELF_PANE_ID, matched_line: "done", read: {} },
				}),
				JSON.stringify({ event: "workspace_updated", data: null }),
				eventEnvelope(makeUnknownEvent()),
				eventEnvelope(makeWorkspaceUpdatedEvent()),
			),
	});
	const client = new HerdrCommandClient(transport);

	const events = await collectEvents(client, 5);

	for (const failure of events.slice(0, 4)) {
		assert.equal((failure as { code: string }).code, "invalid-response");
		assert.match((failure as { message: string }).message, /replace the snapshot/);
	}
	assert.equal((events[4] as { type: string }).type, "workspace-changed");
});

test("TC-21 event cancellation forwards the signal and ends without unavailable", async () => {
	const controller = new AbortController();
	let receivedSignal: AbortSignal | undefined;
	async function* waitingLines(signal?: AbortSignal): AsyncIterable<string> {
		receivedSignal = signal;
		yield acknowledgement();
		await new Promise<void>((resolve) => {
			signal?.addEventListener("abort", () => resolve(), { once: true });
		});
	}
	const { transport } = makeFakeTransport({ eventLines: waitingLines });
	const client = new HerdrCommandClient(transport);

	const events: unknown[] = [];
	const collecting = (async () => {
		for await (const event of client.events(controller.signal)) {
			events.push(event);
		}
	})();
	await new Promise<void>((resolve) => setImmediate(resolve));
	controller.abort();
	await collecting;

	assert.equal(receivedSignal, controller.signal);
	assert.deepEqual(events, []);
});

test("events reports unavailable when the subscription fails to start", async () => {
	const { transport } = makeFakeTransport({
		eventLines: () => {
			throw new Error("connection refused");
		},
	});
	const client = new HerdrCommandClient(transport);

	const events = await collectEvents(client, 1);

	assert.deepEqual(events, [
		{ code: "unavailable", message: "Herdr event subscription failed to start: connection refused" },
	]);
});

test("events reports unavailable and ends the stream when the connection drops mid-subscription", async () => {
	async function* droppingLines(): AsyncIterable<string> {
		yield acknowledgement();
		yield eventEnvelope(makeWorkspaceUpdatedEvent());
		throw new Error("connection reset");
	}
	const { transport } = makeFakeTransport({ eventLines: droppingLines });
	const client = new HerdrCommandClient(transport);

	const events: unknown[] = [];
	for await (const event of client.events()) {
		events.push(event);
	}

	assert.equal(events.length, 2);
	assert.equal((events[0] as { type: string }).type, "workspace-changed");
	assert.deepEqual(events[1], {
		code: "unavailable",
		message: "Herdr event subscription ended: connection reset",
	});
});

test("TC-09 the client never issues a write or input command", async () => {
	const response = makeSnapshotResponse();
	const { transport, calls } = makeFakeTransport({
		snapshotResult: async () => ({ code: 0, stdout: JSON.stringify(response), stderr: "" }),
		paneReadResult: async () => ({ code: 0, stdout: "hello\n", stderr: "" }),
		eventLines: () => subscriptionLines(eventEnvelope(makePaneUpdatedEvent())),
	});
	const client = new HerdrCommandClient(transport);

	await client.snapshot();
	await client.readPane(SELF_PANE_ID, 5);
	await collectEvents(client, 1);

	const WRITE_VERBS = [
		"input",
		"send-text",
		"send-keys",
		"run",
		"close",
		"rename",
		"focus",
		"split",
		"swap",
		"move",
		"resize",
		"zoom",
	];
	for (const call of calls) {
		assert.equal(
			WRITE_VERBS.includes(call.args[1] ?? ""),
			false,
			`command ${JSON.stringify(call.args)} must never use a write or input verb`,
		);
	}
	assert.deepEqual(
		calls.map((call) => call.args.slice(0, 2)),
		[
			["api", "snapshot"],
			["pane", "read"],
		],
	);
});
