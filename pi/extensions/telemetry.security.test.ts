import assert from "node:assert/strict";
import { mkdir, mkdtemp, readFile, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import {
	appendRecord,
	attachFeedback,
	createTelemetryRuntime,
	loadStore,
	registerLifecycle,
	settleRun,
	startRun,
} from "./telemetry.ts";

type TelemetryStore = Parameters<typeof appendRecord>[0];
type TelemetryRecord = Parameters<typeof appendRecord>[1];
type TelemetryRuntime = ReturnType<typeof createTelemetryRuntime>;

type StoreLike = Pick<TelemetryStore, "path" | "records">;
type RuntimeLike = Pick<TelemetryRuntime, "store" | "activeRuns" | "currentParentRunId">;

type LifecycleHandler = (payload?: unknown, context?: unknown) => unknown;

type StoreSnapshot = {
	bytes: string | undefined;
	records: readonly TelemetryRecord[];
};

type RuntimeSnapshot = StoreSnapshot & {
	activeRuns: readonly [string, unknown][];
	currentParentRunId: string | null;
};

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

async function withTelemetryRoot<T>(run: (context: { container: string; root: string }) => Promise<T>): Promise<T> {
	const container = await mkdtemp(join(tmpdir(), "telemetry-security-"));
	const root = join(container, "agent");
	await mkdir(root);

	const originalDirectory = process.env.PI_CODING_AGENT_DIR;
	process.env.PI_CODING_AGENT_DIR = root;

	try {
		return await run({ container, root });
	} finally {
		if (originalDirectory === undefined) {
			delete process.env.PI_CODING_AGENT_DIR;
		} else {
			process.env.PI_CODING_AGENT_DIR = originalDirectory;
		}

		await rm(container, { recursive: true, force: true });
	}
}

function cloneJson<T>(value: T): T {
	return JSON.parse(JSON.stringify(value)) as T;
}

async function readOptionalFile(path: string): Promise<string | undefined> {
	try {
		return await readFile(path, "utf8");
	} catch (error) {
		if (error instanceof Error && "code" in error && (error as { code?: string }).code === "ENOENT") {
			return undefined;
		}

		throw error;
	}
}

async function snapshotStoreState(store: StoreLike): Promise<StoreSnapshot> {
	return {
		bytes: await readOptionalFile(store.path),
		records: cloneJson(store.records),
	};
}

async function snapshotRuntimeState(runtime: RuntimeLike): Promise<RuntimeSnapshot> {
	return {
		bytes: await readOptionalFile(runtime.store.path),
		records: cloneJson(runtime.store.records),
		activeRuns: cloneJson([...runtime.activeRuns.entries()]),
		currentParentRunId: runtime.currentParentRunId,
	};
}

async function assertStoreStateUnchanged(store: StoreLike, snapshot: StoreSnapshot): Promise<void> {
	assert.equal(await readOptionalFile(store.path), snapshot.bytes);
	assert.deepEqual(store.records, snapshot.records);
}

async function assertRuntimeStateUnchanged(runtime: RuntimeLike, snapshot: RuntimeSnapshot): Promise<void> {
	assert.equal(await readOptionalFile(runtime.store.path), snapshot.bytes);
	assert.deepEqual(runtime.store.records, snapshot.records);
	assert.deepEqual(cloneJson([...runtime.activeRuns.entries()]), snapshot.activeRuns);
	assert.equal(runtime.currentParentRunId, snapshot.currentParentRunId);
}

async function assertRejectedStoreOperation(
	operation: () => Promise<unknown>,
	store: StoreLike,
	snapshot: StoreSnapshot,
	matcher: RegExp | string,
): Promise<void> {
	await assert.rejects(() => Promise.resolve().then(operation), matcher);
	await assertStoreStateUnchanged(store, snapshot);
}

async function assertRejectedRuntimeOperation(
	operation: () => Promise<unknown>,
	runtime: RuntimeLike,
	snapshot: RuntimeSnapshot,
	matcher: RegExp | string,
): Promise<void> {
	await assert.rejects(() => Promise.resolve().then(operation), matcher);
	await assertRuntimeStateUnchanged(runtime, snapshot);
}

async function loadStoreRejected(path: string): Promise<boolean> {
	try {
		await loadStore(path);
		return false;
	} catch {
		return true;
	}
}

function createFakeLifecycleApi(): {
	api: Parameters<typeof registerLifecycle>[0];
	handler: (event: string) => LifecycleHandler;
} {
	const handlers = new Map<string, LifecycleHandler>();
	const api = {
		on(event: string, handler: LifecycleHandler) {
			handlers.set(`on:${event}`, handler);
		},
		events: {
			on(event: string, handler: LifecycleHandler) {
				handlers.set(`events:${event}`, handler);
			},
		},
	} as unknown as Parameters<typeof registerLifecycle>[0];

	return {
		api,
		handler(event: string): LifecycleHandler {
			const handler = handlers.get(event);
			assert.ok(handler, `missing lifecycle handler ${event}`);
			return handler;
		},
	};
}

async function invoke(handler: LifecycleHandler, payload?: unknown, context?: unknown): Promise<unknown> {
	return await handler(payload, context);
}

test("telemetry loadStore rejects malformed lines and schema drift", async () => {
	await withTelemetryRoot(async () => {
		const store = await loadStore();
		const cases = [
			{
				name: "malformed JSON",
				contents: "{not json}\n",
				matcher: /not valid JSON/,
			},
			{
				name: "blank line",
				contents: `\n${JSON.stringify(validRunRecord)}\n`,
				matcher: /line 1 is empty/,
			},
			{
				name: "truncated line",
				contents: `${JSON.stringify(validRunRecord)}\n{"recordType":"feedback","runId":"run-1"`,
				matcher: /not valid JSON/,
			},
			{
				name: "extra content fields",
				contents: `${JSON.stringify({
					...validRunRecord,
					prompt: "prompt",
					output: "output",
					tool: "tool",
					path: "/tmp/escape",
					freeText: "free text",
				})}\n`,
				matcher: /closed schema/,
			},
			{
				name: "missing run key",
				contents: `${JSON.stringify((() => {
					const record = { ...validRunRecord } as Record<string, unknown>;
					delete record.tokens;
					return record;
				})())}\n`,
				matcher: /closed schema/,
			},
			{
				name: "missing feedback key",
				contents: `${JSON.stringify((() => {
					const record = { ...validFeedbackRecord } as Record<string, unknown>;
					delete record.createdAt;
					return record;
				})())}\n`,
				matcher: /closed schema/,
			},
			{
				name: "orphan feedback",
				contents: `${JSON.stringify({ ...validFeedbackRecord, runId: "missing-run" })}\n`,
				matcher: /no settled run/,
			},
			{
				name: "duplicate feedback",
				contents: `${JSON.stringify(validRunRecord)}\n${JSON.stringify(validFeedbackRecord)}\n${JSON.stringify({
					...validFeedbackRecord,
					value: "corrected",
				})}\n`,
				matcher: /already exists/,
			},
		];

		for (const entry of cases) {
			await writeFile(store.path, entry.contents);
			const snapshot = await snapshotStoreState(store);
			await assertRejectedStoreOperation(() => loadStore(), store, snapshot, entry.matcher);
		}
	});
});

test("telemetry loadStore rejects duplicate run identifiers and preserves valid relationships", async () => {
	await withTelemetryRoot(async () => {
		const store = await loadStore();
		const duplicateRunRecord = {
			...validRunRecord,
			packageName: "pi-subagents",
			agentName: "reviewer",
			startedAt: "2026-08-17T02:25:00.000Z",
			settledAt: "2026-08-17T02:25:20.000Z",
			durationMs: 20000,
		};
		const duplicateContents = `${JSON.stringify(validRunRecord)}\n${JSON.stringify(duplicateRunRecord)}\n`;
		await writeFile(store.path, duplicateContents);

		await assert.rejects(() => loadStore(), /runId run-1 already exists/);
		assert.equal(await readFile(store.path, "utf8"), duplicateContents);
		assert.deepEqual(store.records, []);

		const validContents = `${JSON.stringify(validRunRecord)}\n${JSON.stringify(validFeedbackRecord)}\n`;
		await writeFile(store.path, validContents);
		const loadedStore = await loadStore();

		assert.deepEqual(loadedStore.records, [validRunRecord, validFeedbackRecord]);
		assert.equal(await readFile(store.path, "utf8"), validContents);
	});
});

test("telemetry appendRecord rejects invalid metrics without changing storage", async () => {
	await withTelemetryRoot(async () => {
		const store = await loadStore();
		const invalidRecords = [
			{
				name: "negative duration and metrics",
				record: {
					...validRunRecord,
					durationMs: -1,
					tokens: {
						input: -1,
						output: 20,
						cacheRead: null,
						cacheWrite: null,
					},
					costUsd: -0.25,
				} as TelemetryRecord,
			},
			{
				name: "nonfinite duration and metrics",
				record: {
					...validRunRecord,
					durationMs: Number.POSITIVE_INFINITY,
					tokens: {
						input: 10,
						output: Number.NEGATIVE_INFINITY,
						cacheRead: null,
						cacheWrite: null,
					},
					costUsd: Number.NaN,
				} as TelemetryRecord,
			},
		];

		for (const entry of invalidRecords) {
			const snapshot = await snapshotStoreState(store);
			await assertRejectedStoreOperation(() => appendRecord(store, entry.record), store, snapshot, /closed schema/);
		}
	});
});

test("telemetry attachFeedback rejects free text, orphan feedback, and duplicates without changing storage", async () => {
	await withTelemetryRoot(async () => {
		const orphanStore = await loadStore();
		const orphanRuntime = createTelemetryRuntime(orphanStore);
		const orphanSnapshot = await snapshotStoreState(orphanStore);
		await assertRejectedStoreOperation(
			() => attachFeedback(orphanRuntime, "missing-run", "accepted", validFeedbackRecord.createdAt),
			orphanStore,
			orphanSnapshot,
			/no settled run/,
		);

		const store = await loadStore();
		await appendRecord(store, validRunRecord);
		const runtime = createTelemetryRuntime(store);

		const freeTextSnapshot = await snapshotStoreState(store);
		await assertRejectedStoreOperation(
			() => attachFeedback(runtime, "run-1", "helpful" as never, validFeedbackRecord.createdAt),
			store,
			freeTextSnapshot,
			/accepted, corrected, or rejected/,
		);

		await attachFeedback(runtime, "run-1", "accepted", validFeedbackRecord.createdAt);
		const duplicateSnapshot = await snapshotStoreState(store);
		await assertRejectedStoreOperation(
			() => attachFeedback(runtime, "run-1", "corrected", validFeedbackRecord.createdAt),
			store,
			duplicateSnapshot,
			/already exists/,
		);
	});
});

test("telemetry settleRun rejects invalid chronology without changing storage", async () => {
	await withTelemetryRoot(async () => {
		const store = await loadStore();
		const runtime = createTelemetryRuntime(store);
		startRun(runtime, "run-1", "pi", null, "2026-08-17T02:24:10.000Z");

		const snapshot = await snapshotRuntimeState(runtime);
		await assertRejectedRuntimeOperation(
			() => settleRun(
				runtime,
				"run-1",
				null,
				"1.0.0",
				"succeeded",
				{
					input: 10,
					output: 20,
					cacheRead: null,
					cacheWrite: null,
				},
				null,
				"2026-08-17T02:24:09.000Z",
			),
			runtime,
			snapshot,
			/must not be earlier than startedAt/,
		);
	});
});

test("telemetry default loadStore and appendRecord stay beneath PI_CODING_AGENT_DIR and ignore outside sentinels", async () => {
	await withTelemetryRoot(async ({ container, root }) => {
		const sentinelPath = join(container, "sentinel.txt");
		await writeFile(sentinelPath, "sentinel");

		const store = await loadStore();
		assert.equal(store.path, join(root, "telemetry.jsonl"));
		assert.deepEqual(store.records, []);

		const sentinelBefore = await readFile(sentinelPath, "utf8");
		await appendRecord(store, validRunRecord);

		assert.equal(await readFile(store.path, "utf8"), `${JSON.stringify(validRunRecord)}\n`);
		assert.equal(await readFile(sentinelPath, "utf8"), sentinelBefore);

		const outsideDirectory = join(container, "outside");
		await mkdir(outsideDirectory);
		const outsideStore = {
			path: join(outsideDirectory, "telemetry.jsonl"),
			records: [],
		};
		const outsideSnapshot = await snapshotStoreState(outsideStore);
		await assertRejectedStoreOperation(() => appendRecord(outsideStore, validRunRecord), outsideStore, outsideSnapshot, /configured root/);
	});
});

test("telemetry rejects explicit and symlinked outside-root loadStore paths", async () => {
	await withTelemetryRoot(async ({ container, root }) => {
		const baselineStore = await loadStore();
		const baselineSnapshot = await snapshotStoreState(baselineStore);

		const outsideDirectory = join(container, "outside");
		await mkdir(outsideDirectory);
		const outsideStore = join(outsideDirectory, "telemetry.jsonl");
		await writeFile(outsideStore, `${JSON.stringify(validRunRecord)}\n`);

		const linkPath = join(root, "escape.jsonl");
		await symlink(outsideStore, linkPath);

		const explicitBefore = await readOptionalFile(outsideStore);
		const explicitRejected = await loadStoreRejected(outsideStore);
		assert.equal(await readOptionalFile(outsideStore), explicitBefore);
		assert.deepEqual(baselineStore.records, baselineSnapshot.records);
		assert.equal(explicitRejected, true, "explicit outside-root path must reject");

		const symlinkBefore = await readOptionalFile(linkPath);
		const symlinkRejected = await loadStoreRejected(linkPath);
		assert.equal(await readOptionalFile(linkPath), symlinkBefore);
		assert.deepEqual(baselineStore.records, baselineSnapshot.records);
		assert.equal(symlinkRejected, true, "symlink outside-root path must reject");

		const traversalPath = `${root}/../escape/telemetry.jsonl`;
		await assert.rejects(() => loadStore(traversalPath), /parent traversal/);

		const rootLink = join(container, "root-link");
		await symlink(root, rootLink);
		const originalDirectory = process.env.PI_CODING_AGENT_DIR;
		process.env.PI_CODING_AGENT_DIR = rootLink;
		try {
			await assert.rejects(() => loadStore(), /symlink/);
		} finally {
			if (originalDirectory === undefined) {
				delete process.env.PI_CODING_AGENT_DIR;
			} else {
				process.env.PI_CODING_AGENT_DIR = originalDirectory;
			}
		}
	});
});

test("telemetry lifecycle append path failures preserve runtime state", async (t) => {
	await withTelemetryRoot(async ({ container, root }) => {
		const operations = ["parent settle", "completed settle", "failed settle"] as const;
		const pathKinds = ["outside root", "symlink"] as const;

		for (const operation of operations) {
			for (const pathKind of pathKinds) {
				await t.test(`${operation} rejects the ${pathKind} store path`, async () => {
					const caseName = `${operation.replaceAll(" ", "-")}-${pathKind.replaceAll(" ", "-")}`;
					const outsideDirectory = join(container, caseName);
					await mkdir(outsideDirectory);
					const outsideStorePath = join(outsideDirectory, "telemetry.jsonl");
					await writeFile(outsideStorePath, "outside sentinel\n");

					let storePath = outsideStorePath;
					if (pathKind === "symlink") {
						storePath = join(root, `${caseName}.jsonl`);
						await symlink(outsideStorePath, storePath);
					}

					const runtime = createTelemetryRuntime({ path: storePath, records: [] });
					const lifecycle = createFakeLifecycleApi();
					registerLifecycle(lifecycle.api, runtime);
					const context = { ui: { setStatus() {} } };
					let completion: () => Promise<unknown>;

					if (operation === "parent settle") {
						await invoke(lifecycle.handler("on:agent_start"), undefined, context);
						completion = () => invoke(lifecycle.handler("on:agent_settled"), undefined, context);
					} else if (operation === "completed settle") {
						await invoke(lifecycle.handler("events:subagents:started"), { id: caseName, type: "security-agent" });
						completion = () => invoke(lifecycle.handler("events:subagents:completed"), { id: caseName, status: "completed", durationMs: 0 });
					} else {
						await invoke(lifecycle.handler("events:subagents:started"), { id: caseName, type: "security-agent" });
						completion = () => invoke(lifecycle.handler("events:subagents:failed"), { id: caseName, status: "error", durationMs: 0 });
					}

					const snapshot = await snapshotRuntimeState(runtime);
					const matcher = pathKind === "symlink" ? /symlink/ : /configured root/;
					await assertRejectedRuntimeOperation(completion, runtime, snapshot, matcher);
				});
			}
		}
	});
});
