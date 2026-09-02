// GH-75: a genuine, non-staleness error inside the refresh()/render() call chain must never
// escape as an unhandled rejection. isCtxActive() only ever catches session staleness (a thrown
// ctx.hasUI read) -- this file pins down the OTHER kind of fault: one thrown while ctx is still
// active, from a call site that isCtxActive never touches (ctx.model, not ctx.hasUI).
import assert from "node:assert/strict";
import { setTimeout as sleep } from "node:timers/promises";
import { test } from "node:test";
import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent";
import statusline from "./statusline.ts";

type Handler = (payload?: unknown, context?: ExtensionContext) => unknown;

function createFakeExtensionAPI(): { api: ExtensionAPI; handler(event: string): Handler } {
	const handlers = new Map<string, Handler>();
	const api = {
		on(event: string, handler: Handler) {
			handlers.set(event, handler);
		},
	} as unknown as ExtensionAPI;
	return {
		api,
		handler(event: string): Handler {
			const found = handlers.get(event);
			assert.ok(found, `missing handler for ${event}`);
			return found;
		},
	};
}

// hasUI stays healthy (ctx is NOT stale) but reading ctx.model throws a genuine, unrelated
// fault -- the exact shape isCtxActive's staleness-only guard was never meant to catch.
function createFaultyActiveContext(): ExtensionContext {
	return {
		get hasUI() {
			return true;
		},
		get model() {
			throw new Error("boom: a real UI/config fault, not a stale session");
		},
		mode: "tui",
	} as unknown as ExtensionContext;
}

async function captureUnhandled<T>(run: () => Promise<T>): Promise<{ result: T; rejections: unknown[] }> {
	const rejections: unknown[] = [];
	const listener = (reason: unknown) => rejections.push(reason);
	process.on("unhandledRejection", listener);
	try {
		const result = await run();
		await sleep(0);
		await sleep(0);
		return { result, rejections };
	} finally {
		process.off("unhandledRejection", listener);
	}
}

test("GH-75: a non-staleness error reading ctx.model inside refresh() never escapes as an unhandled rejection", async () => {
	const api = createFakeExtensionAPI();
	const ctx = createFaultyActiveContext();
	statusline(api.api);

	const originalConsoleError = console.error;
	const loggedErrors: unknown[][] = [];
	console.error = (...args: unknown[]) => {
		loggedErrors.push(args);
	};

	const { rejections } = await captureUnhandled(async () => {
		// session_start calls `void refresh(ctx)` fire-and-forget -- this is the exact
		// call site the issue names.
		await api.handler("session_start")({}, ctx);
		await sleep(0);
	});
	// session_start also arms a real 10-minute setInterval; clear it or the process never exits.
	await api.handler("session_shutdown")({}, ctx);

	console.error = originalConsoleError;

	assert.equal(rejections.length, 0, "the thrown ctx.model fault must be swallowed, not surfaced as an unhandled rejection");
	assert.ok(
		loggedErrors.some((args) => String(args[0]).includes("[statusline]")),
		"a genuine fault is logged, not silently discarded",
	);
});

test("GH-75: the same non-staleness fault reached through model_select and agent_settled also never escapes", async () => {
	const api = createFakeExtensionAPI();
	const ctx = createFaultyActiveContext();
	statusline(api.api);

	const originalConsoleError = console.error;
	console.error = () => {};

	const { rejections } = await captureUnhandled(async () => {
		await api.handler("model_select")({}, ctx);
		await api.handler("agent_settled")({}, ctx);
		await sleep(0);
	});

	console.error = originalConsoleError;
	assert.equal(rejections.length, 0);
});
