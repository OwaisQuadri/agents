import assert from "node:assert/strict";
import { test } from "node:test";

import type { HerdrClient } from "./client.ts";
import { HerdrStateController, type HerdrStateWait } from "./controller.ts";
import {
	makeRawPane,
	makeRawTab,
	makeRawWorkspace,
	makeSnapshotResponse,
	SELF_CWD,
	SELF_PANE_ID,
	SELF_TAB_ID,
	SELF_WORKSPACE_ID,
} from "./fixtures.ts";
import type {
	HerdrPaneOutput,
	HerdrSnapshotResponse,
	HerdrStateEvent,
	HerdrStateFailure,
} from "./types.ts";

interface Deferred<T> {
	promise: Promise<T>;
	resolve(value: T): void;
}

function deferred<T>(): Deferred<T> {
	let resolve!: (value: T) => void;
	const promise = new Promise<T>((promiseResolve) => {
		resolve = promiseResolve;
	});
	return { promise, resolve };
}

function waitUntilAborted(signal: AbortSignal): Promise<void> {
	if (signal.aborted) {
		return Promise.resolve();
	}
	return new Promise((resolve) => {
		signal.addEventListener("abort", () => resolve(), { once: true });
	});
}

async function flush(): Promise<void> {
	await new Promise<void>((resolve) => setImmediate(resolve));
}

function makeLocationSnapshot(
	workspaceId: string,
	tabId: string,
): HerdrSnapshotResponse {
	return makeSnapshotResponse({
		focused_workspace_id: workspaceId,
		focused_tab_id: tabId,
		focused_pane_id: SELF_PANE_ID,
		workspaces: [
			makeRawWorkspace({
				workspace_id: workspaceId,
				label: workspaceId,
				focused: true,
			}),
		],
		tabs: [
			makeRawTab({
				tab_id: tabId,
				workspace_id: workspaceId,
				focused: true,
			}),
		],
		panes: [
			makeRawPane({
				pane_id: SELF_PANE_ID,
				workspace_id: workspaceId,
				tab_id: tabId,
				cwd: SELF_CWD,
				focused: true,
			}),
		],
	});
}

function unavailable(message: string): HerdrStateFailure {
	return { code: "unavailable", message };
}

function unusedPaneRead(): Promise<HerdrPaneOutput | HerdrStateFailure> {
	return Promise.resolve(unavailable("unused pane read"));
}

test("TC-16 start stores immutable state and valid events recalculate self", async () => {
	const eventApplied = deferred<void>();
	const eventSignals: AbortSignal[] = [];
	const snapshotSignals: AbortSignal[] = [];
	let snapshotCalls = 0;
	let eventCalls = 0;
	const client: HerdrClient = {
		async snapshot(signal) {
			snapshotCalls += 1;
			snapshotSignals.push(signal!);
			return makeLocationSnapshot(SELF_WORKSPACE_ID, SELF_TAB_ID);
		},
		events(signal) {
			eventCalls += 1;
			eventSignals.push(signal!);
			return (async function* (): AsyncIterable<HerdrStateEvent | HerdrStateFailure> {
				yield {
					type: "pane-changed",
					pane: {
						id: SELF_PANE_ID,
						workspaceId: "w5S",
						tabId: "w5S:t2",
						label: SELF_PANE_ID,
						cwd: SELF_CWD,
						isFocused: true,
					},
				};
				eventApplied.resolve();
				await waitUntilAborted(signal!);
			})();
		},
		readPane: unusedPaneRead,
	};
	const controller = new HerdrStateController(client);

	await controller.start(SELF_CWD, SELF_PANE_ID);
	const initial = controller.current();
	assert.equal(initial?.self?.workspaceId, "w5R");
	await eventApplied.promise;
	const updated = controller.current();

	assert.equal(updated?.self?.workspaceId, "w5S");
	assert.notEqual(updated, initial);
	assert.notEqual(updated?.snapshot, initial?.snapshot);
	assert.equal(
		initial?.snapshot.panes.find((pane) => pane.id === SELF_PANE_ID)?.workspaceId,
		"w5R",
	);
	assert.equal(snapshotCalls, 1);
	assert.equal(eventCalls, 1);
	assert.equal(snapshotSignals[0], eventSignals[0]);

	controller.stop();
	assert.equal(eventSignals[0]?.aborted, true);
	await flush();
	assert.equal(snapshotCalls, 1);
	assert.equal(eventCalls, 1);
});

// TODO(AGNT-0066.T15): Prove one recovery per invalid-response burst and reset after valid data.
test("TC-17 invalid and unavailable results replace state before a dropped stream reconnects", async () => {
	const firstRecoveryFinished = deferred<void>();
	const reconnect = deferred<void>();
	const reconnectWaitStarted = deferred<void>();
	const secondStreamStarted = deferred<void>();
	const snapshotSignals: AbortSignal[] = [];
	const eventSignals: AbortSignal[] = [];
	const snapshots = [
		makeLocationSnapshot("w5R", "w5R:t2"),
		makeLocationSnapshot("w5S", "w5S:t2"),
		makeLocationSnapshot("w5Y", "w5Y:t2"),
	];
	let snapshotCalls = 0;
	let eventCalls = 0;
	let waitCalls = 0;
	const wait: HerdrStateWait = async (signal) => {
		waitCalls += 1;
		reconnectWaitStarted.resolve();
		await Promise.race([reconnect.promise, waitUntilAborted(signal)]);
	};
	const client: HerdrClient = {
		async snapshot(signal) {
			snapshotSignals.push(signal!);
		const response = snapshots[snapshotCalls];
			snapshotCalls += 1;
			assert.notEqual(response, undefined);
			return response!;
		},
		events(signal) {
			eventCalls += 1;
			eventSignals.push(signal!);
			const call = eventCalls;
			return (async function* (): AsyncIterable<HerdrStateEvent | HerdrStateFailure> {
				if (call === 1) {
					yield { code: "invalid-response", message: "replace state" };
					firstRecoveryFinished.resolve();
					yield unavailable("stream dropped");
					return;
				}
				secondStreamStarted.resolve();
				await waitUntilAborted(signal!);
			})();
		},
		readPane: unusedPaneRead,
	};
	const controller = new HerdrStateController(client, wait);

	await controller.start(SELF_CWD, SELF_PANE_ID);
	assert.equal(controller.current()?.self?.workspaceId, "w5R");
	await firstRecoveryFinished.promise;
	assert.equal(controller.current()?.self?.workspaceId, "w5S");
	await reconnectWaitStarted.promise;
	assert.equal(controller.current()?.self?.workspaceId, "w5Y");

	assert.equal(snapshotCalls, 3);
	assert.equal(eventCalls, 1);
	assert.equal(waitCalls, 1);
	reconnect.resolve();
	await secondStreamStarted.promise;
	assert.equal(eventCalls, 2);
	assert.equal(new Set([...snapshotSignals, ...eventSignals]).size, 1);

	controller.stop();
	await flush();
	assert.equal(snapshotCalls, 3);
	assert.equal(eventCalls, 2);
	assert.equal(waitCalls, 1);
});

test("TC-21 stop aborts a blocked event stream without post-stop work", async () => {
	const streamStarted = deferred<void>();
	let snapshotCalls = 0;
	let eventCalls = 0;
	let waitCalls = 0;
	let eventSignal: AbortSignal | undefined;
	const client: HerdrClient = {
		async snapshot() {
			snapshotCalls += 1;
			return makeSnapshotResponse();
		},
		events(signal) {
			eventCalls += 1;
			eventSignal = signal;
			return (async function* (): AsyncIterable<HerdrStateEvent | HerdrStateFailure> {
				streamStarted.resolve();
				await waitUntilAborted(signal!);
			})();
		},
		readPane: unusedPaneRead,
	};
	const controller = new HerdrStateController(client, async () => {
		waitCalls += 1;
	});

	await controller.start(SELF_CWD, SELF_PANE_ID);
	await streamStarted.promise;
	controller.stop();
	await flush();

	assert.equal(eventSignal?.aborted, true);
	assert.equal(snapshotCalls, 1);
	assert.equal(eventCalls, 1);
	assert.equal(waitCalls, 0);
});

test("TC-21 stop aborts a pending initial snapshot and start resolves with null", async () => {
	const snapshotStarted = deferred<AbortSignal>();
	let snapshotCalls = 0;
	let eventCalls = 0;
	const client: HerdrClient = {
		async snapshot(signal) {
			snapshotCalls += 1;
			snapshotStarted.resolve(signal!);
			return await new Promise<HerdrSnapshotResponse | HerdrStateFailure>(() => {});
		},
		events() {
			eventCalls += 1;
			return (async function* (): AsyncIterable<HerdrStateEvent | HerdrStateFailure> {})();
		},
		readPane: unusedPaneRead,
	};
	const controller = new HerdrStateController(client);

	const start = controller.start(SELF_CWD, SELF_PANE_ID);
	const signal = await snapshotStarted.promise;
	controller.stop();
	await start;

	assert.equal(signal.aborted, true);
	assert.equal(controller.current(), null);
	assert.equal(snapshotCalls, 1);
	assert.equal(eventCalls, 0);
});

test("TC-21 stop aborts a pending recovery snapshot without mutation or reconnect", async () => {
	const recoveryStarted = deferred<AbortSignal>();
	const recoveryAborted = deferred<void>();
	let snapshotCalls = 0;
	let eventCalls = 0;
	let waitCalls = 0;
	const client: HerdrClient = {
		async snapshot(signal) {
			snapshotCalls += 1;
			if (snapshotCalls === 1) {
				return makeSnapshotResponse();
			}
			recoveryStarted.resolve(signal!);
			await waitUntilAborted(signal!);
			recoveryAborted.resolve();
			return makeLocationSnapshot("w5Y", "w5Y:t2");
		},
		events() {
			eventCalls += 1;
			return (async function* (): AsyncIterable<HerdrStateEvent | HerdrStateFailure> {
				yield { code: "invalid-response", message: "recover" };
			})();
		},
		readPane: unusedPaneRead,
	};
	const controller = new HerdrStateController(client, async () => {
		waitCalls += 1;
	});

	await controller.start(SELF_CWD, SELF_PANE_ID);
	const beforeStop = controller.current();
	const signal = await recoveryStarted.promise;
	controller.stop();
	await recoveryAborted.promise;
	await flush();

	assert.equal(signal.aborted, true);
	assert.equal(controller.current(), beforeStop);
	assert.equal(controller.current()?.self?.workspaceId, "w5R");
	assert.equal(snapshotCalls, 2);
	assert.equal(eventCalls, 1);
	assert.equal(waitCalls, 0);
});
