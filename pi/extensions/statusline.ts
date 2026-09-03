import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent";

type UsageWindow = {
	usedPercent: number;
	resetAtEpochSeconds: number;
	windowSeconds: number;
};

type ProviderUsage = {
	fiveHour: UsageWindow | null;
	sevenDay: UsageWindow | null;
};

type Provider = "anthropic" | "openai-codex";

type ProviderAdmission = {
	isFresh: boolean;
	usedPercent: number | null;
	pacePercent: number | null;
	reset: string | null;
	isEligible: boolean;
};

type QuotaAdmission = {
	isAdmitted: boolean;
	providers: Record<Provider, ProviderAdmission>;
};

type AnthropicUsageResponse = {
	five_hour: { utilization: number; resets_at: string | null } | null;
	seven_day: { utilization: number; resets_at: string | null } | null;
};

type CodexWindow = {
	used_percent?: number;
	limit_window_seconds?: number;
	reset_at?: number | string;
	reset_after_seconds?: number;
};

type CodexUsageResponse = {
	rate_limit?: {
		primary_window?: CodexWindow | null;
		secondary_window?: CodexWindow | null;
	};
};

const POLL_INTERVAL_MS = 10 * 60 * 1000;
const MIN_FETCH_INTERVAL_MS = 60 * 1000;
const ANTHROPIC_USAGE_URL = "https://api.anthropic.com/api/oauth/usage";
const CODEX_USAGE_URL = "https://chatgpt.com/backend-api/wham/usage";
const FIVE_HOUR_SECONDS = 5 * 3600;
const SEVEN_DAY_SECONDS = 7 * 86400;
const PROVIDERS: Provider[] = ["anthropic", "openai-codex"];
const QUOTA_ADMISSION_PARAMETERS = {
	type: "object",
	additionalProperties: false,
	properties: {},
} as const;

function isCtxActive(ctx: ExtensionContext): boolean {
	try {
		void ctx.hasUI;
		return true;
	} catch {
		return false;
	}
}

function clampPercent(value: number): number {
	if (!Number.isFinite(value) || value < 0) return 0;
	return value > 100 ? 100 : value;
}

function epochSecondsFromIso(iso: string | null): number {
	if (!iso) return 0;
	const ms = new Date(iso).getTime();
	return Number.isFinite(ms) ? Math.round(ms / 1000) : 0;
}

async function getProviderToken(ctx: ExtensionContext, provider: string): Promise<string | null> {
	try {
		const resolved = await ctx.modelRegistry.getProviderAuth(provider);
		return resolved?.auth.apiKey?.trim() || null;
	} catch {
		return null;
	}
}

function codexAccountIdFromToken(token: string): string | null {
	try {
		const parts = token.split(".");
		if (parts.length !== 3) return null;
		const payload = JSON.parse(Buffer.from(parts[1], "base64url").toString("utf8"));
		return payload?.["https://api.openai.com/auth"]?.chatgpt_account_id ?? null;
	} catch {
		return null;
	}
}

async function fetchAnthropicUsage(ctx: ExtensionContext): Promise<ProviderUsage | null> {
	const token = await getProviderToken(ctx, "anthropic");
	// The usage endpoint serves subscription OAuth tokens only; an API key gets 401.
	if (!token || !token.startsWith("sk-ant-oat")) return null;

	const response = await fetch(ANTHROPIC_USAGE_URL, {
		headers: {
			Accept: "application/json",
			Authorization: `Bearer ${token}`,
			"anthropic-beta": "oauth-2025-04-20",
		},
	});
	if (!response.ok) return null;
	const data = (await response.json()) as AnthropicUsageResponse;

	const toWindow = (
		raw: { utilization: number; resets_at: string | null } | null,
		windowSeconds: number,
	): UsageWindow | null =>
		raw
			? {
					usedPercent: clampPercent(raw.utilization),
					resetAtEpochSeconds: epochSecondsFromIso(raw.resets_at),
					windowSeconds,
				}
			: null;

	return {
		fiveHour: toWindow(data.five_hour, FIVE_HOUR_SECONDS),
		sevenDay: toWindow(data.seven_day, SEVEN_DAY_SECONDS),
	};
}

async function fetchCodexUsage(ctx: ExtensionContext): Promise<ProviderUsage | null> {
	const token = await getProviderToken(ctx, "openai-codex");
	if (!token) return null;
	const accountId = codexAccountIdFromToken(token);
	if (!accountId) return null;

	const response = await fetch(CODEX_USAGE_URL, {
		headers: {
			Accept: "application/json",
			Authorization: `Bearer ${token}`,
			"ChatGPT-Account-Id": accountId,
		},
	});
	if (!response.ok) return null;
	const data = (await response.json()) as CodexUsageResponse;

	const toWindow = (raw: CodexWindow | null | undefined, fallbackSeconds: number): UsageWindow | null => {
		if (!raw) return null;
		const windowSeconds =
			Number.isFinite(raw.limit_window_seconds) && raw.limit_window_seconds! > 0
				? raw.limit_window_seconds!
				: fallbackSeconds;
		const nowSeconds = Math.round(Date.now() / 1000);
		const rawReset = raw.reset_at;
		const numericReset = typeof rawReset === "number" ? rawReset : typeof rawReset === "string" ? Number(rawReset) : Number.NaN;
		const datedReset = typeof rawReset === "string" ? Math.round(new Date(rawReset).getTime() / 1000) : Number.NaN;
		const resetAt =
			Number.isFinite(numericReset) && numericReset > 0
				? Math.round(numericReset > 10_000_000_000 ? numericReset / 1000 : numericReset)
				: Number.isFinite(datedReset) && datedReset > 0
					? datedReset
					: Number.isFinite(raw.reset_after_seconds) && raw.reset_after_seconds! > 0
						? nowSeconds + Math.round(raw.reset_after_seconds!)
						: 0;
		return {
			usedPercent: clampPercent(Number(raw.used_percent ?? 0)),
			resetAtEpochSeconds: resetAt,
			windowSeconds,
		};
	};

	const windows = [
		toWindow(data.rate_limit?.primary_window, FIVE_HOUR_SECONDS),
		toWindow(data.rate_limit?.secondary_window, SEVEN_DAY_SECONDS),
	].filter((window): window is UsageWindow => window !== null);
	return {
		fiveHour: windows.find((window) => window.windowSeconds < 24 * 3600) ?? null,
		sevenDay: windows.find((window) => window.windowSeconds >= 24 * 3600) ?? null,
	};
}

function selectWindow(usage: ProviderUsage): { window: UsageWindow; label: string } | null {
	const five = usage.fiveHour;
	const week = usage.sevenDay;
	if (!five && !week) return null;
	if (!week) return five ? { window: five, label: "5h" } : null;
	if (!five) return { window: week, label: "7d" };
	if (five.usedPercent >= 77 || week.usedPercent >= 77) {
		return five.usedPercent > week.usedPercent ? { window: five, label: "5h" } : { window: week, label: "7d" };
	}
	return { window: week, label: "7d" };
}

function formatReset(diffSeconds: number): string {
	if (diffSeconds <= 0) return "<1m";
	const days = Math.floor(diffSeconds / 86400);
	if (days >= 1) return `${days}d ${Math.floor((diffSeconds % 86400) / 3600)}h`;
	const hours = Math.floor(diffSeconds / 3600);
	if (hours >= 1) return `${hours}h ${Math.floor((diffSeconds % 3600) / 60)}m`;
	return `${Math.floor(diffSeconds / 60)}m`;
}

function pacePercent(window: UsageWindow, label: string, nowSeconds: number, resetDiffSeconds: number): number {
	if (label !== "7d") return clampPercent(((window.windowSeconds - resetDiffSeconds) / window.windowSeconds) * 100);
	const resetAt = window.resetAtEpochSeconds || nowSeconds;
	const windowDays = Math.max(1, Math.round(window.windowSeconds / 86400));
	const daysSinceLastReset = Math.max(0, Math.floor((nowSeconds - (resetAt - window.windowSeconds)) / 86400));
	return clampPercent(((daysSinceLastReset + 1) / windowDays) * 100);
}

function formatCalendarReset(epochSeconds: number): string {
	const reset = new Date(epochSeconds * 1000);
	const now = new Date();
	const today = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
	const resetDay = new Date(reset.getFullYear(), reset.getMonth(), reset.getDate()).getTime();
	const day = resetDay === today ? "today" : resetDay === today + 86_400_000 ? "tomorrow" : reset.toLocaleDateString(undefined, { weekday: "short" });
	return `${day} at ${reset.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" })}`;
}

function renderBar(usage: ProviderUsage, theme: { fg: (color: string, text: string) => string }): string | null {
	const selected = selectWindow(usage);
	if (!selected) return null;
	const { window, label } = selected;
	const nowSeconds = Math.round(Date.now() / 1000);
	const percent = Math.floor(window.usedPercent);
	const diff = Math.max(0, (window.resetAtEpochSeconds || nowSeconds) - nowSeconds);
	const pace = clampPercent(((window.windowSeconds - diff) / window.windowSeconds) * 100);
	const width = Math.floor((process.stdout.columns ?? 80) / 2);
	const fill = Math.floor((percent * width) / 100);
	const mark = Math.min(width - 1, Math.max(0, Math.floor((pace * width) / 100)));
	let bar = "";
	for (let index = 0; index < width; index++) {
		if (index === mark) bar += theme.fg("warning", "│");
		else if (index < fill) bar += "█";
		else bar += theme.fg("dim", "░");
	}
	return `${theme.fg("dim", label)} ${bar} ${percent}% ${theme.fg("dim", `(resets in ${formatReset(diff)})`)}`;
}

function quotaAdmissionFor(usage: ProviderUsage | null, nowSeconds: number): ProviderAdmission {
	const selected = usage ? selectWindow(usage) : null;
	if (!selected) {
		return { isFresh: false, usedPercent: null, pacePercent: null, reset: null, isEligible: false };
	}

	const diff = Math.max(0, (selected.window.resetAtEpochSeconds || nowSeconds) - nowSeconds);
	const usedPercent = Math.floor(selected.window.usedPercent);
	const pacedPercent = Math.floor(pacePercent(selected.window, selected.label, nowSeconds, diff));
	return {
		isFresh: true,
		usedPercent,
		pacePercent: pacedPercent,
		reset: selected.label === "5h" ? formatCalendarReset(selected.window.resetAtEpochSeconds || nowSeconds) : `in ${formatReset(diff)}`,
		isEligible: usedPercent <= pacedPercent,
	};
}

export default function statusline(pi: ExtensionAPI) {
	const usageByProvider = new Map<string, ProviderUsage>();

	pi.registerTool({
		name: "quota_admission",
		label: "Quota Admission",
		description: "Check whether fresh Anthropic or OpenAI Codex quota is on pace for new work.",
		parameters: QUOTA_ADMISSION_PARAMETERS,
		async execute(_toolCallId, _params, _signal, _onUpdate, ctx) {
			const fetched = await Promise.all(
				PROVIDERS.map(async (provider) => {
					try {
						const usage = provider === "anthropic" ? await fetchAnthropicUsage(ctx) : await fetchCodexUsage(ctx);
						return [provider, usage] as const;
					} catch {
						return [provider, null] as const;
					}
				}),
			);
			const nowSeconds = Math.round(Date.now() / 1000);
			const providers = Object.fromEntries(
				fetched.map(([provider, usage]) => {
					if (usage) usageByProvider.set(provider, usage);
					return [provider, quotaAdmissionFor(usage, nowSeconds)];
				}),
			) as Record<Provider, ProviderAdmission>;
			const result: QuotaAdmission = {
				isAdmitted: PROVIDERS.some((provider) => providers[provider].isFresh && providers[provider].isEligible),
				providers,
			};
			return {
				content: [{ type: "text", text: JSON.stringify(result) }],
				details: result,
			};
		},
	});
	const lastFetchAtByProvider = new Map<string, number>();
	let pollInterval: ReturnType<typeof setInterval> | null = null;
	let activeContext: ExtensionContext | null = null;

	function activeProvider(ctx: ExtensionContext): "anthropic" | "openai-codex" | null {
		const provider = ctx.model?.provider;
		return provider === "anthropic" || provider === "openai-codex" ? provider : null;
	}

	function render(ctx: ExtensionContext) {
		if (!isCtxActive(ctx)) return;
		// isCtxActive already cleared staleness above -- anything thrown past this point is a genuine
		// render fault, and the fire-and-forget call sites (refresh(), onResize) have no other catch.
		try {
			const provider = activeProvider(ctx);
			const usage = provider ? usageByProvider.get(provider) : undefined;
			const selected = usage ? selectWindow(usage) : null;
			if (!provider || !selected) {
				(globalThis as { __owaisQuotaState?: unknown }).__owaisQuotaState = undefined;
				ctx.ui.setStatus("statusline", undefined);
				return;
			}
			ctx.ui.setStatus("statusline", renderBar(usage, ctx.ui.theme) ?? undefined);
			const nowSeconds = Math.round(Date.now() / 1000);
			const diff = Math.max(0, (selected.window.resetAtEpochSeconds || nowSeconds) - nowSeconds);
			(globalThis as { __owaisQuotaState?: unknown }).__owaisQuotaState = {
				provider,
				usedPercent: Math.floor(selected.window.usedPercent),
				pacePercent: Math.floor(pacePercent(selected.window, selected.label, nowSeconds, diff)),
				label: selected.label,
				reset: selected.label === "5h" ? formatCalendarReset(selected.window.resetAtEpochSeconds || nowSeconds) : `in ${formatReset(diff)}`,
			};
		} catch (error) {
			console.error("[statusline] render failed:", error);
		}
	}

	const onResize = () => {
		if (activeContext) render(activeContext);
	};

	async function refresh(ctx: ExtensionContext, isForced = false) {
		if (!isCtxActive(ctx)) return;
		// this whole body runs fire-and-forget (`void refresh(ctx)` at every call site below), so
		// any throw past this point -- not just a fetch failure, which the inner catch already
		// covers -- would otherwise escape as an unhandled rejection with a raw stack trace.
		try {
			const provider = activeProvider(ctx);
			if (!provider) {
				render(ctx);
				return;
			}
			const now = Date.now();
			if (!isForced && now - (lastFetchAtByProvider.get(provider) ?? 0) < MIN_FETCH_INTERVAL_MS) {
				render(ctx);
				return;
			}
			lastFetchAtByProvider.set(provider, now);
			try {
				const usage =
					provider === "anthropic" ? await fetchAnthropicUsage(ctx) : await fetchCodexUsage(ctx);
				if (usage) usageByProvider.set(provider, usage);
			} catch {
				// A fetch failure keeps the previous bar; the next poll retries.
			}
			if (!isCtxActive(ctx)) return;
			render(ctx);
		} catch (error) {
			if (isCtxActive(ctx)) console.error("[statusline] refresh failed:", error);
		}
	}

	pi.on("session_start", async (_event, ctx) => {
		activeContext = ctx;
		void refresh(ctx);
		if (pollInterval) clearInterval(pollInterval);
		pollInterval = setInterval(() => void refresh(ctx, true), POLL_INTERVAL_MS);
		process.stdout.off("resize", onResize);
		if (ctx.mode === "tui") process.stdout.on("resize", onResize);
	});

	pi.on("model_select", async (_event, ctx) => {
		void refresh(ctx);
	});

	pi.on("agent_settled", async (_event, ctx) => {
		void refresh(ctx);
	});

	pi.on("session_shutdown", async () => {
		if (pollInterval) {
			clearInterval(pollInterval);
			pollInterval = null;
		}
		activeContext = null;
		process.stdout.off("resize", onResize);
	});
}
