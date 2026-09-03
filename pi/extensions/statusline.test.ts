import assert from "node:assert/strict";
import { test } from "node:test";
import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent";

import statusline from "./statusline.ts";

type Provider = "anthropic" | "openai-codex";

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

function makeOpenAICodexToken(accountId: string): string {
	const payload = Buffer.from(JSON.stringify({ "https://api.openai.com/auth": { chatgpt_account_id: accountId } })).toString(
		"base64url",
	);
	return `header.${payload}.signature`;
}

function createContext(): ExtensionContext {
	const tokens: Record<Provider, string> = {
		anthropic: "sk-ant-oat-test-token",
		"openai-codex": makeOpenAICodexToken("account-test-id"),
	};
	return {
		modelRegistry: {
			async getProviderAuth(provider: string) {
				return { auth: { apiKey: tokens[provider as Provider] } };
			},
		},
	} as ExtensionContext;
}

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

function jsonResponse(payload: unknown, isOk = true) {
	return {
		ok: isOk,
		json: async () => payload,
	};
}

function installMockFetch(
	mock: (input: RequestInfo | URL, init?: RequestInit) => Promise<{ ok: boolean; json(): Promise<unknown> }>,
): () => void {
	const original = globalThis.fetch;
	globalThis.fetch = mock as typeof fetch;
	return () => {
		globalThis.fetch = original;
	};
}

function requestUrl(input: RequestInfo | URL): string {
	if (typeof input === "string") return input;
	if (input instanceof URL) return input.toString();
	return input.url;
}

function anthropicUsage(usedPercent: number, resetOffsetSeconds: number) {
	return {
		five_hour: { utilization: usedPercent, resets_at: new Date(Date.now() + resetOffsetSeconds * 1000).toISOString() },
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

async function execute(): Promise<ToolResult> {
	return createQuotaAdmissionTool().execute("call", {}, undefined, undefined, createContext());
}

function details(result: ToolResult): {
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

	const result = await execute();
	restoreFetch();
	const admission = details(result);

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

	const result = await execute();
	restoreFetch();
	const admission = details(result);

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

	const result = await execute();
	restoreFetch();
	const admission = details(result);

	assert.equal(admission.providers.anthropic.isEligible, false);
	assert.equal(admission.providers["openai-codex"].isEligible, false);
	assert.equal(admission.isAdmitted, false);
});

test("quota_admission fails closed when one provider fetch is missing", async () => {
	const restoreFetch = installMockFetch(async (input) =>
		requestUrl(input).includes("api/oauth/usage")
			? jsonResponse(anthropicUsage(50, 3_600))
			: jsonResponse({}, false),
	);

	const result = await execute();
	restoreFetch();
	const admission = details(result);

	assert.equal(admission.providers.anthropic.isFresh, true);
	assert.equal(admission.providers["openai-codex"].isFresh, false);
	assert.equal(admission.providers["openai-codex"].isEligible, false);
	assert.equal(admission.isAdmitted, true);
});

test("quota_admission fails closed when both providers are missing", async () => {
	const restoreFetch = installMockFetch(async () => jsonResponse({}, false));

	const result = await execute();
	restoreFetch();
	const admission = details(result);

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

	const result = await execute();
	restoreFetch();
	const serialized = JSON.stringify(result);

	assert.doesNotMatch(serialized, /sensitive-token-account-raw-response/);
	assert.doesNotMatch(serialized, /sk-ant-oat-test-token/);
	assert.doesNotMatch(serialized, /account-test-id/);
});
