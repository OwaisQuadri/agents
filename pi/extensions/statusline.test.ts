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

function extractBarWidth(line: string): number {
	const match = line.match(/[█░│]+/);
	assert.ok(match);
	return match[0].length;
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

test("TC-05 Anthropic bar uses half the viewport width", async () => {
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
	await waitForStatusCount(context.recording, 1);

	restoreFetch();
	restoreColumns();

	assert.equal(isCalled, true);
	assert.equal(context.recording.statuses.length, 1);
	const status = context.recording.statuses.at(-1);
	assert.equal(status?.key, "statusline");
	assert.equal(typeof status?.text, "string");
	assert.equal(extractBarWidth(status?.text ?? ""), 50);
});

test("TC-06 Codex primary window renders below 90 percent at half the viewport width", async () => {
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
	await waitForStatusCount(context.recording, 1);

	restoreFetch();
	restoreColumns();

	const status = context.recording.statuses.at(-1);
	assert.equal(status?.key, "statusline");
	assert.match(status?.text ?? "", /^5h /);
	assert.equal(extractPercent(status?.text ?? ""), 30);
	assert.equal(extractBarWidth(status?.text ?? ""), 50);
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

test("TC-08 terminal resize updates the bar to half the new viewport width", async () => {
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
	await waitForStatusCount(context.recording, 1);
	assert.equal(extractBarWidth(context.recording.statuses.at(-1)?.text ?? ""), 40);

	Object.defineProperty(process.stdout, "columns", { configurable: true, value: 120 });
	process.stdout.emit("resize");
	await waitForStatusCount(context.recording, 2);
	assert.equal(extractBarWidth(context.recording.statuses.at(-1)?.text ?? ""), 60);

	await invoke(api.handler("session_shutdown"), context.ctx);
	restoreFetch();
	restoreColumns();
});

type ToolResult = {
	content: Array<{ type: string; text: string }>;
	details: unknown;
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

function createQuotaAdmissionTool(): RegisteredTool {
	let tool: RegisteredTool | undefined;
	const pi = {
		registerTool(value: RegisteredTool) {
			tool = value;
		},
		on() {},
	} as unknown as ExtensionAPI;

	statusline(pi);
	assert.ok(tool, "quota_admission must register at startup");
	assert.equal(tool.name, "quota_admission");
	return tool;
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
	const context = createMockContext();
	context.setProviderAuth("anthropic", "sk-ant-oat-test-token");
	context.setProviderAuth("openai-codex", makeOpenAICodexToken("account-test-id"));
	return createQuotaAdmissionTool().execute("call", {}, undefined, undefined, context.ctx);
}

function quotaAdmissionDetails(result: ToolResult): {
	isAdmitted: boolean;
	providers: Record<Provider, { isFresh: boolean; usedPercent: number | null; pacePercent: number | null; reset: string | null; isEligible: boolean }>;
} {
	return result.details as {
		isAdmitted: boolean;
		providers: Record<Provider, { isFresh: boolean; usedPercent: number | null; pacePercent: number | null; reset: string | null; isEligible: boolean }>;
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

test("quota_admission fails closed when one provider fetch is missing", async () => {
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
		});
	}
	assert.equal(admission.isAdmitted, false);
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
