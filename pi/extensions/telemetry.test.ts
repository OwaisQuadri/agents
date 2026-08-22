import assert from "node:assert/strict";
import { mkdir, mkdtemp, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import type { ExtensionAPI, ExtensionCommandContext, ExtensionContext } from "@earendil-works/pi-coding-agent";
import {
	appendRecord,
	attachFeedback,
	createTelemetryRuntime,
	filterRuns,
	loadStore,
	registerCommands,
	registerLifecycle,
	settleRun,
	startRun,
	telemetryCounts,
} from "./telemetry.ts";

type Handler = (payload?: unknown, ctx?: ExtensionContext) => unknown;
type CommandHandler = (args: string, ctx: ExtensionCommandContext) => Promise<void> | void;

type StatusUpdate = {
	key: string;
	text: string | undefined;
};

type RecordingUi = {
	statuses: StatusUpdate[];
	notifications: Array<{ message: string; type?: "info" | "warning" | "error" }>;
	ui: {
		setStatus(key: string, text: string | undefined): void;
		notify(message: string, type?: "info" | "warning" | "error"): void;
		theme: {
			fg(mode: string, text: string): string;
		};
	};
};

const runRecordKeys = [
	"recordType",
	"runId",
	"parentRunId",
	"packageName",
	"packageVersion",
	"agentName",
	"startedAt",
	"settledAt",
	"durationMs",
	"status",
	"tokens",
	"costUsd",
] as const;

const tokenUsageKeys = ["input", "output", "cacheRead", "cacheWrite"] as const;
const contentBearingKeys = ["summary", "task", "goal", "output", "errors", "paths", "toolArguments", "results"] as const;
const flatTokenKeys = ["inputTokens", "outputTokens", "safeInputTokens", "safeOutputTokens"] as const;

const feedbackRecordKeys = ["recordType", "runId", "value", "createdAt"] as const;

const validRunRecord = {
	recordType: "run",
	runId: "run-1",
	parentRunId: null,
	packageName: "pi",
	packageVersion: "1.0.0",
	agentName: null,
	startedAt: "2026-08-17T02:24:00.000Z",
	settledAt: "2026-08-17T02:24:10.000Z",
	durationMs: 10000,
	status: "succeeded",
	tokens: {
		input: 10,
		output: 20,
		cacheRead: null,
		cacheWrite: null,
	},
	costUsd: 0.25,
} as const;

const validFeedbackRecord = {
	recordType: "feedback",
	runId: "run-1",
	value: "accepted",
	createdAt: "2026-08-17T02:24:11.000Z",
} as const;

async function withTelemetryDirectory<T>(directory: string, run: () => Promise<T>): Promise<T> {
	const originalDirectory = process.env.PI_CODING_AGENT_DIR;
	process.env.PI_CODING_AGENT_DIR = directory;

	try {
		return await run();
	} finally {
		if (originalDirectory === undefined) {
			delete process.env.PI_CODING_AGENT_DIR;
		} else {
			process.env.PI_CODING_AGENT_DIR = originalDirectory;
		}
	}
}

function createRecordingUi(): RecordingUi {
	const statuses: StatusUpdate[] = [];
	const notifications: RecordingUi["notifications"] = [];

	return {
		statuses,
		notifications,
		ui: {
			setStatus(key: string, text: string | undefined) {
				statuses.push({ key, text });
			},
			notify(message: string, type?: "info" | "warning" | "error") {
				notifications.push({ message, type });
			},
			theme: {
				fg(_mode: string, text: string) {
					return text;
				},
			},
		},
	};
}

function createFakeLifecycleContext(recordingUi = createRecordingUi()): ExtensionContext {
	return {
		ui: recordingUi.ui,
		mode: "tui",
		hasUI: true,
		cwd: "/tmp",
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
}

function createFakeCommandContext(recordingUi = createRecordingUi()): ExtensionCommandContext {
	return {
		...createFakeLifecycleContext(recordingUi),
		getSystemPromptOptions: () => ({} as never),
		waitForIdle: async () => undefined,
		newSession: async () => ({ cancelled: false }),
		fork: async () => ({ cancelled: false }),
		navigateTree: async () => ({ cancelled: false }),
		switchSession: async () => ({ cancelled: false }),
		reload: async () => undefined,
	} as ExtensionCommandContext;
}

function createFakeExtensionAPI() {
	const handlers = new Map<string, Handler>();
	const commandHandlers = new Map<string, CommandHandler>();
	const api = {
		on(event: string, handler: Handler) {
			handlers.set(`on:${event}`, handler);
		},
		events: {
			on(event: string, handler: Handler) {
				handlers.set(`events:${event}`, handler);
			},
		},
		registerCommand(name: string, options: { handler: CommandHandler }) {
			commandHandlers.set(name, options.handler);
		},
	} as unknown as ExtensionAPI;

	return {
		api,
		handlers,
		commandHandlers,
		on(event: string): Handler {
			const handler = handlers.get(`on:${event}`);
			assert.ok(handler, `missing handler for ${event}`);
			return handler;
		},
		event(event: string): Handler {
			const handler = handlers.get(`events:${event}`);
			assert.ok(handler, `missing event handler for ${event}`);
			return handler;
		},
		command(name: string): CommandHandler {
			const handler = commandHandlers.get(name);
			assert.ok(handler, `missing command for ${name}`);
			return handler;
		},
		commandNames(): string[] {
			return [...commandHandlers.keys()];
		},
	};
}

function assertRunRecord(record: Record<string, unknown>): void {
	assert.deepEqual(Object.keys(record), [...runRecordKeys]);
	assert.deepEqual(Object.keys(record.tokens as Record<string, unknown>), [...tokenUsageKeys]);
	for (const key of contentBearingKeys) {
		assert.equal(key in record, false);
	}
	for (const key of flatTokenKeys) {
		assert.equal(key in record, false);
	}
}

function assertFeedbackRecord(record: Record<string, unknown>): void {
	assert.deepEqual(Object.keys(record), [...feedbackRecordKeys]);
}

async function invoke(handler: Handler, payload?: unknown, ctx?: ExtensionContext): Promise<unknown> {
	if (ctx === undefined) {
		return await handler(payload);
	}

	return await handler(payload, ctx);
}

async function invokeCommand(handler: CommandHandler, args: string, ctx = createFakeCommandContext()): Promise<void> {
	await handler(args, ctx);
}

test("telemetry store loading and appending", async () => {
	const absentDirectory = await mkdtemp(join(tmpdir(), "telemetry-store-"));
	await withTelemetryDirectory(absentDirectory, async () => {
		const store = await loadStore();
		const expectedPath = join(absentDirectory, "telemetry.jsonl");

		assert.equal(store.path, expectedPath);
		assert.deepEqual(store.records, []);
		await assert.rejects(readFile(expectedPath, "utf8"));
	});

	const loadedDirectory = await mkdtemp(join(tmpdir(), "telemetry-store-"));
	await withTelemetryDirectory(loadedDirectory, async () => {
		const path = join(loadedDirectory, "telemetry.jsonl");
		await writeFile(path, `${JSON.stringify(validRunRecord)}\n${JSON.stringify(validFeedbackRecord)}\n`);

		const store = await loadStore();
		assert.deepEqual(store.records, [validRunRecord, validFeedbackRecord]);
		assertRunRecord(store.records[0] as Record<string, unknown>);
		assertFeedbackRecord(store.records[1] as Record<string, unknown>);
	});

	const rejectedDirectory = await mkdtemp(join(tmpdir(), "telemetry-store-"));
	await withTelemetryDirectory(rejectedDirectory, async () => {
		const path = join(rejectedDirectory, "telemetry.jsonl");
		await writeFile(path, `${JSON.stringify({ ...validRunRecord, prompt: "secret" })}\n`);

		await assert.rejects(loadStore(), /closed schema/);
	});

	const appendDirectory = await mkdtemp(join(tmpdir(), "telemetry-store-"));
	await withTelemetryDirectory(appendDirectory, async () => {
		const store = await loadStore();
		await appendRecord(store, validFeedbackRecord);

		assert.deepEqual(store.records, [validFeedbackRecord]);
		assertFeedbackRecord(store.records[0] as Record<string, unknown>);
		assert.equal((await readFile(store.path, "utf8")).trim(), JSON.stringify(validFeedbackRecord));
	});

	const invalidAppendDirectory = await mkdtemp(join(tmpdir(), "telemetry-store-"));
	await withTelemetryDirectory(invalidAppendDirectory, async () => {
		const store = await loadStore();
		const invalidRecord = { ...validRunRecord, path: "/tmp/secret" } as Parameters<typeof appendRecord>[1];

		await assert.rejects(appendRecord(store, invalidRecord), /closed schema/);
		assert.deepEqual(store.records, []);
		await assert.rejects(readFile(store.path, "utf8"));
	});

	const blockedDirectory = await mkdtemp(join(tmpdir(), "telemetry-store-"));
	await withTelemetryDirectory(blockedDirectory, async () => {
		const blockedPath = join(blockedDirectory, "blocked");
		await mkdir(blockedPath);
		const store: Parameters<typeof appendRecord>[0] = { path: blockedPath, records: [] };

		await assert.rejects(appendRecord(store, validRunRecord), /directory|EISDIR/i);
		assert.deepEqual(store.records, []);
	});
});

test("telemetry status command notifies active and failed counts including zero", async () => {
	const runtime = createTelemetryRuntime({
		path: "/tmp/telemetry.jsonl",
		records: [
			{
				recordType: "run",
				runId: "run-failed",
				parentRunId: null,
				packageName: "pi",
				packageVersion: "1.0.0",
				agentName: null,
				startedAt: "2026-08-17T02:24:00.000Z",
				settledAt: "2026-08-17T02:24:10.000Z",
				durationMs: 10000,
				status: "failed",
				tokens: {
					input: 10,
					output: 20,
					cacheRead: null,
					cacheWrite: null,
				},
				costUsd: 0.25,
			},
		],
	});
	runtime.activeRuns.set("active-1", {
		startedAt: "2026-08-17T02:23:00.000Z",
		packageName: "pi",
		parentRunId: null,
		agentName: null,
	});

	const api = createFakeExtensionAPI();
	registerCommands(api.api, runtime);
	assert.deepEqual(api.commandNames(), ["telemetry-status", "telemetry-runs", "telemetry-feedback"]);

	const recordingUi = createRecordingUi();
	await invokeCommand(api.command("telemetry-status"), "", createFakeCommandContext(recordingUi));

	assert.deepEqual(recordingUi.statuses, []);
	assert.deepEqual(recordingUi.notifications, [{ message: "active: 1 failed: 1", type: undefined }]);

	const emptyRuntime = createTelemetryRuntime({ path: "/tmp/empty-telemetry.jsonl", records: [] });
	const emptyApi = createFakeExtensionAPI();
	registerCommands(emptyApi.api, emptyRuntime);
	const emptyRecordingUi = createRecordingUi();
	await invokeCommand(emptyApi.command("telemetry-status"), "", createFakeCommandContext(emptyRecordingUi));

	assert.deepEqual(emptyRecordingUi.statuses, []);
	assert.deepEqual(emptyRecordingUi.notifications, [{ message: "active: 0 failed: 0", type: undefined }]);
});


test("telemetry lifecycle does not add a footer status", async () => {
	const directory = await mkdtemp(join(tmpdir(), "telemetry-status-surface-"));
	await withTelemetryDirectory(directory, async () => {
		const runtime = createTelemetryRuntime({ path: join(directory, "telemetry.jsonl"), records: [] });
		const api = createFakeExtensionAPI();
		registerLifecycle(api.api, runtime);

		const recordingUi = createRecordingUi();
		await invoke(api.event("subagents:started"), { id: "async-1", type: "subagent-a" });
		await invoke(api.event("subagents:completed"), { id: "async-1", status: "completed" });

		assert.deepEqual(recordingUi.statuses, []);
		assert.deepEqual(recordingUi.notifications, []);
	});
});


test("telemetry filtered-run command returns matching runs in storage order", async () => {
	const store: Parameters<typeof createTelemetryRuntime>[0] = {
		path: "/tmp/telemetry.jsonl",
		records: [
			{
				recordType: "feedback",
				runId: "run-2",
				value: "accepted",
				createdAt: "2026-08-17T02:24:01.000Z",
			},
			{
				recordType: "run",
				runId: "run-1",
				parentRunId: null,
				packageName: "pi",
				packageVersion: "1.0.0",
				agentName: "agent-a",
				startedAt: "2026-08-17T02:24:00.000Z",
				settledAt: "2026-08-17T02:24:10.000Z",
				durationMs: 10000,
				status: "succeeded",
				tokens: {
					input: 10,
					output: 20,
					cacheRead: null,
					cacheWrite: null,
				},
				costUsd: 0.25,
			},
			{
				recordType: "run",
				runId: "run-2",
				parentRunId: null,
				packageName: "pi",
				packageVersion: "1.0.0",
				agentName: "agent-a",
				startedAt: "2026-08-17T02:25:00.000Z",
				settledAt: "2026-08-17T02:25:20.000Z",
				durationMs: 20000,
				status: "succeeded",
				tokens: {
					input: 15,
					output: 30,
					cacheRead: null,
					cacheWrite: null,
				},
				costUsd: 0.25,
			},
			{
				recordType: "feedback",
				runId: "run-1",
				value: "accepted",
				createdAt: "2026-08-17T02:24:11.000Z",
			},
			{
				recordType: "run",
				runId: "run-3",
				parentRunId: null,
				packageName: "pi",
				packageVersion: "1.0.0",
				agentName: "agent-a",
				startedAt: "2026-08-17T02:26:00.000Z",
				settledAt: "2026-08-17T02:26:09.000Z",
				durationMs: 9000,
				status: "succeeded",
				tokens: {
					input: 8,
					output: 16,
					cacheRead: null,
					cacheWrite: null,
				},
				costUsd: 0.3,
			},
			{
				recordType: "feedback",
				runId: "run-3",
				value: "rejected",
				createdAt: "2026-08-17T02:26:11.000Z",
			},
		],
	};

	const api = createFakeExtensionAPI();
	registerCommands(api.api, createTelemetryRuntime(store));
	assert.deepEqual(api.commandNames(), ["telemetry-status", "telemetry-runs", "telemetry-feedback"]);

	const recordingUi = createRecordingUi();
	const filter = {
		packageName: "pi",
		packageVersion: "1.0.0",
		agentName: "agent-a",
		status: "succeeded",
		minimumDurationMs: 1000,
		maximumCostUsd: 0.25,
		feedback: "accepted",
	};

	await invokeCommand(api.command("telemetry-runs"), JSON.stringify(filter), createFakeCommandContext(recordingUi));

	assert.deepEqual(recordingUi.statuses, []);
	assert.equal(recordingUi.notifications.length, 1);
	const parsed = JSON.parse(recordingUi.notifications[0]?.message ?? "null") as Array<Record<string, unknown>>;
	assert.deepEqual(parsed.map((record) => record.runId), ["run-1", "run-2"]);
	assertRunRecord(parsed[0] as Record<string, unknown>);
	assertRunRecord(parsed[1] as Record<string, unknown>);
});


test("telemetry feedback command accepts categorical feedback and rejects free text", async () => {
	const directory = await mkdtemp(join(tmpdir(), "telemetry-feedback-command-"));
	await withTelemetryDirectory(directory, async () => {
		const store = await loadStore();
		await appendRecord(store, validRunRecord);
		const runtime = createTelemetryRuntime(store);
		const api = createFakeExtensionAPI();
		registerCommands(api.api, runtime);

		const recordingUi = createRecordingUi();
		await invokeCommand(api.command("telemetry-feedback"), "run-1 accepted", createFakeCommandContext(recordingUi));

		assert.equal(store.records.length, 2);
		const feedbackRecord = store.records[1] as Record<string, unknown>;
		assertFeedbackRecord(feedbackRecord);
		assert.equal(feedbackRecord.runId, "run-1");
		assert.equal(feedbackRecord.value, "accepted");
		assert.equal(Number.isFinite(Date.parse(feedbackRecord.createdAt as string)), true);

		await assert.rejects(
			invokeCommand(api.command("telemetry-feedback"), "run-1 accepted extra", createFakeCommandContext(recordingUi)),
			/exactly runId/,
		);
		await assert.rejects(
			invokeCommand(api.command("telemetry-feedback"), "run-1 corrected", createFakeCommandContext(recordingUi)),
			/already exists/,
		);
	});
});


test("telemetry feedback helper rejects orphan runs and invalid timestamps", async () => {
	const orphanRuntime = createTelemetryRuntime({ path: "/tmp/telemetry.jsonl", records: [] });
	await assert.rejects(attachFeedback(orphanRuntime, "missing-run", "accepted", "2026-08-17T02:24:11.000Z"), /no settled run/);
	assert.deepEqual(orphanRuntime.store.records, []);

	const settledRuntime = createTelemetryRuntime({ path: "/tmp/telemetry.jsonl", records: [validRunRecord] });
	await assert.rejects(attachFeedback(settledRuntime, "run-1", "accepted", "not-a-timestamp"), /createdAt/);
	assert.deepEqual(settledRuntime.store.records, [validRunRecord]);
});


test("telemetry feedback helper leaves memory unchanged when append fails", async () => {
	const blockedDirectory = await mkdtemp(join(tmpdir(), "telemetry-feedback-blocked-"));
	await withTelemetryDirectory(blockedDirectory, async () => {
		const blockedPath = join(blockedDirectory, "blocked");
		await mkdir(blockedPath);
		const runtime = createTelemetryRuntime({ path: blockedPath, records: [validRunRecord] });

		await assert.rejects(attachFeedback(runtime, "run-1", "accepted", validFeedbackRecord.createdAt), /directory|EISDIR/i);
		assert.equal(runtime.store.records.length, 1);
		await assert.rejects(readFile(runtime.store.path, "utf8"));

		runtime.store.path = join(blockedDirectory, "retry.jsonl");
		await writeFile(runtime.store.path, `${JSON.stringify(validRunRecord)}\n`);
		await attachFeedback(runtime, "run-1", "accepted", validFeedbackRecord.createdAt);
		assert.equal(runtime.store.records.length, 2);
		assert.equal((await loadStore(runtime.store.path)).records.length, 2);
	});
});

test("telemetry serializes concurrent feedback for one run", async () => {
	const directory = await mkdtemp(join(tmpdir(), "telemetry-feedback-concurrent-"));
	await withTelemetryDirectory(directory, async () => {
		const store = await loadStore();
		await appendRecord(store, validRunRecord);
		const runtime = createTelemetryRuntime(store);

		const first = attachFeedback(runtime, "run-1", "accepted", validFeedbackRecord.createdAt);
		const second = attachFeedback(runtime, "run-1", "corrected", validFeedbackRecord.createdAt);

		await assert.rejects(second, /already pending/);
		await first;
		assert.deepEqual(
			store.records.filter((record) => record.recordType === "feedback").map((record) => record.value),
			["accepted"],
		);
		assert.equal((await loadStore()).records.length, 2);
	});
});


test("telemetry filters combine all fields and keep storage order", () => {
	const store = {
		path: "/tmp/telemetry.jsonl",
		records: [
			{
				recordType: "feedback",
				runId: "run-2",
				value: "accepted",
				createdAt: "2026-08-17T02:24:01.000Z",
			},
			{
				recordType: "run",
				runId: "run-1",
				parentRunId: null,
				packageName: "pi",
				packageVersion: "1.0.0",
				agentName: "agent-a",
				startedAt: "2026-08-17T02:24:00.000Z",
				settledAt: "2026-08-17T02:24:10.000Z",
				durationMs: 1000,
				status: "succeeded",
				tokens: {
					input: 10,
					output: 20,
					cacheRead: null,
					cacheWrite: null,
				},
				costUsd: 0.25,
			},
			{
				recordType: "run",
				runId: "run-2",
				parentRunId: null,
				packageName: "pi",
				packageVersion: "1.0.0",
				agentName: "agent-a",
				startedAt: "2026-08-17T02:25:00.000Z",
				settledAt: "2026-08-17T02:25:20.000Z",
				durationMs: 1500,
				status: "succeeded",
				tokens: {
					input: 15,
					output: 30,
					cacheRead: null,
					cacheWrite: null,
				},
				costUsd: 0.25,
			},
			{
				recordType: "feedback",
				runId: "run-1",
				value: "accepted",
				createdAt: "2026-08-17T02:24:11.000Z",
			},
			{
				recordType: "run",
				runId: "run-3",
				parentRunId: null,
				packageName: "pi",
				packageVersion: "1.0.0",
				agentName: "agent-a",
				startedAt: "2026-08-17T02:26:00.000Z",
				settledAt: "2026-08-17T02:26:10.000Z",
				durationMs: 900,
				status: "succeeded",
				tokens: {
					input: 8,
					output: 16,
					cacheRead: null,
					cacheWrite: null,
				},
				costUsd: 0.30,
			},
			{
				recordType: "feedback",
				runId: "run-3",
				value: "rejected",
				createdAt: "2026-08-17T02:26:11.000Z",
			},
		],
	};

	const result = filterRuns(store, {
		packageName: "pi",
		packageVersion: "1.0.0",
		agentName: "agent-a",
		status: "succeeded",
		minimumDurationMs: 1000,
		maximumCostUsd: 0.25,
		feedback: "accepted",
	});

	assert.deepEqual(
		result.map((record) => record.runId),
		["run-1", "run-2"],
	);
});

test("telemetry filters reject negative boundaries and missing settled runs", () => {
	const validStore = {
		path: "/tmp/telemetry.jsonl",
		records: [validRunRecord],
	};

	assert.throws(() => filterRuns(validStore, { minimumDurationMs: -1 }), /non-negative/);
	assert.throws(() => filterRuns(validStore, { maximumCostUsd: -0.01 }), /non-negative/);
	assert.throws(
		() =>
			filterRuns(
				{
					path: "/tmp/telemetry.jsonl",
					records: [
						{
							recordType: "feedback",
							runId: "missing-run",
							value: "accepted",
							createdAt: "2026-08-17T02:24:11.000Z",
						},
					],
				},
				{},
			),
		/no settled run/,
	);
});

test("telemetry counts active runtime entries and failed stored runs separately", () => {
	const runtime = {
		activeRuns: new Map([
			[
				"active-1",
				{
					startedAt: "2026-08-17T02:24:00.000Z",
					packageName: "pi",
					packageVersion: "1.0.0",
					parentRunId: null,
					agentName: null,
				},
			],
			[
				"active-2",
				{
					startedAt: "2026-08-17T02:25:00.000Z",
					packageName: "pi",
					packageVersion: "1.0.0",
					parentRunId: null,
					agentName: "agent-a",
				},
			],
		]),
		store: {
			path: "/tmp/telemetry.jsonl",
			records: [
				{
					recordType: "run",
					runId: "run-1",
					parentRunId: null,
					packageName: "pi",
					packageVersion: "1.0.0",
					agentName: null,
					startedAt: "2026-08-17T02:24:00.000Z",
					settledAt: "2026-08-17T02:24:10.000Z",
					durationMs: 1000,
					status: "failed",
					tokens: {
						input: 10,
						output: 20,
						cacheRead: null,
						cacheWrite: null,
					},
					costUsd: 0.25,
				},
				{
					recordType: "run",
					runId: "run-2",
					parentRunId: null,
					packageName: "pi",
					packageVersion: "1.0.0",
					agentName: null,
					startedAt: "2026-08-17T02:25:00.000Z",
					settledAt: "2026-08-17T02:25:10.000Z",
					durationMs: 1000,
					status: "succeeded",
					tokens: {
						input: 10,
						output: 20,
						cacheRead: null,
						cacheWrite: null,
					},
					costUsd: 0.25,
				},
				validFeedbackRecord,
			],
		},
	};

	assert.deepEqual(telemetryCounts(runtime), { active: 2, failed: 1 });
});

test("telemetry start and settle validation preserve active runs on append failure", async () => {
	const blockedDirectory = await mkdtemp(join(tmpdir(), "telemetry-start-settle-"));
	await withTelemetryDirectory(blockedDirectory, async () => {
		const store = { path: join(blockedDirectory, "blocked"), records: [] };
		await mkdir(store.path);
		const runtime = createTelemetryRuntime(store);
		const startedAt = "2026-08-17T02:24:00.000Z";
		const settledAt = "2026-08-17T02:24:05.000Z";
		const tokens = {
			input: null,
			output: null,
			cacheRead: null,
			cacheWrite: null,
		};

		assert.throws(() => startRun(runtime, "run-direct", "", null, startedAt), /packageName/);
		assert.throws(() => settleRun(runtime, "run-direct", null, "", "succeeded", tokens, null, settledAt), /packageVersion/);
		assert.throws(() => settleRun(runtime, "run-direct", null, "9.9.9", "succeeded", { ...tokens, input: -1 }, null, settledAt), /tokens/);

		startRun(runtime, "run-direct", "custom-package", null, startedAt);
		assert.deepEqual(runtime.activeRuns.get("run-direct"), {
			startedAt,
			packageName: "custom-package",
			parentRunId: null,
			agentName: null,
		});

		await assert.rejects(settleRun(runtime, "run-direct", null, "9.9.9", "succeeded", tokens, null, settledAt), /directory|EISDIR/i);
		assert.equal(runtime.activeRuns.has("run-direct"), true);
		assert.deepEqual(runtime.store.records, []);
		await assert.rejects(readFile(store.path, "utf8"));

		runtime.store.path = join(blockedDirectory, "retry.jsonl");
		await settleRun(runtime, "run-direct", null, "9.9.9", "succeeded", tokens, null, settledAt);
		assert.equal(runtime.activeRuns.has("run-direct"), false);
		assert.equal((await loadStore(runtime.store.path)).records.length, 1);
	});
});

test("telemetry serializes concurrent settlement for one run", async () => {
	const directory = await mkdtemp(join(tmpdir(), "telemetry-settle-concurrent-"));
	await withTelemetryDirectory(directory, async () => {
		const store = await loadStore();
		const runtime = createTelemetryRuntime(store);
		const startedAt = "2026-08-17T02:24:00.000Z";
		const settledAt = "2026-08-17T02:24:05.000Z";
		const tokens = { input: null, output: null, cacheRead: null, cacheWrite: null };
		startRun(runtime, "run-concurrent", "custom-package", null, startedAt);

		const first = settleRun(runtime, "run-concurrent", null, "9.9.9", "succeeded", tokens, null, settledAt);
		const second = settleRun(runtime, "run-concurrent", null, "9.9.9", "failed", tokens, null, settledAt);

		await assert.rejects(second, /already pending/);
		await first;
		assert.equal(runtime.activeRuns.has("run-concurrent"), false);
		assert.equal(store.records.length, 1);
		assert.equal((store.records[0] as RunRecord).status, "succeeded");
		assert.equal((await loadStore()).records.length, 1);
	});
});

test("telemetry parent lifecycle records runtime package and null metrics", async () => {
	const directory = await mkdtemp(join(tmpdir(), "telemetry-parent-"));
	await withTelemetryDirectory(directory, async () => {
		const store = await loadStore();
		const runtime = createTelemetryRuntime(store);
		const api = createFakeExtensionAPI();
		registerLifecycle(api.api, runtime);

		const startHandler = api.on("agent_start");
		const settledHandler = api.on("agent_settled");
		const ctx = createFakeLifecycleContext();

		await invoke(startHandler, undefined, ctx);
		assert.equal(runtime.activeRuns.size, 1);
		const parentRunId = [...runtime.activeRuns.keys()][0];
		assert.ok(parentRunId);

		await invoke(settledHandler, undefined, ctx);
		assert.equal(runtime.activeRuns.size, 0);
		assert.equal(runtime.currentParentRunId, null);
		assert.equal(store.records.length, 1);

		const record = store.records[0] as Record<string, unknown>;
		assertRunRecord(record);
		assert.equal(record.recordType, "run");
		assert.equal(record.runId, parentRunId);
		assert.equal(record.parentRunId, null);
		assert.equal(record.packageName, "@earendil-works/pi-coding-agent");
		assert.equal(record.packageVersion, "0.84.2");
		assert.equal(record.agentName, null);
		assert.equal(record.status, "succeeded");
		assert.deepEqual(record.tokens, {
			input: null,
			output: null,
			cacheRead: null,
			cacheWrite: null,
		});
		assert.equal(record.costUsd, null);
		assert.equal(typeof record.startedAt, "string");
		assert.equal(typeof record.settledAt, "string");
		assert.equal(typeof record.durationMs, "number");
		assert.equal((record.durationMs as number) >= 0, true);
	});
});

test("telemetry subagents lifecycle records pinned package and normalized usage", async () => {
	const directory = await mkdtemp(join(tmpdir(), "telemetry-subagents-success-"));
	await withTelemetryDirectory(directory, async () => {
		const store = await loadStore();
		const runtime = createTelemetryRuntime(store);
		const api = createFakeExtensionAPI();
		registerLifecycle(api.api, runtime);

		const startedHandler = api.event("subagents:started");
		const completedHandler = api.event("subagents:completed");

		await invoke(startedHandler, { id: "run-1", type: "subagent-a", description: "review the diff" });
		assert.equal(runtime.activeRuns.has("run-1"), true);
		assert.deepEqual(runtime.activeRuns.get("run-1"), {
			startedAt: runtime.activeRuns.get("run-1")?.startedAt,
			packageName: "pi-subagents",
			parentRunId: null,
			agentName: "subagent-a",
		});

		const startedAt = runtime.activeRuns.get("run-1")?.startedAt;
		assert.ok(startedAt);
		await invoke(completedHandler, {
			id: "run-1",
			type: "subagent-a",
			status: "completed",
			durationMs: 10_000,
			toolUses: 3,
			usage: {
				input: 12,
				output: 34,
				cacheRead: 7,
				cacheWrite: 11,
				totalTokens: 64,
				cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 1.5 },
			},
		});

		assert.equal(runtime.activeRuns.size, 0);
		assert.equal(store.records.length, 1);

		const record = store.records[0] as Record<string, unknown>;
		assertRunRecord(record);
		assert.equal(record.recordType, "run");
		assert.equal(record.runId, "run-1");
		assert.equal(record.parentRunId, null);
		assert.equal(record.packageName, "pi-subagents");
		assert.equal(record.packageVersion, "0.18.0");
		assert.equal(record.agentName, "subagent-a");
		assert.equal(record.status, "succeeded");
		assert.deepEqual(record.tokens, {
			input: 12,
			output: 34,
			cacheRead: 7,
			cacheWrite: 11,
		});
		assert.equal(record.costUsd, 1.5);
	});
});

test("telemetry subagents lifecycle maps steered, failure, and cancellation", async () => {
	const directory = await mkdtemp(join(tmpdir(), "telemetry-subagents-status-"));
	await withTelemetryDirectory(directory, async () => {
		const store = await loadStore();
		const runtime = createTelemetryRuntime(store);
		const api = createFakeExtensionAPI();
		registerLifecycle(api.api, runtime);

		const startedHandler = api.event("subagents:started");
		const completedHandler = api.event("subagents:completed");
		const failedHandler = api.event("subagents:failed");

		await invoke(startedHandler, { id: "run-steered", type: "subagent-a" });
		await invoke(completedHandler, { id: "run-steered", status: "steered", durationMs: 1_000 });

		await invoke(startedHandler, { id: "run-error", type: "subagent-a" });
		await invoke(failedHandler, { id: "run-error", status: "error", durationMs: 1_000 });

		await invoke(startedHandler, { id: "run-stopped", type: "subagent-b" });
		await invoke(failedHandler, { id: "run-stopped", status: "stopped", durationMs: 1_000 });

		await invoke(startedHandler, { id: "run-aborted", type: "subagent-b" });
		await invoke(failedHandler, { id: "run-aborted", status: "aborted", durationMs: 1_000 });

		assert.equal(runtime.activeRuns.size, 0);
		assert.equal(store.records.length, 4);
		assert.deepEqual(
			store.records.map((record) => (record.recordType === "run" ? [record.runId, record.status] : null)),
			[
				["run-steered", "succeeded"],
				["run-error", "failed"],
				["run-stopped", "cancelled"],
				["run-aborted", "cancelled"],
			],
		);

		for (const record of store.records) {
			if (record.recordType === "run") {
				assertRunRecord(record as Record<string, unknown>);
				assert.deepEqual((record as Record<string, unknown>).tokens, {
					input: null,
					output: null,
					cacheRead: null,
					cacheWrite: null,
				});
				assert.equal((record as Record<string, unknown>).costUsd, null);
			}
		}
	});
});

test("telemetry rejects malformed and re-settled lifecycle events", async () => {
	const directory = await mkdtemp(join(tmpdir(), "telemetry-malformed-"));
	await withTelemetryDirectory(directory, async () => {
		const store = await loadStore();
		const runtime = createTelemetryRuntime(store);
		const api = createFakeExtensionAPI();
		registerLifecycle(api.api, runtime);

		const startedHandler = api.event("subagents:started");
		const completedHandler = api.event("subagents:completed");

		await assert.rejects(invoke(startedHandler, { id: "", type: "subagent-a" }), /runId/);
		assert.equal(runtime.activeRuns.size, 0);
		assert.equal(store.records.length, 0);

		await invoke(startedHandler, { id: "run-bad", type: "subagent-a" });
		await assert.rejects(
			invoke(completedHandler, {
				id: "run-bad",
				status: "completed",
				usage: {
					input: "bad",
					output: 2,
					cacheRead: 0,
					cacheWrite: 0,
					totalTokens: 2,
					cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 1 },
				},
			}),
			/usage.input/,
		);
		assert.equal(runtime.activeRuns.has("run-bad"), true);
		assert.equal(store.records.length, 0);

		await invoke(startedHandler, { id: "run-dup", type: "subagent-a" });
		await invoke(completedHandler, { id: "run-dup", status: "completed", durationMs: 1_000 });
		assert.equal(store.records.length, 1);

		await assert.rejects(
			invoke(completedHandler, { id: "run-dup", status: "completed", durationMs: 1_000 }),
			/is missing/,
		);
		assert.equal(store.records.length, 1);
		assert.equal((store.records[0] as RunRecord).runId, "run-dup");
	});
});

test("telemetry shutdown cancels remaining active runs", async () => {
	const directory = await mkdtemp(join(tmpdir(), "telemetry-shutdown-"));
	await withTelemetryDirectory(directory, async () => {
		const store = await loadStore();
		const runtime = createTelemetryRuntime(store);
		const api = createFakeExtensionAPI();
		registerLifecycle(api.api, runtime);

		const startHandler = api.on("agent_start");
		const startedHandler = api.event("subagents:started");
		const shutdownHandler = api.on("session_shutdown");
		const ctx = createFakeLifecycleContext();

		await invoke(startHandler, undefined, ctx);
		const parentRunId = [...runtime.activeRuns.keys()][0];
		assert.ok(parentRunId);

		await invoke(startedHandler, { id: "run-shutdown", type: "subagent-a" });
		assert.equal(runtime.activeRuns.size, 2);

		await invoke(shutdownHandler, { type: "session_shutdown", reason: "quit" }, ctx);

		assert.equal(runtime.activeRuns.size, 0);
		assert.equal(runtime.currentParentRunId, null);
		assert.equal(store.records.length, 2);
		assert.deepEqual(
			store.records.map((record) => (record.recordType === "run" ? record.status : null)),
			["cancelled", "cancelled"],
		);
		for (const record of store.records) {
			if (record.recordType === "run") {
				assertRunRecord(record as Record<string, unknown>);
			}
		}
	});
});

test("telemetry shutdown attempts every run and reports partial failure", async () => {
	const directory = await mkdtemp(join(tmpdir(), "telemetry-shutdown-partial-"));
	await withTelemetryDirectory(directory, async () => {
		const store = await loadStore();
		const runtime = createTelemetryRuntime(store);
		const api = createFakeExtensionAPI();
		const recordingUi = createRecordingUi();
		registerLifecycle(api.api, runtime);
		startRun(runtime, "invalid-shutdown", "other-package", null, "2026-08-17T02:24:00.000Z");
		startRun(runtime, "valid-shutdown", runtime.packageName, null, "2026-08-17T02:24:01.000Z");

		const shutdownHandler = api.on("session_shutdown");
		const ctx = createFakeLifecycleContext(recordingUi);
		await assert.rejects(invoke(shutdownHandler, { type: "session_shutdown", reason: "quit" }, ctx), (error: unknown) => {
			assert.ok(error instanceof AggregateError);
			assert.match(error.message, /failed to settle 1 active run/);
			assert.equal(error.errors.length, 1);
			assert.match(String(error.errors[0]), /packageVersion/);
			return true;
		});

		assert.deepEqual([...runtime.activeRuns.keys()], ["invalid-shutdown"]);
		assert.equal(store.records.length, 1);
		assert.equal((store.records[0] as RunRecord).runId, "valid-shutdown");
		assert.equal((store.records[0] as RunRecord).status, "cancelled");
		assert.deepEqual(recordingUi.statuses, []);
		assert.equal((await loadStore()).records.length, 1);
	});
});
