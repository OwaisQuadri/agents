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
}

/**
 * Builds a fake `HerdrTransport` whose `runCommand` dispatches on the
 * command's first two arguments and whose `subscribeEvents` replays a fixed
 * list of raw JSON lines, matching the shape a real spawned `herdr` command
 * or socket connection would present to `HerdrCommandClient`.
 */
function makeFakeTransport(options: {
	snapshotResult?: () => Promise<HerdrCommandResult>;
	paneReadResult?: () => Promise<HerdrCommandResult>;
	eventLines?: () => AsyncIterable<string>;
}): { transport: HerdrTransport; calls: RecordedCall[] } {
	const calls: RecordedCall[] = [];
	const transport: HerdrTransport = {
		runCommand: async (args: string[]) => {
			calls.push({ args });
			if (args[0] === "api" && args[1] === "snapshot") {
				if (options.snapshotResult === undefined) {
					throw new Error("fake transport: no snapshot result configured");
				}
				return options.snapshotResult();
			}
			if (args[0] === "pane" && args[1] === "read") {
				if (options.paneReadResult === undefined) {
					throw new Error("fake transport: no pane read result configured");
				}
				return options.paneReadResult();
			}
			throw new Error(`fake transport: unexpected command ${JSON.stringify(args)}`);
		},
		subscribeEvents: () => {
			if (options.eventLines === undefined) {
				throw new Error("fake transport: no event lines configured");
			}
			return options.eventLines();
		},
	};
	return { transport, calls };
}

async function collectEvents(
	client: HerdrCommandClient,
	limit: number,
): Promise<unknown[]> {
	const collected: unknown[] = [];
	for await (const event of client.events()) {
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

test("readPane reports invalid-response for a malformed pane identifier or line limit", async () => {
	const { transport, calls } = makeFakeTransport({});
	const client = new HerdrCommandClient(transport);

	assert.equal((await client.readPane("", 5) as { code: string }).code, "invalid-response");
	assert.equal((await client.readPane(SELF_PANE_ID, 0) as { code: string }).code, "invalid-response");
	assert.equal((await client.readPane(SELF_PANE_ID, -1) as { code: string }).code, "invalid-response");
	assert.equal((await client.readPane(SELF_PANE_ID, 1.5) as { code: string }).code, "invalid-response");
	assert.deepEqual(calls, [], "an invalid request must never reach the transport");
});

// TODO(AGNT-0066.T01): Prove unknown and malformed events request recovery and the stream continues.
test("TC-05 events yields normalized events and skips an unknown event", async () => {
	const { transport } = makeFakeTransport({
		eventLines: () =>
			linesFrom([
				JSON.stringify(makeWorkspaceUpdatedEvent()),
				JSON.stringify(makeUnknownEvent()),
				JSON.stringify(makePaneUpdatedEvent()),
			]),
	});
	const client = new HerdrCommandClient(transport);

	const events = await collectEvents(client, 2);

	assert.equal(events.length, 2);
	assert.deepEqual(events[0], {
		type: "workspace-changed",
		workspace: { id: SELF_WORKSPACE_ID, label: "jerusalem (renamed)", worktree: { path: "/Users/pi/workspaces/jerusalem", branch: null }, isFocused: true },
	});
	assert.equal((events[1] as { type: string }).type, "pane-changed");
});

test("events reports invalid-response for a line that is not JSON and continues", async () => {
	const { transport } = makeFakeTransport({
		eventLines: () => linesFrom(["not json", JSON.stringify(makeWorkspaceUpdatedEvent())]),
	});
	const client = new HerdrCommandClient(transport);

	const events = await collectEvents(client, 2);

	assert.equal((events[0] as { code: string }).code, "invalid-response");
	assert.equal((events[1] as { type: string }).type, "workspace-changed");
});

test("events reports invalid-response for a line that does not decode to an object", async () => {
	const { transport } = makeFakeTransport({
		eventLines: () => linesFrom(["42", JSON.stringify(makeWorkspaceUpdatedEvent())]),
	});
	const client = new HerdrCommandClient(transport);

	const events = await collectEvents(client, 2);

	assert.equal((events[0] as { code: string }).code, "invalid-response");
	assert.equal((events[1] as { type: string }).type, "workspace-changed");
});

test("events reports invalid-response for a malformed recognized event and continues", async () => {
	const { transport } = makeFakeTransport({
		eventLines: () =>
			linesFrom([
				JSON.stringify(makeMalformedWorkspaceUpdatedEvent()),
				JSON.stringify(makePaneUpdatedEvent()),
			]),
	});
	const client = new HerdrCommandClient(transport);

	const events = await collectEvents(client, 2);

	assert.equal((events[0] as { code: string }).code, "invalid-response");
	assert.equal((events[1] as { type: string }).type, "pane-changed");
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
		yield JSON.stringify(makeWorkspaceUpdatedEvent());
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
		eventLines: () => linesFrom([JSON.stringify(makePaneUpdatedEvent())]),
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
