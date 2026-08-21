import assert from "node:assert/strict";
import { setTimeout as sleep } from "node:timers/promises";
import { test } from "node:test";
import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent";
import statusline from "./statusline.ts";

type Handler = (payload?: unknown, context?: ExtensionContext) => unknown;
type Provider = "anthropic" | "openai-codex";

type StatusUpdate = {
	key: string;
	text: string | undefined;
};

type RecordingUi = {
	statuses: StatusUpdate[];
	ui: {
		setStatus(key: string, text: string | undefined): void;
		notify(message: string, type?: "info" | "warning" | "error"): void;
		theme: {
			fg(mode: string, text: string): string;
		};
	};
};

type MockContext = {
	ctx: ExtensionContext;
	recording: RecordingUi;
	setActive(value: boolean): void;
	setProvider(provider: Provider | null): void;
	setProviderAuth(provider: Provider, token: string | null): void;
	hasUIReads: number;
	modelReads: number;
	providerAuthReads: number;
	assertActiveCalls: number;
};

function createStatusCapture(): RecordingUi {
	const statuses: StatusUpdate[] = [];

	return {
		statuses,
		ui: {
			setStatus(key: string, text: string | undefined) {
				statuses.push({ key, text });
			},
			notify() {},

			theme: {
				fg(_mode: string, text: string) {
					return text;
				},
			},
		},
	};
}

function makeInactiveContextError(): Error {
	const error = new Error("extension context is no longer active");
	(error as { code?: string }).code = "ERR_EXTENSION_CONTEXT_INACTIVE";
	return error;
}

function createMockContext(initial: { provider?: Provider | null; isActive?: boolean; hasUI?: boolean } = {}): MockContext {
	const providerAuth: Partial<Record<Provider, string | null>> = {
		anthropic: null,
		"openai-codex": null,
	};
	let provider: Provider | null = initial.provider ?? "anthropic";
	let isActive = initial.isActive ?? true;
	let hasUIValue = initial.hasUI ?? true;
	let hasUIReads = 0;
	let modelReads = 0;
	let providerAuthReads = 0;
	let assertActiveCalls = 0;
	const recording = createStatusCapture();

	const ctx = {
		ui: recording.ui,
		get hasUI() {
			hasUIReads += 1;
			if (!isActive) {
				throw makeInactiveContextError();
			}
			return hasUIValue;
		},
		get model() {
			modelReads += 1;
			if (!isActive) {
				throw makeInactiveContextError();
			}
			return provider === null ? null : { provider };
		},
		modelRegistry: {
			async getProviderAuth(providerName: string) {
				providerAuthReads += 1;
				const token = providerAuth[providerName as Provider] ?? null;
				return token === null ? null : { auth: { apiKey: token } };
			},
		},
		// TODO(AGNT-0028.T04): walk W3/D-02 -- the real ExtensionContext has
		// no assertActive() method. Remove this mock method and the
		// assertActiveCalls tracking entirely; hasUI above already throws
		// correctly when stale, matching the real per-getter guard shape.
		// Fix the three assertions at lines ~305, ~331, ~375 that check
		// assertActiveCalls to check hasUIReads instead.
		assertActive() {
			assertActiveCalls += 1;
			if (!isActive) {
				throw makeInactiveContextError();
			}
		},
		mode: "tui",
		cwd: "/tmp",
		sessionManager: {} as never,
		scopedModels: [],
		thinkingLevel: undefined,
		isIdle() {
			return true;
		},
		isProjectTrusted() {
			return true;
		},
		signal: undefined,
		abort() {
			return undefined;
		},
		hasPendingMessages() {
			return false;
		},
		shutdown() {
			return undefined;
		},
		getContextUsage() {
			return undefined;
		},
		compact() {
			return undefined;
		},
		getSystemPrompt() {
			return "";
		},
	} as ExtensionContext;

	return {
		ctx,
		recording,
		setActive(value: boolean) {
			isActive = value;
		},
		setProvider(next: Provider | null) {
			provider = next;
		},
		setProviderAuth(providerName: Provider, token: string | null) {
			providerAuth[providerName] = token;
		},
		get hasUIReads() {
			return hasUIReads;
		},
		get modelReads() {
			return modelReads;
		},
		get providerAuthReads() {
			return providerAuthReads;
		},
		get assertActiveCalls() {
			return assertActiveCalls;
		},
	};
}

function createFakeExtensionAPI(): {
	api: ExtensionAPI;
	handler(event: string): Handler;
} {
	const handlers = new Map<string, Handler>();
	const api = {
		on(event: string, handler: Handler) {
			handlers.set(`on:${event}`, handler);
		},
	} as unknown as ExtensionAPI;

	return {
		api,
		handler(event: string): Handler {
			const handler = handlers.get(`on:${event}`);
			assert.ok(handler, `missing handler for ${event}`);
			return handler;
		},
	};
}

async function invoke(handler: Handler, context: ExtensionContext, payload?: unknown): Promise<unknown> {
	return await handler(payload, context);
}

function installMockFetch(
	mock: (input: RequestInfo | URL, init?: RequestInit | undefined) => Promise<{ ok: boolean; json(): Promise<unknown> }>,
): () => void {
	const original = globalThis.fetch;
	globalThis.fetch = mock as typeof fetch;
	return () => {
		globalThis.fetch = original;
	};
}

function installFakeSetInterval(): {
	callback?: () => void;
	restore(): void;
} {
	let callback: (() => void) | undefined;
	const originalSetInterval = globalThis.setInterval;
	const originalClearInterval = globalThis.clearInterval;

	globalThis.setInterval = ((handler: (...args: never[]) => unknown) => {
		callback = () => {
			handler();
		};
		return 1 as unknown as ReturnType<typeof setInterval>;
	}) as typeof setInterval;
	globalThis.clearInterval = (() => {
		return undefined;
	}) as typeof clearInterval;

	return {
		get callback() {
			return callback;
		},
		restore() {
			globalThis.setInterval = originalSetInterval;
			globalThis.clearInterval = originalClearInterval;
		},
	};
}

async function captureUnhandled<T>(run: () => Promise<T>): Promise<{ result: T; rejections: unknown[] }> {
	const rejections: unknown[] = [];
	const listener = (reason: unknown) => {
		rejections.push(reason);
	};
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

async function waitForStatusCount(ui: RecordingUi, expected: number): Promise<void> {
	const timeout = Date.now() + 100;
	while (ui.statuses.length < expected) {
		if (Date.now() > timeout) {
			throw new Error(`timed out waiting for ${expected} status updates`);
		}
		await sleep(0);
	}
}

function jsonResponse(payload: unknown) {
	return {
		ok: true,
		json: async () => payload,
	};
}

function requestUrl(input: RequestInfo | URL): string {
	if (typeof input === "string") return input;
	if (input instanceof URL) return input.toString();
	return (input as Request).url;
}

function toIsoOffset(offsetSeconds: number): string {
	return new Date(Date.now() + offsetSeconds * 1000).toISOString();
}

function makeOpenAICodexToken(accountId: string): string {
	const payload = Buffer.from(JSON.stringify({ "https://api.openai.com/auth": { chatgpt_account_id: accountId } })).toString(
		"base64url",
	);
	return `header.${payload}.signature`;
}

function extractPercent(line: string): number {
	const match = line.match(/(\d+)%/);
	assert.ok(match);
	return Number.parseInt(match[1] ?? "0", 10);
}

test("TC-01 render() no-ops silently on stale ctx", async () => {
	const api = createFakeExtensionAPI();
	const context = createMockContext({ provider: "anthropic", isActive: false });
	let fetchCalls = 0;
	const restoreFetch = installMockFetch(async () => {
		fetchCalls += 1;
		return jsonResponse({});
	});

	statusline(api.api);
	const modelSelect = api.handler("model_select");

	const { rejections } = await captureUnhandled(async () => {
		await invoke(modelSelect, context.ctx);
	});

	restoreFetch();

	assert.equal(context.assertActiveCalls, 1);
	assert.equal(context.modelReads, 0);
	assert.equal(context.hasUIReads, 0);
	assert.equal(context.providerAuthReads, 0);
	assert.equal(fetchCalls, 0);
	assert.deepEqual(context.recording.statuses, []);
	assert.equal(rejections.length, 0);
});

test("TC-02 refresh() no-ops before touching stale ctx's model", async () => {
	const api = createFakeExtensionAPI();
	const context = createMockContext({ provider: "anthropic", isActive: false });
	let fetchCalls = 0;
	const restoreFetch = installMockFetch(async () => {
		fetchCalls += 1;
		return jsonResponse({});
	});

	statusline(api.api);
	const agentSettled = api.handler("agent_settled");
	const { rejections } = await captureUnhandled(async () => {
		await invoke(agentSettled, context.ctx);
	});

	restoreFetch();

	assert.equal(context.assertActiveCalls, 1);
	assert.equal(context.modelReads, 0);
	assert.equal(context.providerAuthReads, 0);
	assert.equal(fetchCalls, 0);
	assert.deepEqual(context.recording.statuses, []);
	assert.equal(rejections.length, 0);
});

test("TC-03 refresh() no-ops when ctx goes stale mid-fetch", async () => {
	const api = createFakeExtensionAPI();
	const context = createMockContext({ provider: "anthropic" });
	context.setProviderAuth("anthropic", "sk-ant-oat-live-token");
	let fetchCalls = 0;
	const restoreFetch = installMockFetch(async (input: RequestInfo | URL) => {
		fetchCalls += 1;
		if (fetchCalls === 1) {
			return jsonResponse({ five_hour: null, seven_day: null });
		}
		context.setActive(false);
		await sleep(0);
		return jsonResponse({
			five_hour: { utilization: 95, resets_at: toIsoOffset(3600) },
			seven_day: { utilization: 40, resets_at: toIsoOffset(86400) },
		});
	});
	const interval = installFakeSetInterval();

	statusline(api.api);
	await invoke(api.handler("session_start"), context.ctx);
	await sleep(0);
	const baselineStatusCount = context.recording.statuses.length;
	assert.ok(interval.callback);

	const { rejections } = await captureUnhandled(async () => {
		interval.callback?.();
		await sleep(0);
		await sleep(0);
	});

	interval.restore();
	restoreFetch();

	assert.equal(fetchCalls, 2);
	assert.equal(context.recording.statuses.length, baselineStatusCount);
	assert.ok(context.assertActiveCalls >= 1);
	assert.equal(rejections.length, 0);
});

test("TC-04 setInterval poll tick no-ops after context invalidation", async () => {
	const api = createFakeExtensionAPI();
	const context = createMockContext({ provider: "openai-codex", isActive: true, hasUI: true });
	let fetchCalls = 0;
	const restoreFetch = installMockFetch(async (input: RequestInfo | URL) => {
		fetchCalls += 1;
		return jsonResponse({ five_hour: null, seven_day: null });
	});
	const interval = installFakeSetInterval();

	statusline(api.api);
	await invoke(api.handler("session_start"), context.ctx);
	await sleep(0);
	const baselineStatusCount = context.recording.statuses.length;
	assert.ok(interval.callback);

	context.setActive(false);
	const { rejections } = await captureUnhandled(async () => {
		interval.callback?.();
		await sleep(0);
		await sleep(0);
	});

	interval.restore();
	restoreFetch();

	assert.equal(fetchCalls, 0);
	assert.equal(context.recording.statuses.length, baselineStatusCount);
	assert.equal(rejections.length, 0);
});

test("TC-05 live ctx still renders usage bar unchanged", async () => {
	const api = createFakeExtensionAPI();
	const context = createMockContext({ provider: "anthropic" });
	context.setProviderAuth("anthropic", "sk-ant-oat-usage-token");
	let releaseFetch: () => void = () => {};
	const fetchGate = new Promise<void>((resolve) => {
		releaseFetch = () => resolve();
	});
	let called = false;

	const restoreFetch = installMockFetch(async () => {
		called = true;
		await fetchGate;
		return jsonResponse({
			five_hour: { utilization: 90, resets_at: toIsoOffset(3000) },
			seven_day: { utilization: 10, resets_at: toIsoOffset(86400) },
		});
	});

	statusline(api.api);
	await invoke(api.handler("model_select"), context.ctx);
	releaseFetch();
	await waitForStatusCount(context.recording, 1);

	restoreFetch();

	assert.equal(called, true);
	assert.equal(context.recording.statuses.length, 1);
	const status = context.recording.statuses.at(-1);
	assert.equal(status?.key, "statusline");
	assert.equal(typeof status?.text, "string");
	assert.notEqual(status?.text, "");
});

test("TC-07 provider switch does not render stale session A usage", async () => {
	const api = createFakeExtensionAPI();
	const sessionA = createMockContext({ provider: "anthropic" });
	const sessionB = createMockContext({ provider: "openai-codex" });

	sessionA.setProviderAuth("anthropic", "sk-ant-oat-session-a");
	sessionB.setProviderAuth("openai-codex", makeOpenAICodexToken("account-openai-2"));

	const restoreFetch = installMockFetch(async (input: RequestInfo | URL) => {
		const url = requestUrl(input);
		if (url.includes("api/oauth/usage")) {
			return jsonResponse({
				five_hour: { utilization: 95, resets_at: toIsoOffset(600) },
				seven_day: { utilization: 20, resets_at: toIsoOffset(86400) },
			});
		}
		if (url.includes("wham/usage")) {
			const now = Math.floor(Date.now() / 1000);
			return jsonResponse({
				rate_limit: {
					primary_window: {
						used_percent: 30,
						limit_window_seconds: 18000,
						reset_at: now + 1800,
						reset_after_seconds: 1800,
					},
					secondary_window: {
						used_percent: 22,
						limit_window_seconds: 604800,
						reset_at: now + 7000,
						reset_after_seconds: 7000,
					},
				},
			});
		}

		throw new Error(`unexpected usage request: ${url}`);
	});

	statusline(api.api);
	const handler = api.handler("model_select");
	await invoke(handler, sessionA.ctx);
	await waitForStatusCount(sessionA.recording, 1);

	const anthropicText = sessionA.recording.statuses.at(-1)?.text;
	assert.equal(typeof anthropicText, "string");

	sessionA.setActive(false);
	await invoke(handler, sessionB.ctx);
	await waitForStatusCount(sessionB.recording, 1);

	restoreFetch();

	const openaiText = sessionB.recording.statuses.at(-1)?.text;
	assert.equal(typeof openaiText, "string");
	assert.notEqual(openaiText, anthropicText);
	assert.notEqual(extractPercent(openaiText), extractPercent(anthropicText));
});
