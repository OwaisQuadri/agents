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

type AnthropicUsageResponse = {
	five_hour: { utilization: number; resets_at: string | null } | null;
	seven_day: { utilization: number; resets_at: string | null } | null;
};

type CodexWindow = {
	used_percent?: number;
	limit_window_seconds?: number;
	reset_at?: number;
	reset_after_seconds?: number;
};

type CodexUsageResponse = {
	rate_limit?: {
		primary_window?: CodexWindow | null;
		secondary_window?: CodexWindow | null;
	};
};

const BAR_WIDTH = 18;
const POLL_INTERVAL_MS = 10 * 60 * 1000;
const MIN_FETCH_INTERVAL_MS = 60 * 1000;
const ANTHROPIC_USAGE_URL = "https://api.anthropic.com/api/oauth/usage";
const CODEX_USAGE_URL = "https://chatgpt.com/backend-api/wham/usage";
const FIVE_HOUR_SECONDS = 5 * 3600;
const SEVEN_DAY_SECONDS = 7 * 86400;

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
		const resetAt =
			Number.isFinite(raw.reset_at) && raw.reset_at! > 0
				? Math.round(raw.reset_at!)
				: Number.isFinite(raw.reset_after_seconds) && raw.reset_after_seconds! > 0
					? nowSeconds + Math.round(raw.reset_after_seconds!)
					: 0;
		return {
			usedPercent: clampPercent(Number(raw.used_percent ?? 0)),
			resetAtEpochSeconds: resetAt,
			windowSeconds,
		};
	};

	return {
		fiveHour: toWindow(data.rate_limit?.primary_window, FIVE_HOUR_SECONDS),
		sevenDay: toWindow(data.rate_limit?.secondary_window, SEVEN_DAY_SECONDS),
	};
}

// The weekly window is the standing view. The session window replaces it only once it
// is both near its cap and the worse of the two, so the line reports one number.
function selectWindow(usage: ProviderUsage): { window: UsageWindow; label: string } | null {
	const five = usage.fiveHour;
	const week = usage.sevenDay;
	if (!five && !week) return null;
	if (!week) return five && five.usedPercent > 90 ? { window: five, label: "5h" } : null;
	if (!five) return { window: week, label: "7d" };
	if (five.usedPercent > 90 && five.usedPercent > week.usedPercent) {
		return { window: five, label: "5h" };
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

function renderBar(usage: ProviderUsage, theme: { fg: (color: string, text: string) => string }): string | null {
	const selected = selectWindow(usage);
	if (!selected) return null;
	const { window, label } = selected;

	const nowSeconds = Math.round(Date.now() / 1000);
	const percent = Math.floor(window.usedPercent);
	const diff = Math.max(0, (window.resetAtEpochSeconds || nowSeconds) - nowSeconds);

	// On-pace is the share of the window already spent, read from its own reset stamp.
	const pace = clampPercent(((window.windowSeconds - diff) / window.windowSeconds) * 100);
	const fill = Math.floor((percent * BAR_WIDTH) / 100);
	const mark = Math.min(BAR_WIDTH - 1, Math.max(0, Math.floor((pace * BAR_WIDTH) / 100)));

	let bar = "";
	for (let i = 0; i < BAR_WIDTH; i++) {
		if (i === mark) bar += theme.fg("warning", "│");
		else if (i < fill) bar += "█";
		else bar += theme.fg("dim", "░");
	}

	const isOverCap = (usage.fiveHour?.usedPercent ?? 0) >= 100 || (usage.sevenDay?.usedPercent ?? 0) >= 100;
	const percentText = isOverCap
		? theme.fg("error", `${percent}%`)
		: percent > pace
			? theme.fg("warning", `${percent}%`)
			: `${percent}%`;

	return `${theme.fg("dim", label)} ${bar} ${percentText} ${theme.fg("dim", `(resets in ${formatReset(diff)})`)}`;
}

export default function statusline(pi: ExtensionAPI) {
	const usageByProvider = new Map<string, ProviderUsage>();
	const lastFetchAtByProvider = new Map<string, number>();
	let pollInterval: ReturnType<typeof setInterval> | null = null;

	function activeProvider(ctx: ExtensionContext): "anthropic" | "openai-codex" | null {
		const provider = ctx.model?.provider;
		return provider === "anthropic" || provider === "openai-codex" ? provider : null;
	}

	function render(ctx: ExtensionContext) {
		if (!isCtxActive(ctx)) return;
		if (!ctx.hasUI) return;
		const provider = activeProvider(ctx);
		const usage = provider ? usageByProvider.get(provider) : undefined;
		const line = usage ? renderBar(usage, ctx.ui.theme) : null;
		ctx.ui.setStatus("statusline", line ?? undefined);
	}

	async function refresh(ctx: ExtensionContext, isForced = false) {
		if (!isCtxActive(ctx)) return;
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
	}

	pi.on("session_start", async (_event, ctx) => {
		void refresh(ctx);
		if (pollInterval) clearInterval(pollInterval);
		pollInterval = setInterval(() => void refresh(ctx, true), POLL_INTERVAL_MS);
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
	});
}
