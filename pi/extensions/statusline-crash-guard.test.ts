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
		registerTool() {},
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

function createFaultyActiveContext(): ExtensionContext {
	return {
		get hasUI() {
			return true;
		},
		modelRegistry: {
			async getProviderAuth(provider: string) {
				return provider === "anthropic" ? { auth: { apiKey: "sk-ant-oat-fault" } } : null;
			},
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

test("GH-75: a provider refresh error never escapes as an unhandled rejection", async () => {
	const api = createFakeExtensionAPI();
	const ctx = createFaultyActiveContext();
	statusline(api.api);

	const originalFetch = globalThis.fetch;
	globalThis.fetch = async () => {
		throw new Error("boom: a provider refresh fault, not a stale session");
	};
	const originalConsoleError = console.error;
	const loggedErrors: unknown[][] = [];
	console.error = (...args: unknown[]) => {
		loggedErrors.push(args);
	};

	const { rejections } = await captureUnhandled(async () => {
		await api.handler("session_start")({}, ctx);
		await sleep(0);
	});
	await api.handler("session_shutdown")({}, ctx);

	globalThis.fetch = originalFetch;
	console.error = originalConsoleError;

	assert.equal(rejections.length, 0, "the provider fault must be swallowed, not surfaced as an unhandled rejection");
	assert.ok(
		loggedErrors.some((args) => args[0] === "[statusline] anthropic refresh failed:"),
		"the provider-level guard logs the failed provider",
	);
});

test("GH-75: the same provider fault from model_select and agent_settled never escapes", async () => {
	const api = createFakeExtensionAPI();
	const ctx = createFaultyActiveContext();
	statusline(api.api);

	const originalFetch = globalThis.fetch;
	globalThis.fetch = async () => {
		throw new Error("boom: a provider refresh fault, not a stale session");
	};
	const originalConsoleError = console.error;
	console.error = () => {};

	const { rejections } = await captureUnhandled(async () => {
		await api.handler("model_select")({}, ctx);
		await api.handler("agent_settled")({}, ctx);
		await sleep(0);
	});

	globalThis.fetch = originalFetch;
	console.error = originalConsoleError;
	assert.equal(rejections.length, 0);
});
