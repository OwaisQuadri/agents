import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { createServer, type Server, type Socket } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

import type { ExtensionAPI, ExtensionCommandContext, ExtensionContext } from "@earendil-works/pi-coding-agent";

import herdrState, { createTransport, registerHerdrStateCommand } from "./herdr-state.ts";
import { HerdrCommandClient, type HerdrClient } from "./herdr-state/client.ts";
import {
	makeSnapshotResponse,
	OTHER_CWD,
	OTHER_PANE_ID,
	OTHER_TAB_ID,
	OTHER_WORKSPACE_ID,
	SELF_CWD,
	SELF_PANE_ID,
	SELF_TAB_ID,
	SELF_WORKSPACE_ID,
} from "./herdr-state/fixtures.ts";
import type { HerdrPaneOutput, HerdrSnapshotResponse, HerdrStateEvent, HerdrStateFailure } from "./herdr-state/types.ts";

type CommandHandler = (args: string, ctx: ExtensionCommandContext) => Promise<void> | void;

interface RegisteredCommand {
	description?: string;
	handler: CommandHandler;
}

/**
 * Builds a fake `ExtensionAPI` that only implements `registerCommand`,
 * matching the mocked Pi command-registration pattern already used by
 * `telemetry.test.ts` and `live-diff.test.ts`.
 */
function createFakeExtensionAPI(execImpl?: ExtensionAPI["exec"]) {
	const commands = new Map<string, RegisteredCommand>();
	const execCalls: { command: string; args: string[]; options?: unknown }[] = [];
	const api = {
		registerCommand(name: string, options: RegisteredCommand) {
			commands.set(name, options);
		},
		exec: async (command: string, args: string[], options?: unknown) => {
			execCalls.push(options === undefined ? { command, args } : { command, args, options });
			if (execImpl === undefined) {
				throw new Error("fake extension API: no exec implementation configured");
			}
			return execImpl(command, args, options as never);
		},
	} as unknown as ExtensionAPI;

	return {
		api,
		execCalls,
		commandNames(): string[] {
			return [...commands.keys()];
		},
		command(name: string): RegisteredCommand {
			const registered = commands.get(name);
			assert.ok(registered, `missing command for ${name}`);
			return registered;
		},
	};
}

function createFakeCommandContext(cwd: string): { ctx: ExtensionCommandContext; notifications: string[] } {
	const notifications: string[] = [];
	const lifecycle: ExtensionContext = {
		ui: {
			notify: (message: string) => {
				notifications.push(message);
			},
		} as ExtensionContext["ui"],
		mode: "tui",
		hasUI: true,
		cwd,
		sessionManager: {} as never,
		modelRegistry: {} as never,
		model: undefined,
		scopedModels: [],
		thinkingLevel: undefined,
		isIdle: () => true,
		isProjectTrusted: () => true,
		signal: undefined,
		abort: () => undefined,
		hasPendingMessages: () => false,
		shutdown: () => undefined,
		getContextUsage: () => undefined,
		compact: () => undefined,
		getSystemPrompt: () => "",
	} as ExtensionContext;

	const ctx: ExtensionCommandContext = {
		...lifecycle,
		getSystemPromptOptions: () => ({} as never),
		waitForIdle: async () => undefined,
		newSession: async () => ({ cancelled: false }),
		fork: async () => ({ cancelled: false }),
		navigateTree: async () => ({ cancelled: false }),
		switchSession: async () => ({ cancelled: false }),
		reload: async () => undefined,
	} as ExtensionCommandContext;

	return { ctx, notifications };
}

interface FakeClientOptions {
	snapshotResult?: () => Promise<HerdrSnapshotResponse | HerdrStateFailure>;
	paneReadResult?: (paneId: string, lineLimit: number) => Promise<HerdrPaneOutput | HerdrStateFailure>;
}

/**
 * Builds a fake `HerdrClient` that records every `readPane` call and
 * dispatches configured responses, matching the injected-client testing
 * seam named in `.map/AGNT-0066/testability.md`.
 */
function makeFakeClient(options: FakeClientOptions): {
	client: HerdrClient;
	snapshotCallCount: () => number;
	paneReadCalls: { paneId: string; lineLimit: number }[];
} {
	let snapshotCallCount = 0;
	const paneReadCalls: { paneId: string; lineLimit: number }[] = [];
	const client: HerdrClient = {
		snapshot: async () => {
			snapshotCallCount += 1;
			if (options.snapshotResult === undefined) {
				throw new Error("fake client: no snapshot result configured");
			}
			return options.snapshotResult();
		},
		async *events(): AsyncIterable<HerdrStateEvent | HerdrStateFailure> {
			throw new Error("fake client: events() must not be called by the state command");
		},
		readPane: async (paneId: string, lineLimit: number) => {
			paneReadCalls.push({ paneId, lineLimit });
			if (options.paneReadResult === undefined) {
				throw new Error("fake client: no pane read result configured");
			}
			return options.paneReadResult(paneId, lineLimit);
		},
	};
	return { client, snapshotCallCount: () => snapshotCallCount, paneReadCalls };
}

function okSnapshot(): () => Promise<HerdrSnapshotResponse> {
	const response = makeSnapshotResponse();
	return async () => response;
}

async function listenOnScratchSocket(
	onConnection: (socket: Socket) => void,
): Promise<{ directory: string; socketPath: string; server: Server }> {
	const directory = await mkdtemp(join(tmpdir(), "pi-herdr-state-"));
	const socketPath = join(directory, "herdr.sock");
	const server = createServer(onConnection);
	await new Promise<void>((resolve, reject) => {
		server.once("error", reject);
		server.listen(socketPath, () => {
			server.off("error", reject);
			resolve();
		});
	});
	return { directory, socketPath, server };
}

async function closeScratchSocket(server: Server, directory: string): Promise<void> {
	await new Promise<void>((resolve, reject) => {
		server.close((error) => error === undefined ? resolve() : reject(error));
	});
	await rm(directory, { recursive: true, force: true });
}

const EXPECTED_EVENT_FILTERS = [
	"workspace.created",
	"workspace.updated",
	"workspace.metadata_updated",
	"workspace.renamed",
	"workspace.moved",
	"workspace.reordered",
	"workspace.closed",
	"workspace.focused",
	"worktree.created",
	"worktree.opened",
	"worktree.removed",
	"tab.created",
	"tab.closed",
	"tab.focused",
	"tab.renamed",
	"tab.moved",
	"pane.created",
	"pane.closed",
	"pane.updated",
	"pane.focused",
	"pane.moved",
	"pane.exited",
	"pane.agent_detected",
	"layout.updated",
];
const EXPECTED_SUBSCRIPTION_REQUEST = `${JSON.stringify({
	id: "pi-herdr-state-events",
	method: "events.subscribe",
	params: { subscriptions: EXPECTED_EVENT_FILTERS.map((type) => ({ type })) },
})}\n`;
const SUBSCRIPTION_ACKNOWLEDGEMENT = `${JSON.stringify({
	id: "pi-herdr-state-events",
	result: { type: "subscription_started" },
})}\n`;

const NO_INJECTED_PANE_ID = Symbol("no injected Herdr pane identifier");

async function runCommand(
	client: HerdrClient,
	args: string,
	cwd: string,
	selfPaneId: string | typeof NO_INJECTED_PANE_ID = SELF_PANE_ID,
): Promise<{ notifications: string[] }> {
	const fakeApi = createFakeExtensionAPI();
	registerHerdrStateCommand(fakeApi.api, client);
	const { ctx, notifications } = createFakeCommandContext(cwd);

	const originalPaneId = process.env.HERDR_PANE_ID;
	if (selfPaneId === NO_INJECTED_PANE_ID) {
		delete process.env.HERDR_PANE_ID;
	} else {
		process.env.HERDR_PANE_ID = selfPaneId;
	}
	try {
		await fakeApi.command("herdr-state").handler(args, ctx);
	} finally {
		if (originalPaneId === undefined) {
			delete process.env.HERDR_PANE_ID;
		} else {
			process.env.HERDR_PANE_ID = originalPaneId;
		}
	}
	return { notifications };
}

// TODO(AGNT-0066.T10): Prove lifecycle start, cached reads, recovery, and shutdown.
test("registers exactly one read-only herdr-state command", () => {
	const fakeApi = createFakeExtensionAPI();
	const { client } = makeFakeClient({});

	registerHerdrStateCommand(fakeApi.api, client);

	assert.deepEqual(fakeApi.commandNames(), ["herdr-state"]);
	const registered = fakeApi.command("herdr-state");
	assert.match(registered.description ?? "", /workspace/i);
});

test("TC-01 global state lists every workspace and marks Pi's workspace, tab, and pane", async () => {
	const { client } = makeFakeClient({ snapshotResult: okSnapshot() });

	const { notifications } = await runCommand(client, "", SELF_CWD);

	assert.equal(notifications.length, 1);
	const output = notifications[0] ?? "";
	assert.match(output, /jerusalem/);
	assert.match(output, /edinburgh/);
	assert.match(output, new RegExp(`Pi location: workspace ${SELF_WORKSPACE_ID}, tab ${SELF_TAB_ID}, pane ${SELF_PANE_ID}\\.`));
});

test("global state reports Pi's location as absent when no pane or working directory matches", async () => {
	const { client } = makeFakeClient({ snapshotResult: okSnapshot() });

	const { notifications } = await runCommand(client, "", "/no/such/directory", NO_INJECTED_PANE_ID);

	assert.match(notifications[0] ?? "", /Pi location: not found/);
});

test("TC-02 workspace detail is scoped to the requested workspace", async () => {
	const { client } = makeFakeClient({ snapshotResult: okSnapshot() });

	const { notifications } = await runCommand(client, `workspace ${OTHER_WORKSPACE_ID}`, SELF_CWD);

	const output = notifications[0] ?? "";
	assert.match(output, new RegExp(OTHER_TAB_ID));
	assert.match(output, new RegExp(OTHER_PANE_ID));
	assert.doesNotMatch(output, new RegExp(SELF_TAB_ID));
	assert.doesNotMatch(output, new RegExp(SELF_PANE_ID));
});

test("workspace detail reports an absent workspace explicitly", async () => {
	const { client } = makeFakeClient({ snapshotResult: okSnapshot() });

	const { notifications } = await runCommand(client, "workspace no-such-workspace", SELF_CWD);

	assert.equal(notifications[0], "Workspace no-such-workspace is absent from the current Herdr session.");
});

test("TC-03 pane output is bounded and visibly reports truncation", async () => {
	const longText = "line2\nline3\nline4\nline5\nline6";
	const { client, paneReadCalls } = makeFakeClient({
		snapshotResult: okSnapshot(),
		paneReadResult: async (paneId, lineLimit) => ({ paneId, text: longText, isTruncated: true }),
	});

	const { notifications } = await runCommand(client, `pane ${SELF_PANE_ID} 5`, SELF_CWD);

	assert.deepEqual(paneReadCalls, [{ paneId: SELF_PANE_ID, lineLimit: 5 }]);
	const output = notifications[0] ?? "";
	assert.match(output, /truncated/);
	assert.equal(output.split("\n").filter((line) => line.startsWith("line")).length, 5);
});

test("pane detail uses the default bound when no line limit is given", async () => {
	const { client, paneReadCalls } = makeFakeClient({
		snapshotResult: okSnapshot(),
		paneReadResult: async (paneId) => ({ paneId, text: "hello", isTruncated: false }),
	});

	await runCommand(client, `pane ${SELF_PANE_ID}`, SELF_CWD);

	assert.equal(paneReadCalls[0]?.lineLimit, 200);
});

test("accepts exact pane line-limit boundaries", async () => {
	for (const lineLimit of [1, 10_000]) {
		const { client, snapshotCallCount, paneReadCalls } = makeFakeClient({
			snapshotResult: okSnapshot(),
			paneReadResult: async (paneId) => ({ paneId, text: "hello", isTruncated: false }),
		});

		await runCommand(client, `pane ${SELF_PANE_ID} ${lineLimit}`, SELF_CWD);

		assert.equal(snapshotCallCount(), 1);
		assert.deepEqual(paneReadCalls, [{ paneId: SELF_PANE_ID, lineLimit }]);
	}
});

test("TC-07 pane text with escapes and a fake workspace label renders as literal bounded data", async () => {
	const hostileText = "\x1b[31mrm -rf /\x1b[0m\nworkspace fake-workspace focused=true\nherdr api snapshot";
	const { client } = makeFakeClient({
		snapshotResult: okSnapshot(),
		paneReadResult: async (paneId) => ({ paneId, text: hostileText, isTruncated: false }),
	});

	const { notifications } = await runCommand(client, `pane ${SELF_PANE_ID}`, SELF_CWD);

	assert.match(notifications[0] ?? "", /rm -rf \//);

	const { notifications: globalAfter } = await runCommand(client, "", SELF_CWD);
	assert.match(globalAfter[0] ?? "", /jerusalem/);
	assert.match(globalAfter[0] ?? "", /edinburgh/);
	assert.match(globalAfter[0] ?? "", new RegExp(`Pi location: workspace ${SELF_WORKSPACE_ID}, tab ${SELF_TAB_ID}, pane ${SELF_PANE_ID}\\.`));
});

test("TC-08 pane detail reports not-found explicitly and leaves the global listing unaffected", async () => {
	const notFound: HerdrStateFailure = { code: "not-found", message: "pane bogus-pane-id not found" };
	const { client } = makeFakeClient({
		snapshotResult: okSnapshot(),
		paneReadResult: async () => notFound,
	});

	const { notifications } = await runCommand(client, "pane bogus-pane-id", SELF_CWD);
	assert.equal(notifications[0], "Herdr state is not available (not-found): pane bogus-pane-id not found");

	const { notifications: globalAfter } = await runCommand(client, "", SELF_CWD);
	assert.match(globalAfter[0] ?? "", /jerusalem/);
	assert.match(globalAfter[0] ?? "", /edinburgh/);
});

test("global state reports an unavailable Herdr session explicitly", async () => {
	const failure: HerdrStateFailure = { code: "unavailable", message: "herdr: no running session" };
	const { client } = makeFakeClient({ snapshotResult: async () => failure });

	const { notifications } = await runCommand(client, "", SELF_CWD);

	assert.equal(notifications[0], "Herdr state is not available (unavailable): herdr: no running session");
});

test("workspace and pane detail also report an unavailable Herdr session before reading anything else", async () => {
	const failure: HerdrStateFailure = { code: "unavailable", message: "herdr: no running session" };
	const { client, paneReadCalls } = makeFakeClient({ snapshotResult: async () => failure });

	const { notifications: workspaceNotifications } = await runCommand(
		client,
		`workspace ${SELF_WORKSPACE_ID}`,
		SELF_CWD,
	);
	assert.match(workspaceNotifications[0] ?? "", /unavailable/);

	const { notifications: paneNotifications } = await runCommand(client, `pane ${SELF_PANE_ID}`, SELF_CWD);
	assert.match(paneNotifications[0] ?? "", /unavailable/);
	assert.deepEqual(paneReadCalls, [], "pane read must never run once the snapshot is unavailable");
});

test("rejects unrecognized and malformed arguments without querying Herdr", async () => {
	const { client } = makeFakeClient({});
	const fakeApi = createFakeExtensionAPI();
	registerHerdrStateCommand(fakeApi.api, client);
	const { ctx } = createFakeCommandContext(SELF_CWD);
	const handler = fakeApi.command("herdr-state").handler;

	await assert.rejects(() => Promise.resolve(handler("bogus", ctx)));
	await assert.rejects(() => Promise.resolve(handler("workspace", ctx)));
	await assert.rejects(() => Promise.resolve(handler("pane", ctx)));
});

test("rejects invalid explicit pane line-limit tokens before querying Herdr", async (t) => {
	const usage = "Usage: /herdr-state [workspace <workspace-id> | pane <pane-id> [line-limit]]";
	const rejectedTokens = ["0", "10001", "9007199254740992", "1e3", "-1", "1.5", "not-a-number"];

	for (const token of rejectedTokens) {
		await t.test(token, async () => {
			const { client, snapshotCallCount, paneReadCalls } = makeFakeClient({});
			const fakeApi = createFakeExtensionAPI();
			registerHerdrStateCommand(fakeApi.api, client);
			const { ctx } = createFakeCommandContext(SELF_CWD);
			const handler = fakeApi.command("herdr-state").handler;

			await assert.rejects(
				() => Promise.resolve(handler(`pane ${SELF_PANE_ID} ${token}`, ctx)),
				(error: unknown) => {
					assert.ok(error instanceof Error);
					assert.ok(error.message.includes(usage));
					return true;
				},
			);
			assert.equal(snapshotCallCount(), 0);
			assert.deepEqual(paneReadCalls, []);
		});
	}
});

test("the command never calls the client's live event subscription", async () => {
	const { client } = makeFakeClient({
		snapshotResult: okSnapshot(),
		paneReadResult: async (paneId) => ({ paneId, text: "hello", isTruncated: false }),
	});

	await runCommand(client, "", SELF_CWD);
	await runCommand(client, `workspace ${SELF_WORKSPACE_ID}`, SELF_CWD);
	await runCommand(client, `pane ${SELF_PANE_ID}`, SELF_CWD);
	// `events()` throws in the fake client above if invoked; reaching this
	// point without an unhandled rejection is the assertion.
});

test("TC-18 transport forwards snapshot cancellation to Pi exec", async () => {
	const controller = new AbortController();
	const reason = new Error("stop snapshot");
	let receivedSignal: AbortSignal | undefined;
	const fakeApi = createFakeExtensionAPI(async (_command, _args, options) => {
		receivedSignal = options?.signal;
		return new Promise<never>((_resolve, reject) => {
			options?.signal?.addEventListener("abort", () => reject(options.signal?.reason), {
				once: true,
			});
		});
	});
	const client = new HerdrCommandClient(createTransport(fakeApi.api));

	const pending = client.snapshot(controller.signal);
	controller.abort(reason);

	const result = await pending;

	assert.equal((result as { code: string }).code, "unavailable");
	assert.match((result as { message: string }).message, /stop snapshot/);
	assert.equal(receivedSignal, controller.signal);
	assert.equal(
		(fakeApi.execCalls[0]?.options as { signal?: AbortSignal } | undefined)?.signal,
		controller.signal,
	);
});

test("TC-20 socket transport sends one exact read-only subscription and reads an envelope", async () => {
	let received = "";
	let serverSocket: Socket | undefined;
	const requestReceived = Promise.withResolvers<void>();
	const scratch = await listenOnScratchSocket((socket) => {
		serverSocket = socket;
		socket.setEncoding("utf8");
		socket.on("data", (chunk) => {
			received += String(chunk);
			if (!received.includes("\n")) {
				return;
			}
			requestReceived.resolve();
			socket.write(`${JSON.stringify({
				id: "pi-herdr-state-events",
				result: { type: "subscription_started" },
			})}\n`);
			const data = {
				type: "workspace_updated",
				workspace: {
					workspace_id: SELF_WORKSPACE_ID,
					number: 1,
					label: "jerusalem (renamed)",
					focused: true,
					pane_count: 1,
					tab_count: 1,
					active_tab_id: SELF_TAB_ID,
					agent_status: "idle",
					worktree: {
						repo_key: "agents",
						repo_name: "agents",
						repo_root: SELF_CWD,
						checkout_path: SELF_CWD,
						is_linked_worktree: true,
					},
				},
			};
			socket.write(`${JSON.stringify({ event: data.type, data })}\n`);
		});
	});
	const originalSocketPath = process.env.HERDR_SOCKET_PATH;
	process.env.HERDR_SOCKET_PATH = scratch.socketPath;
	try {
		const fakeApi = createFakeExtensionAPI();
		const client = new HerdrCommandClient(createTransport(fakeApi.api));
		const events: unknown[] = [];
		for await (const event of client.events()) {
			events.push(event);
			break;
		}
		await requestReceived.promise;

		assert.equal(received, EXPECTED_SUBSCRIPTION_REQUEST);
		assert.equal(EXPECTED_EVENT_FILTERS.length, 24);
		assert.equal(events.length, 1);
		assert.equal((events[0] as { type: string }).type, "workspace-changed");
		assert.deepEqual(fakeApi.execCalls, []);
	} finally {
		if (originalSocketPath === undefined) {
			delete process.env.HERDR_SOCKET_PATH;
		} else {
			process.env.HERDR_SOCKET_PATH = originalSocketPath;
		}
		serverSocket?.destroy();
		await closeScratchSocket(scratch.server, scratch.directory);
	}
});

test("socket closure after acknowledgement yields one unavailable result", async () => {
	let received = "";
	let isRequestReceived = false;
	let serverSocket: Socket | undefined;
	const requestReceived = Promise.withResolvers<string>();
	const scratch = await listenOnScratchSocket((socket) => {
		serverSocket = socket;
		socket.setEncoding("utf8");
		socket.on("data", (chunk) => {
			received += String(chunk);
			if (isRequestReceived || !received.includes("\n")) {
				return;
			}
			isRequestReceived = true;
			requestReceived.resolve(received);
			socket.end(SUBSCRIPTION_ACKNOWLEDGEMENT);
		});
	});
	const originalSocketPath = process.env.HERDR_SOCKET_PATH;
	process.env.HERDR_SOCKET_PATH = scratch.socketPath;
	try {
		const fakeApi = createFakeExtensionAPI();
		const client = new HerdrCommandClient(createTransport(fakeApi.api));
		const events: unknown[] = [];
		for await (const event of client.events()) {
			events.push(event);
		}

		assert.equal(await requestReceived.promise, EXPECTED_SUBSCRIPTION_REQUEST);
		assert.equal(events.length, 1);
		assert.equal((events[0] as { code: string }).code, "unavailable");
	} finally {
		if (originalSocketPath === undefined) {
			delete process.env.HERDR_SOCKET_PATH;
		} else {
			process.env.HERDR_SOCKET_PATH = originalSocketPath;
		}
		serverSocket?.destroy();
		await closeScratchSocket(scratch.server, scratch.directory);
	}
});

test("TC-21 socket abort closes the event stream without unavailable", async () => {
	let serverSocket: Socket | undefined;
	const requestReceived = Promise.withResolvers<void>();
	const socketClosed = Promise.withResolvers<void>();
	const scratch = await listenOnScratchSocket((socket) => {
		serverSocket = socket;
		socket.setEncoding("utf8");
		socket.once("close", () => socketClosed.resolve());
		socket.once("data", () => {
			requestReceived.resolve();
			socket.write(`${JSON.stringify({
				id: "pi-herdr-state-events",
				result: { type: "subscription_started" },
			})}\n`);
		});
	});
	const originalSocketPath = process.env.HERDR_SOCKET_PATH;
	process.env.HERDR_SOCKET_PATH = scratch.socketPath;
	try {
		const controller = new AbortController();
		const fakeApi = createFakeExtensionAPI();
		const client = new HerdrCommandClient(createTransport(fakeApi.api));
		const events: unknown[] = [];
		const collecting = (async () => {
			for await (const event of client.events(controller.signal)) {
				events.push(event);
			}
		})();
		await requestReceived.promise;
		await new Promise<void>((resolve) => setImmediate(resolve));
		controller.abort();
		await collecting;
		await socketClosed.promise;

		assert.deepEqual(events, []);
		assert.deepEqual(fakeApi.execCalls, []);
	} finally {
		if (originalSocketPath === undefined) {
			delete process.env.HERDR_SOCKET_PATH;
		} else {
			process.env.HERDR_SOCKET_PATH = originalSocketPath;
		}
		serverSocket?.destroy();
		await closeScratchSocket(scratch.server, scratch.directory);
	}
});

test("TC-09 the default export wires a transport that only issues read-only herdr commands", async () => {
	const response = makeSnapshotResponse();
	const fakeApi = createFakeExtensionAPI(async (command, args) => {
		assert.equal(command, "herdr");
		if (args[0] === "api" && args[1] === "snapshot") {
			return { stdout: JSON.stringify(response), stderr: "", code: 0, killed: false };
		}
		if (args[0] === "pane" && args[1] === "read") {
			return { stdout: "hello\n", stderr: "", code: 0, killed: false };
		}
		throw new Error(`unexpected herdr command: ${JSON.stringify(args)}`);
	});

	herdrState(fakeApi.api);

	assert.deepEqual(fakeApi.commandNames(), ["herdr-state"]);
	const { ctx, notifications } = createFakeCommandContext(OTHER_CWD);
	await fakeApi.command("herdr-state").handler("", ctx);

	assert.match(notifications[0] ?? "", /jerusalem/);
	assert.deepEqual(
		fakeApi.execCalls.map((call) => call.args),
		[["api", "snapshot"]],
	);

	await fakeApi.command("herdr-state").handler(`pane ${OTHER_PANE_ID}`, ctx);
	assert.deepEqual(
		fakeApi.execCalls.map((call) => call.args.slice(0, 2)),
		[
			["api", "snapshot"],
			["api", "snapshot"],
			["pane", "read"],
		],
	);
});
