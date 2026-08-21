// NOTE: AGNT-0028.T05: no in-repo non-interactive pi workflowScript harness exists here, so this is the documented fallback proxy with two concurrent refresh(ctx, true) calls and a mid-fetch stale-ctx transition (no real pi spawn).
import * as assert from "node:assert/strict";
import { test } from "node:test";
import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent";

import statusline from "./statusline.ts";

type Deferred = {
	promise: Promise<void>;
	resolve: () => void;
};

function deferred(): Deferred {
	let resolve!: () => void;
	const promise = new Promise<void>((value) => {
		resolve = value;
	});
	return { promise, resolve };
}

function waitFor(predicate: () => boolean, timeoutMs = 1000, pollMs = 5): Promise<void> {
	const startedAt = Date.now();
	return new Promise((resolve, reject) => {
		const check = () => {
			try {
				if (predicate()) return resolve();
			} catch (error) {
				return reject(error);
			}
			if (Date.now() - startedAt > timeoutMs) return reject(new Error("waitFor timed out"));
			setTimeout(check, pollMs);
		};
		setTimeout(check, 0);
	});
}

function createFakePi() {
	const handlers = new Map<string, (event: unknown, ctx: ExtensionContext) => unknown>();
	const api = {
		on(event: string, handler: (event: unknown, ctx: ExtensionContext) => unknown) {
			handlers.set(event, handler);
		},
	} as unknown as ExtensionAPI;
	return { api, handlers };
}

function createMockContext() {
	let isActive = true;
	const setStatusCalls: Array<{ key: string; text: string | undefined }> = [];
	// TODO(AGNT-0028.T05): walk W3/D-02 -- the real ExtensionContext has
	// no assertActive() method. Remove this mock method. Make hasUI
	// below throw when !isActive instead (matching the real per-getter
	// guard shape) rather than always returning true unconditionally.
	const ctx = {
		assertActive() {
			if (!isActive) throw new Error("This extension ctx is stale");
		},
		get hasUI() {
			return true;
		},
		ui: {
			setStatus(key: string, text: string | undefined) {
				setStatusCalls.push({ key, text });
			},
			theme: {
				fg(_mode: string, text: string) {
					return text;
				},
			},
		},
		model: {
			provider: "anthropic",
		},
		modelRegistry: {
			getProviderAuth() {
				return Promise.resolve({ auth: { apiKey: "sk-ant-oat-fallback-token" } });
			},
		},
	} as unknown as ExtensionContext;

	return {
		ctx,
		setStatusCalls,
		markStale() {
			isActive = false;
		},
	};
}

test("TC-06 fallback: two concurrent refresh(true) calls with mid-fetch stale ctx must stay clean", async () => {
	const { api, handlers } = createFakePi();
	statusline(api);
	const sessionStart = handlers.get("session_start");
	assert.ok(sessionStart, "session_start handler must register");

	const context = createMockContext();
	const unhandledRejections: unknown[] = [];
	const onUnhandledRejection = (reason: unknown) => {
		unhandledRejections.push(reason);
	};
	process.on("unhandledRejection", onUnhandledRejection);

	const originalSetInterval = globalThis.setInterval;
	const originalFetch = globalThis.fetch;
	let intervalCallback: (() => void) | undefined;
	let fetchCalls = 0;
	const fetchBlocks = [deferred(), deferred(), deferred()];
	const fetchReturns = [deferred(), deferred(), deferred()];

	try {
		globalThis.setInterval = ((callback: (..._args: never[]) => void): ReturnType<typeof setInterval> => {
			intervalCallback = () => callback();
			return 0 as ReturnType<typeof setInterval>;
		}) as typeof setInterval;

		globalThis.fetch = (async (_input: RequestInfo | URL, _init?: RequestInit) => {
			const callIndex = ++fetchCalls;
			assert.ok(callIndex <= 3, `unexpected fetch call #${callIndex}`);
			if (callIndex === 2) {
				// Let both refresh calls begin before the context becomes stale.
				queueMicrotask(() => {
					context.markStale();
				});
			}
			await fetchBlocks[callIndex - 1].promise;
			fetchReturns[callIndex - 1].resolve();

			return new Response(
				JSON.stringify({
					five_hour: { utilization: 12, resets_at: "2026-01-01T00:00:00.000Z" },
					seven_day: { utilization: 34, resets_at: "2026-01-01T00:00:00.000Z" },
				}),
				{
					status: 200,
					headers: { "Content-Type": "application/json" },
				},
			);
		}) as typeof fetch;

		await sessionStart({}, context.ctx);
		await waitFor(() => fetchCalls === 1);
		fetchBlocks[0].resolve();
		await waitFor(() => fetchCalls === 1 && context.setStatusCalls.length >= 1);
		await waitFor(() => intervalCallback !== undefined);

		const statusCountAfterStartup = context.setStatusCalls.length;
		assert.doesNotThrow(() => {
			assert.ok(intervalCallback, "expected interval callback captured");
			intervalCallback?.();
			intervalCallback?.();
		}, "refresh callbacks must not throw synchronously");

		await waitFor(() => fetchCalls === 3);
		fetchBlocks[1].resolve();
		fetchBlocks[2].resolve();
		await Promise.all([fetchReturns[1].promise, fetchReturns[2].promise]);

		await new Promise((resolve) => setTimeout(resolve, 10));
		assert.equal(context.setStatusCalls.length, statusCountAfterStartup);
	} finally {
		globalThis.setInterval = originalSetInterval;
		globalThis.fetch = originalFetch;
		process.off("unhandledRejection", onUnhandledRejection);
	}

	assert.equal(fetchCalls, 3);
	assert.equal(unhandledRejections.length, 0);
});
