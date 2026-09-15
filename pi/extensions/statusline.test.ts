import assert from "node:assert/strict";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
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
		registerTool() {},
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

function installTerminalColumns(columns: number): () => void {
	const descriptor = Object.getOwnPropertyDescriptor(process.stdout, "columns");
	Object.defineProperty(process.stdout, "columns", { configurable: true, value: columns });
	return () => {
		if (descriptor) Object.defineProperty(process.stdout, "columns", descriptor);
		else delete (process.stdout as { columns?: number }).columns;
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

type QuotaDisplay = {
	provider: Provider;
	usedPercent: number;
};

type QuotaState = Partial<Record<Provider, QuotaDisplay>>;

function currentQuotaState(): QuotaState | undefined {
	return (globalThis as { __owaisQuotaState?: QuotaState }).__owaisQuotaState;
}

function clearQuotaState(): void {
	(globalThis as { __owaisQuotaState?: QuotaState }).__owaisQuotaState = undefined;
}

async function waitForQuotaState(provider: Provider, previous?: QuotaDisplay): Promise<QuotaDisplay> {
	const timeout = Date.now() + 100;
	while (!currentQuotaState()?.[provider] || currentQuotaState()?.[provider] === previous) {
		if (Date.now() > timeout) throw new Error(`timed out waiting for ${provider} quota state`);
		await sleep(0);
	}
	return currentQuotaState()?.[provider] as QuotaDisplay;
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

	assert.equal(context.hasUIReads, 1);
	assert.equal(context.modelReads, 0);
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

	assert.equal(context.hasUIReads, 1);
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
	assert.ok(context.hasUIReads >= 1);
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

test("TC-05 Anthropic usage updates shared quota state", async () => {
	clearQuotaState();
	const restoreColumns = installTerminalColumns(100);
	const api = createFakeExtensionAPI();
	const context = createMockContext({ provider: "anthropic" });
	context.setProviderAuth("anthropic", "sk-ant-oat-usage-token");
	let releaseFetch: () => void = () => {};
	const fetchGate = new Promise<void>((resolve) => {
		releaseFetch = () => resolve();
	});
	let isCalled = false;

	const restoreFetch = installMockFetch(async () => {
		isCalled = true;
		await fetchGate;
		return jsonResponse({
			five_hour: { utilization: 90, resets_at: toIsoOffset(3000) },
			seven_day: { utilization: 10, resets_at: toIsoOffset(86400) },
		});
	});

	statusline(api.api);
	await invoke(api.handler("model_select"), context.ctx);
	releaseFetch();
	const quota = await waitForQuotaState("anthropic");

	restoreFetch();
	restoreColumns();

	assert.equal(isCalled, true);
	assert.equal(quota.usedPercent, 90);
});

test("TC-06 Codex primary window updates shared quota state", async () => {
	clearQuotaState();
	const restoreColumns = installTerminalColumns(100);
	const api = createFakeExtensionAPI();
	const context = createMockContext({ provider: "openai-codex" });
	context.setProviderAuth("openai-codex", makeOpenAICodexToken("account-openai-1"));
	const restoreFetch = installMockFetch(async () => {
		const now = Math.floor(Date.now() / 1000);
		return jsonResponse({
			rate_limit: {
				primary_window: {
					used_percent: 30,
					limit_window_seconds: 18000,
					reset_at: now + 1800,
				},
			},
		});
	});

	statusline(api.api);
	await invoke(api.handler("model_select"), context.ctx);
	const quota = await waitForQuotaState("openai-codex");

	restoreFetch();
	restoreColumns();

	assert.equal(quota.usedPercent, 30);
});

test("provider usage requests carry timeout signals", async () => {
	clearQuotaState();
	const api = createFakeExtensionAPI();
	const context = createMockContext();
	context.setProviderAuth("anthropic", "sk-ant-oat-timeout");
	context.setProviderAuth("openai-codex", makeOpenAICodexToken("account-timeout"));
	const signals: Array<AbortSignal | null | undefined> = [];
	const restoreFetch = installMockFetch(async (input, init) => {
		signals.push(init?.signal);
		return requestUrl(input).includes("api/oauth/usage")
			? jsonResponse({
					five_hour: { utilization: 10, resets_at: toIsoOffset(3600) },
					seven_day: null,
				})
			: jsonResponse(codexUsage(20, 7200));
	});

	statusline(api.api);
	await invoke(api.handler("model_select"), context.ctx);
	await Promise.all([waitForQuotaState("anthropic"), waitForQuotaState("openai-codex")]);

	restoreFetch();

	assert.equal(signals.length, 2);
	assert.ok(signals.every((signal) => signal instanceof AbortSignal));
});

test("TC-07 provider switch retains both provider usage displays", async () => {
	clearQuotaState();
	const api = createFakeExtensionAPI();
	const context = createMockContext({ provider: "anthropic" });
	context.setProviderAuth("anthropic", "sk-ant-oat-session");
	context.setProviderAuth("openai-codex", makeOpenAICodexToken("account-openai-2"));
	const restoreFetch = installMockFetch(async (input: RequestInfo | URL) =>
		requestUrl(input).includes("api/oauth/usage")
			? jsonResponse({
					five_hour: { utilization: 95, resets_at: toIsoOffset(600) },
					seven_day: { utilization: 20, resets_at: toIsoOffset(86400) },
				})
			: jsonResponse(codexUsage(22, 7_000)),
	);

	statusline(api.api);
	const handler = api.handler("model_select");
	await invoke(handler, context.ctx);
	const anthropicQuota = await waitForQuotaState("anthropic");
	const openaiQuota = await waitForQuotaState("openai-codex");
	context.setProvider("openai-codex");
	await invoke(handler, context.ctx);
	await sleep(0);

	restoreFetch();

	assert.equal(anthropicQuota.usedPercent, 95);
	assert.equal(openaiQuota.usedPercent, 22);
	assert.equal(currentQuotaState()?.anthropic?.usedPercent, 95);
	assert.equal(currentQuotaState()?.["openai-codex"]?.usedPercent, 22);
});

test("TC-08 one provider refresh failure keeps its last display and updates the other", async () => {
	clearQuotaState();
	const api = createFakeExtensionAPI();
	const context = createMockContext({ provider: "anthropic" });
	context.setProviderAuth("anthropic", "sk-ant-oat-refresh-failure");
	context.setProviderAuth("openai-codex", makeOpenAICodexToken("account-refresh-failure"));
	const interval = installFakeSetInterval();
	const calls = new Map<string, number>();
	const restoreFetch = installMockFetch(async (input) => {
		const url = requestUrl(input);
		const provider = url.includes("api/oauth/usage") ? "anthropic" : "openai-codex";
		const count = (calls.get(provider) ?? 0) + 1;
		calls.set(provider, count);
		if (provider === "anthropic") {
			return count === 1 ? jsonResponse(anthropicUsage(50, 3_600)) : { ok: false, json: async () => ({}) };
		}
		return jsonResponse(codexUsage(count === 1 ? 30 : 44, 1_800));
	});

	statusline(api.api);
	await invoke(api.handler("session_start"), context.ctx);
	const firstAnthropic = await waitForQuotaState("anthropic");
	const firstOpenAI = await waitForQuotaState("openai-codex");
	interval.callback?.();
	const secondOpenAI = await waitForQuotaState("openai-codex", firstOpenAI);

	assert.equal(firstAnthropic.usedPercent, 50);
	assert.equal(secondOpenAI.usedPercent, 44);
	assert.equal(currentQuotaState()?.anthropic?.usedPercent, 50);

	await invoke(api.handler("session_shutdown"), context.ctx);
	interval.restore();
	restoreFetch();
});

test("TC-09 failed refreshes remain throttled", async () => {
	clearQuotaState();
	const api = createFakeExtensionAPI();
	const context = createMockContext({ provider: "anthropic" });
	context.setProviderAuth("anthropic", "sk-ant-oat-throttle");
	context.setProviderAuth("openai-codex", makeOpenAICodexToken("account-throttle"));
	let fetchCalls = 0;
	const restoreFetch = installMockFetch(async () => {
		fetchCalls += 1;
		return { ok: false, json: async () => ({}) };
	});

	statusline(api.api);
	for (let index = 0; index < 5; index += 1) {
		await invoke(api.handler("agent_settled"), context.ctx);
	}
	await sleep(0);
	await sleep(0);

	restoreFetch();
	assert.equal(fetchCalls, 2);
});

test("TC-10 a late older response cannot replace newer provider usage", async () => {
	clearQuotaState();
	const api = createFakeExtensionAPI();
	const context = createMockContext({ provider: "anthropic" });
	context.setProviderAuth("anthropic", "sk-ant-oat-generation");
	const interval = installFakeSetInterval();
	let releaseFirst: () => void = () => {};
	const firstGate = new Promise<void>((resolve) => {
		releaseFirst = resolve;
	});
	let fetchCalls = 0;
	const restoreFetch = installMockFetch(async () => {
		fetchCalls += 1;
		if (fetchCalls === 1) {
			await firstGate;
			return jsonResponse(anthropicUsage(11, 3_600));
		}
		return jsonResponse(anthropicUsage(99, 3_600));
	});

	statusline(api.api);
	await invoke(api.handler("session_start"), context.ctx);
	interval.callback?.();
	const newest = await waitForQuotaState("anthropic");
	releaseFirst();
	await sleep(0);
	await sleep(0);

	assert.equal(newest.usedPercent, 99);
	assert.equal(currentQuotaState()?.anthropic?.usedPercent, 99);
	await invoke(api.handler("session_shutdown"), context.ctx);
	interval.restore();
	restoreFetch();
});

test("TC-11 one hung provider does not withhold the other provider display", async () => {
	clearQuotaState();
	const api = createFakeExtensionAPI();
	const context = createMockContext({ provider: "anthropic" });
	context.setProviderAuth("anthropic", "sk-ant-oat-hung");
	context.setProviderAuth("openai-codex", makeOpenAICodexToken("account-hung"));
	const interval = installFakeSetInterval();
	const restoreFetch = installMockFetch(async (input) => {
		if (requestUrl(input).includes("api/oauth/usage")) return await new Promise(() => {});
		return jsonResponse(codexUsage(30, 1_800));
	});

	statusline(api.api);
	await invoke(api.handler("session_start"), context.ctx);
	const openai = await waitForQuotaState("openai-codex");

	assert.equal(openai.usedPercent, 30);
	assert.equal(currentQuotaState()?.anthropic, undefined);
	await invoke(api.handler("session_shutdown"), context.ctx);
	interval.restore();
	restoreFetch();
});

test("TC-12 an old session response cannot match a new session generation", async () => {
	clearQuotaState();
	const api = createFakeExtensionAPI();
	const context = createMockContext({ provider: "anthropic" });
	context.setProviderAuth("anthropic", "sk-ant-oat-session-generation");
	const interval = installFakeSetInterval();
	let releaseFirst: () => void = () => {};
	const firstGate = new Promise<void>((resolve) => {
		releaseFirst = resolve;
	});
	let fetchCalls = 0;
	const restoreFetch = installMockFetch(async () => {
		fetchCalls += 1;
		if (fetchCalls === 1) {
			await firstGate;
			return jsonResponse(anthropicUsage(11, 3_600));
		}
		return jsonResponse(anthropicUsage(99, 3_600));
	});

	statusline(api.api);
	await invoke(api.handler("session_start"), context.ctx);
	await invoke(api.handler("session_start"), context.ctx);
	const newest = await waitForQuotaState("anthropic");
	releaseFirst();
	await sleep(0);
	await sleep(0);

	assert.equal(newest.usedPercent, 99);
	assert.equal(currentQuotaState()?.anthropic?.usedPercent, 99);
	await invoke(api.handler("session_shutdown"), context.ctx);
	interval.restore();
	restoreFetch();
});

test("TC-13 session shutdown rejects an in-flight response", async () => {
	clearQuotaState();
	const api = createFakeExtensionAPI();
	const context = createMockContext({ provider: "anthropic" });
	context.setProviderAuth("anthropic", "sk-ant-oat-shutdown");
	const interval = installFakeSetInterval();
	let releaseFetch: () => void = () => {};
	const fetchGate = new Promise<void>((resolve) => {
		releaseFetch = resolve;
	});
	const restoreFetch = installMockFetch(async () => {
		await fetchGate;
		return jsonResponse(anthropicUsage(70, 3_600));
	});

	statusline(api.api);
	await invoke(api.handler("session_start"), context.ctx);
	await invoke(api.handler("session_shutdown"), context.ctx);
	releaseFetch();
	await sleep(0);
	await sleep(0);

	assert.equal(currentQuotaState(), undefined);
	interval.restore();
	restoreFetch();
});

test("TC-14 terminal resize rerenders shared quota state", async () => {
	clearQuotaState();
	const restoreColumns = installTerminalColumns(80);
	const api = createFakeExtensionAPI();
	const context = createMockContext({ provider: "anthropic" });
	context.setProviderAuth("anthropic", "sk-ant-oat-resize-token");
	const restoreFetch = installMockFetch(async () =>
		jsonResponse({
			five_hour: { utilization: 95, resets_at: toIsoOffset(600) },
			seven_day: { utilization: 20, resets_at: toIsoOffset(86400) },
		}),
	);

	statusline(api.api);
	await invoke(api.handler("session_start"), context.ctx);
	const beforeResize = await waitForQuotaState("anthropic");

	Object.defineProperty(process.stdout, "columns", { configurable: true, value: 120 });
	process.stdout.emit("resize");
	const afterResize = await waitForQuotaState("anthropic", beforeResize);
	assert.deepEqual(afterResize, beforeResize);

	await invoke(api.handler("session_shutdown"), context.ctx);
	restoreFetch();
	restoreColumns();
});

type ToolResult = {
	content: Array<{ type: string; text: string }>;
	details: unknown;
};

type ProviderAdmissionDetails = {
	isFresh: boolean;
	usedPercent: number | null;
	pacePercent: number | null;
	reset: string | null;
	isEligible: boolean;
	isOverridden: boolean;
};

type RegisteredTool = {
	name: string;
	execute: (
		toolCallId: string,
		params: Record<string, never>,
		signal: AbortSignal | undefined,
		onUpdate: undefined,
		ctx: ExtensionContext,
	) => Promise<ToolResult>;
};

function createQuotaAdmissionTool(settingsPath?: string): RegisteredTool {
	let tool: RegisteredTool | undefined;
	const pi = {
		registerTool(value: RegisteredTool) {
			tool = value;
		},
		on() {},
	} as unknown as ExtensionAPI;

	statusline(pi, { settingsPath });
	assert.ok(tool, "quota_admission must register at startup");
	assert.equal(tool.name, "quota_admission");
	return tool;
}

function withSettings(body: unknown, run: (settingsPath: string) => Promise<void>): Promise<void> {
	const directory = mkdtempSync(join(tmpdir(), "quota-admission-test-"));
	const settingsPath = join(directory, "settings.json");
	if (typeof body === "string") writeFileSync(settingsPath, body, "utf8");
	else if (body !== undefined) writeFileSync(settingsPath, JSON.stringify(body), "utf8");
	return run(settingsPath).finally(() => rmSync(directory, { recursive: true, force: true }));
}

function anthropicUsage(usedPercent: number, resetOffsetSeconds: number) {
	return {
		five_hour: { utilization: usedPercent, resets_at: toIsoOffset(resetOffsetSeconds) },
		seven_day: null,
	};
}

function codexUsage(usedPercent: number, resetOffsetSeconds: number) {
	return {
		rate_limit: {
			primary_window: {
				used_percent: usedPercent,
				limit_window_seconds: 18_000,
				reset_after_seconds: resetOffsetSeconds,
			},
		},
	};
}

async function executeQuotaAdmission(): Promise<ToolResult> {
	return withSettings(undefined, async (settingsPath) => {
		const context = createMockContext();
		context.setProviderAuth("anthropic", "sk-ant-oat-test-token");
		context.setProviderAuth("openai-codex", makeOpenAICodexToken("account-test-id"));
		return createQuotaAdmissionTool(settingsPath).execute("call", {}, undefined, undefined, context.ctx);
	});
}

test("quota_admission cannot overwrite a newer footer refresh", async () => {
	clearQuotaState();
	const handlers = new Map<string, Handler>();
	let quotaTool: RegisteredTool | undefined;
	const api = {
		on(event: string, handler: Handler) {
			handlers.set(event, handler);
		},
		registerTool(value: RegisteredTool) {
			quotaTool = value;
		},
	} as unknown as ExtensionAPI;
	const context = createMockContext({ provider: "anthropic" });
	context.setProviderAuth("anthropic", "sk-ant-oat-tool-race");
	let releaseToolFetch: () => void = () => {};
	const toolGate = new Promise<void>((resolve) => {
		releaseToolFetch = resolve;
	});
	let fetchCalls = 0;
	const restoreFetch = installMockFetch(async () => {
		fetchCalls += 1;
		if (fetchCalls === 1) {
			await toolGate;
			return jsonResponse(anthropicUsage(10, 3_600));
		}
		return jsonResponse(anthropicUsage(90, 3_600));
	});

	statusline(api);
	assert.ok(quotaTool);
	const toolResult = quotaTool.execute("call", {}, undefined, undefined, context.ctx);
	await sleep(0);
	await invoke(handlers.get("model_select") as Handler, context.ctx);
	const newest = await waitForQuotaState("anthropic");
	releaseToolFetch();
	await toolResult;
	await sleep(0);

	assert.equal(newest.usedPercent, 90);
	assert.equal(currentQuotaState()?.anthropic?.usedPercent, 90);
	restoreFetch();
});

test("failed quota_admission does not discard an in-flight successful refresh", async () => {
	clearQuotaState();
	const handlers = new Map<string, Handler>();
	let quotaTool: RegisteredTool | undefined;
	const api = {
		on(event: string, handler: Handler) {
			handlers.set(event, handler);
		},
		registerTool(value: RegisteredTool) {
			quotaTool = value;
		},
	} as unknown as ExtensionAPI;
	const context = createMockContext({ provider: "anthropic" });
	context.setProviderAuth("anthropic", "sk-ant-oat-tool-failure-race");
	const interval = installFakeSetInterval();
	let releaseRefresh: () => void = () => {};
	const refreshGate = new Promise<void>((resolve) => {
		releaseRefresh = resolve;
	});
	let fetchCalls = 0;
	const restoreFetch = installMockFetch(async () => {
		fetchCalls += 1;
		if (fetchCalls === 1) {
			await refreshGate;
			return jsonResponse(anthropicUsage(55, 3_600));
		}
		return { ok: false, json: async () => ({}) };
	});

	statusline(api);
	assert.ok(quotaTool);
	await invoke(handlers.get("session_start") as Handler, context.ctx);
	await sleep(0);
	await quotaTool.execute("call", {}, undefined, undefined, context.ctx);
	releaseRefresh();
	const refreshed = await waitForQuotaState("anthropic");

	assert.equal(refreshed.usedPercent, 55);
	await invoke(handlers.get("session_shutdown") as Handler, context.ctx);
	interval.restore();
	restoreFetch();
});

function quotaAdmissionDetails(result: ToolResult): {
	checkedAtEpochSeconds: number;
	isAdmitted: boolean;
	providers: Record<Provider, ProviderAdmissionDetails>;
} {
	return result.details as {
		checkedAtEpochSeconds: number;
		isAdmitted: boolean;
		providers: Record<Provider, ProviderAdmissionDetails>;
	};
}

test("quota_admission admits an eligible plan", async () => {
	let fetchCalls = 0;
	const restoreFetch = installMockFetch(async (input) => {
		fetchCalls += 1;
		return requestUrl(input).includes("api/oauth/usage")
			? jsonResponse(anthropicUsage(50, 3_600))
			: jsonResponse(codexUsage(95, 1_800));
	});

	const result = await executeQuotaAdmission();
	restoreFetch();
	const admission = quotaAdmissionDetails(result as ToolResult);

	assert.equal(fetchCalls, 2);
	assert.ok(Number.isFinite(admission.checkedAtEpochSeconds));
	assert.equal(admission.providers.anthropic.isEligible, true);
	assert.equal(admission.providers["openai-codex"].isEligible, false);
	assert.equal(admission.isAdmitted, true);
});

test("quota_admission admits usage equal to pace", async () => {
	const restoreFetch = installMockFetch(async (input) =>
		requestUrl(input).includes("api/oauth/usage")
			? jsonResponse(anthropicUsage(95, 3_600))
			: jsonResponse(codexUsage(90, 1_800)),
	);

	const result = await executeQuotaAdmission();
	restoreFetch();
	const admission = quotaAdmissionDetails(result as ToolResult);

	assert.equal(admission.providers["openai-codex"].usedPercent, admission.providers["openai-codex"].pacePercent);
	assert.equal(admission.providers["openai-codex"].isEligible, true);
	assert.equal(admission.isAdmitted, true);
});

test("quota_admission rejects plans when both providers are ahead of pace", async () => {
	const restoreFetch = installMockFetch(async (input) =>
		requestUrl(input).includes("api/oauth/usage")
			? jsonResponse(anthropicUsage(95, 3_600))
			: jsonResponse(codexUsage(95, 1_800)),
	);

	const result = await executeQuotaAdmission();
	restoreFetch();
	const admission = quotaAdmissionDetails(result as ToolResult);

	assert.equal(admission.providers.anthropic.isEligible, false);
	assert.equal(admission.providers["openai-codex"].isEligible, false);
	assert.equal(admission.isAdmitted, false);
});

test("quota_admission fails closed when reset timestamps are missing", async () => {
	const restoreFetch = installMockFetch(async (input) =>
		requestUrl(input).includes("api/oauth/usage")
			? jsonResponse({ five_hour: { utilization: 100, resets_at: null }, seven_day: null })
			: jsonResponse({
					rate_limit: {
						primary_window: { used_percent: 100, limit_window_seconds: 18_000 },
					},
				}),
	);

	const result = await executeQuotaAdmission();
	restoreFetch();
	const admission = quotaAdmissionDetails(result as ToolResult);

	for (const provider of ["anthropic", "openai-codex"] as const) {
		assert.equal(admission.providers[provider].isFresh, true);
		assert.equal(admission.providers[provider].pacePercent, null);
		assert.equal(admission.providers[provider].reset, null);
		assert.equal(admission.providers[provider].isEligible, false);
	}
	assert.equal(admission.isAdmitted, false);
});

test("quota_admission keeps a missing provider ineligible when the other admits", async () => {
	const restoreFetch = installMockFetch(async (input) =>
		requestUrl(input).includes("api/oauth/usage")
			? jsonResponse(anthropicUsage(50, 3_600))
			: { ok: false, json: async () => ({}) },
	);

	const result = await executeQuotaAdmission();
	restoreFetch();
	const admission = quotaAdmissionDetails(result as ToolResult);

	assert.equal(admission.providers.anthropic.isFresh, true);
	assert.equal(admission.providers["openai-codex"].isFresh, false);
	assert.equal(admission.providers["openai-codex"].isEligible, false);
	assert.equal(admission.isAdmitted, true);
});

test("quota_admission fails closed when both providers are missing", async () => {
	const restoreFetch = installMockFetch(async () => ({ ok: false, json: async () => ({}) }));

	const result = await executeQuotaAdmission();
	restoreFetch();
	const admission = quotaAdmissionDetails(result as ToolResult);

	for (const provider of ["anthropic", "openai-codex"] as const) {
		assert.deepEqual(admission.providers[provider], {
			isFresh: false,
			usedPercent: null,
			pacePercent: null,
			reset: null,
			isEligible: false,
			isOverridden: false,
		});
	}
	assert.equal(admission.isAdmitted, false);
});

test("quota_admission overrides Anthropic without changing OpenAI Codex measurement", async () => {
	await withSettings({ quotaAdmission: { overrideProvider: "anthropic" } }, async (settingsPath) => {
		const restoreFetch = installMockFetch(async (input) =>
			requestUrl(input).includes("api/oauth/usage")
				? jsonResponse(anthropicUsage(95, 3_600))
				: jsonResponse(codexUsage(95, 1_800)),
		);
		const context = createMockContext();
		context.setProviderAuth("anthropic", "sk-ant-oat-test-token");
		context.setProviderAuth("openai-codex", makeOpenAICodexToken("account-test-id"));

		const result = await createQuotaAdmissionTool(settingsPath).execute("call", {}, undefined, undefined, context.ctx);
		restoreFetch();
		const admission = quotaAdmissionDetails(result);

		assert.equal(admission.providers.anthropic.isEligible, true);
		assert.equal(admission.providers.anthropic.isOverridden, true);
		assert.equal(admission.providers["openai-codex"].isEligible, false);
		assert.equal(admission.providers["openai-codex"].isOverridden, false);
		assert.equal(admission.isAdmitted, true);
	});
});

test("quota_admission overrides OpenAI Codex when usage is unavailable", async () => {
	await withSettings({ quotaAdmission: { overrideProvider: "openai-codex" } }, async (settingsPath) => {
		const restoreFetch = installMockFetch(async () => ({ ok: false, json: async () => ({}) }));
		const context = createMockContext();

		const result = await createQuotaAdmissionTool(settingsPath).execute("call", {}, undefined, undefined, context.ctx);
		restoreFetch();
		const admission = quotaAdmissionDetails(result);

		assert.equal(admission.providers.anthropic.isEligible, false);
		assert.equal(admission.providers.anthropic.isOverridden, false);
		assert.equal(admission.providers["openai-codex"].isEligible, true);
		assert.equal(admission.providers["openai-codex"].isOverridden, true);
		assert.equal(admission.isAdmitted, true);
		assert.equal((globalThis as { __owaisQuotaAdmissionState?: unknown }).__owaisQuotaAdmissionState, result.details);
	});
});

for (const [name, body] of [
	["absent", undefined],
	["unknown", { quotaAdmission: { overrideProvider: "other" } }],
	["malformed", "{ not json"],
] as const) {
	test(`quota_admission ignores ${name} override settings`, async () => {
		await withSettings(body, async (settingsPath) => {
			const restoreFetch = installMockFetch(async () => ({ ok: false, json: async () => ({}) }));
			const context = createMockContext();

			const result = await createQuotaAdmissionTool(settingsPath).execute("call", {}, undefined, undefined, context.ctx);
			restoreFetch();
			const admission = quotaAdmissionDetails(result);

			assert.equal(admission.providers.anthropic.isOverridden, false);
			assert.equal(admission.providers["openai-codex"].isOverridden, false);
			assert.equal(admission.isAdmitted, false);
		});
	});
}

test("quota_admission ignores an unreadable settings path", async () => {
	const directory = mkdtempSync(join(tmpdir(), "quota-admission-directory-"));
	try {
		const restoreFetch = installMockFetch(async () => ({ ok: false, json: async () => ({}) }));
		const context = createMockContext();

		const result = await createQuotaAdmissionTool(directory).execute("call", {}, undefined, undefined, context.ctx);
		restoreFetch();
		const admission = quotaAdmissionDetails(result);

		assert.equal(admission.providers.anthropic.isOverridden, false);
		assert.equal(admission.providers["openai-codex"].isOverridden, false);
		assert.equal(admission.isAdmitted, false);
	} finally {
		rmSync(directory, { recursive: true, force: true });
	}
});

test("quota_admission redacts tokens, account identifiers, and raw responses", async () => {
	const secret = "sensitive-token-account-raw-response";
	const restoreFetch = installMockFetch(async (input) =>
		requestUrl(input).includes("api/oauth/usage")
			? jsonResponse({ ...anthropicUsage(50, 3_600), secret })
			: jsonResponse({ ...codexUsage(95, 1_800), secret }),
	);

	const result = await executeQuotaAdmission();
	restoreFetch();
	const serialized = JSON.stringify(result);

	assert.doesNotMatch(serialized, /sensitive-token-account-raw-response/);
	assert.doesNotMatch(serialized, /sk-ant-oat-test-token/);
	assert.doesNotMatch(serialized, /account-test-id/);
});
