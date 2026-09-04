import type { ExtensionAPI, ExtensionCommandContext, ExtensionContext } from "@earendil-works/pi-coding-agent";
import { spawn } from "node:child_process";
import { access, appendFile, mkdir, readFile, writeFile } from "node:fs/promises";
import { constants } from "node:fs";
import { delimiter, resolve } from "node:path";
import { homedir } from "node:os";
import { join } from "node:path";

// ExtensionContext.model has no exported type name from @earendil-works/pi-coding-agent
// (it re-exports Model only structurally). This mirrors the fields this file reads.
type ModelLike = {
	provider: string;
	id?: string;
	baseUrl?: string;
	cost: { input: number; output: number; cacheRead: number };
};

// Compiled by install.sh from config/model-tiers.json. One fallback map per tier: a model
// in two tiers' chains keeps a distinct next hop in each. tierPrimaries names each tier's
// primary, which is how a session resolves the tier it walks — see resolveHomeTier.
type ThinkingLevel = "off" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max";
type TierHop = { model: string; thinking: ThinkingLevel };
type FallbackMap = Record<string, TierHop>;
type TieredFallbackMap = Record<string, FallbackMap>;
type CompiledTiers = { tiered: TieredFallbackMap; primaries: Record<string, TierHop>; climb: Record<string, string> };

// AgentMessage has no exported type name from @earendil-works/pi-coding-agent either;
// this mirrors the fields this file reads off an assistant message.
type AssistantMessageLike = {
	role: string;
	stopReason?: string;
	errorMessage?: string;
};

type ResetPlan = {
	isDetected: boolean;
	resetAtMs: number | null;
	matchedText: string;
};

type PendingJob = {
	sessionFile: string;
	resetAtMs: number;
	scheduledAtMs: number;
	pid: number;
};

type PendingStore = Record<string, PendingJob>;

function stateDir(): string {
	return join(homedir(), ".pi", "agent", "usage-limit-continue");
}

function stateFile(): string {
	return join(stateDir(), "pending.json");
}

function logFile(): string {
	return join(stateDir(), "resume.log");
}

/**
 * Appends a timestamped diagnostic line to resume.log, best effort. Routing decisions
 * (detected, fallback found/not found, switch applied, retry queued) are otherwise
 * invisible once a headless worker's stderr is truncated to 200 chars by the caller.
 */
function logDiagnostic(line: string): void {
	void mkdir(stateDir(), { recursive: true })
		.then(() => appendFile(logFile(), `${new Date().toISOString()} [usage-limit] ${line}\n`))
		.catch(() => {
			// Diagnostics never block or fail the fallback path.
		});
}

function settingsFile(): string {
	return join(homedir(), ".pi", "agent", "settings.json");
}

const RESUME_PROMPT = "continue";
const MAX_SCHEDULABLE_WAIT_MS = 8 * 24 * 60 * 60 * 1000;

const LOCAL_PROVIDER_PATTERN = /ollama|lmstudio|llama\.cpp|^local$/i;
const LOCAL_HOST_PATTERN = /localhost|127\.0\.0\.1|0\.0\.0\.0|\[::1\]/i;

const USAGE_LIMIT_PATTERN =
	/usage limit|rate.?limit|too many requests|429|quota exceeded|weekly limit|5.hour limit|five.hour limit/i;

const SESSION_WINDOW_PATTERN = /5.hour|five.hour|session limit/i;
const WEEKLY_WINDOW_PATTERN = /weekly|7.day|seven.day/i;
const SESSION_WINDOW_MS = 5 * 60 * 60 * 1000;
const WEEKLY_WINDOW_MS = 7 * 24 * 60 * 60 * 1000;

const RESET_AT_CLOCK_PATTERN = /resets?\s+at\s+(\d{1,2})(?::(\d{2}))?\s*(am|pm)?/i;
const RESET_IN_DURATION_PATTERN =
	/(?:resets?|try again|available again)\s+in\s+((?:\d+\s*d(?:ays?)?\s*)?(?:\d+\s*h(?:ours?)?\s*)?(?:\d+\s*m(?:in(?:utes?)?)?\s*)?(?:\d+\s*s(?:ec(?:onds?)?)?\s*)?)/i;
const ISO_TIMESTAMP_PATTERN = /\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:?\d{2})/;

/** Reports whether a model is served locally, so it is exempt from usage-limit tracking. */
export function isLocalModel(model: ModelLike | undefined): boolean {
	if (!model) {
		return false;
	}
	const isProviderLocal = LOCAL_PROVIDER_PATTERN.test(model.provider);
	const isHostLocal = LOCAL_HOST_PATTERN.test(model.baseUrl ?? "");
	return isProviderLocal || isHostLocal;
}

/** Reports whether text names a usage or rate limit, independent of whether a reset time is parseable. */
export function detectUsageLimitSignal(text: string): boolean {
	return USAGE_LIMIT_PATTERN.test(text);
}

function parseDurationToMs(fragment: string): number | null {
	const days = /(\d+)\s*d/i.exec(fragment);
	const hours = /(\d+)\s*h/i.exec(fragment);
	const minutes = /(\d+)\s*m(?!s)/i.exec(fragment);
	const seconds = /(\d+)\s*s/i.exec(fragment);
	if (!days && !hours && !minutes && !seconds) {
		return null;
	}
	const dayMs = days ? Number(days[1]) * 24 * 60 * 60 * 1000 : 0;
	const hourMs = hours ? Number(hours[1]) * 60 * 60 * 1000 : 0;
	const minuteMs = minutes ? Number(minutes[1]) * 60 * 1000 : 0;
	const secondMs = seconds ? Number(seconds[1]) * 1000 : 0;
	return dayMs + hourMs + minuteMs + secondMs;
}

/** Parses a reset time out of a 429 response's headers (retry-after, or any *-reset header). */
export function parseResetFromHeaders(headers: Record<string, string>, nowMs: number): number | null {
	const candidates: number[] = [];
	for (const [key, value] of Object.entries(headers)) {
		const normalizedKey = key.toLowerCase();
		if (normalizedKey === "retry-after") {
			const seconds = Number(value);
			if (Number.isFinite(seconds)) {
				candidates.push(nowMs + seconds * 1000);
				continue;
			}
			const httpDateMs = Date.parse(value);
			if (Number.isFinite(httpDateMs)) {
				candidates.push(httpDateMs);
			}
			continue;
		}
		if (normalizedKey.endsWith("-reset") || normalizedKey.includes("-reset-")) {
			const isoMs = Date.parse(value);
			if (Number.isFinite(isoMs) && isoMs > nowMs) {
				candidates.push(isoMs);
				continue;
			}
			const durationMs = parseDurationToMs(value);
			if (durationMs !== null) {
				candidates.push(nowMs + durationMs);
				continue;
			}
			const seconds = Number(value);
			if (Number.isFinite(seconds)) {
				candidates.push(nowMs + seconds * 1000);
			}
		}
	}
	return candidates.length > 0 ? Math.max(...candidates) : null;
}

/**
 * Falls back to the plan's stated 5-hour session window or 7-day weekly window when the
 * text names one but carries no parseable instant (Claude's two known usage-limit windows).
 */
export function fallbackWindowResetMs(text: string, nowMs: number): number | null {
	if (SESSION_WINDOW_PATTERN.test(text)) {
		return nowMs + SESSION_WINDOW_MS;
	}
	if (WEEKLY_WINDOW_PATTERN.test(text)) {
		return nowMs + WEEKLY_WINDOW_MS;
	}
	return null;
}

/** Parses a reset time out of an error message's free text ("resets at 3pm", "resets in 2h"). */
export function parseResetFromText(text: string, nowMs: number): number | null {
	const isoMatch = ISO_TIMESTAMP_PATTERN.exec(text);
	if (isoMatch) {
		const isoMs = Date.parse(isoMatch[0]);
		if (Number.isFinite(isoMs)) {
			return isoMs;
		}
	}

	const durationMatch = RESET_IN_DURATION_PATTERN.exec(text);
	if (durationMatch) {
		const durationMs = parseDurationToMs(durationMatch[1]);
		if (durationMs !== null) {
			return nowMs + durationMs;
		}
	}

	const clockMatch = RESET_AT_CLOCK_PATTERN.exec(text);
	if (clockMatch) {
		const [, hourText, minuteText, meridiem] = clockMatch;
		let hour = Number(hourText);
		const minute = minuteText ? Number(minuteText) : 0;
		if (meridiem?.toLowerCase() === "pm" && hour < 12) {
			hour += 12;
		}
		if (meridiem?.toLowerCase() === "am" && hour === 12) {
			hour = 0;
		}
		const candidate = new Date(nowMs);
		candidate.setHours(hour, minute, 0, 0);
		if (candidate.getTime() <= nowMs) {
			candidate.setDate(candidate.getDate() + 1);
		}
		return candidate.getTime();
	}

	return null;
}

/** Combines the 429 header signal and the error-text signal into one plan for whether and when to resume. */
export function computeResetPlan(input: {
	status: number;
	headers: Record<string, string>;
	text: string;
	model: ModelLike | undefined;
	nowMs: number;
}): ResetPlan | null {
	if (isLocalModel(input.model)) {
		return null;
	}
	const isRateLimited = input.status === 429;
	const hasTextSignal = detectUsageLimitSignal(input.text);
	if (!isRateLimited && !hasTextSignal) {
		return null;
	}
	const resetAtMs =
		parseResetFromHeaders(input.headers, input.nowMs) ??
		parseResetFromText(input.text, input.nowMs) ??
		fallbackWindowResetMs(input.text, input.nowMs);
	return { isDetected: true, resetAtMs, matchedText: input.text };
}

/**
 * Finds the tier fallback for a model, within ONE tier's own map, so a usage limit degrades
 * one tier sideways along that tier's own chain — never a different tier's chain that happens
 * to share the same model at a different position.
 * @param fallbacks One tier's compiled fallback map, keyed by `provider/id`.
 * @param provider Provider id of the model that hit the limit.
 * @param modelId Model id of the model that hit the limit.
 * @returns The next hop's model and thinking level, or null when the chain ends here.
 */
export function findTierFallback(fallbacks: FallbackMap, provider: string, modelId: string): TierHop | null {
	return fallbacks[`${provider}/${modelId}`] ?? null;
}

/**
 * Finds which tier a model is the PRIMARY of, so a session's first usage-limit hop knows which
 * tier's chain to walk. A model that names more than one tier's primary (should not happen —
 * each tier's primary is meant to be unique) resolves to the first tier in sorted order,
 * deterministically rather than by object-key iteration order.
 * @param primaries Each tier's own primary model and thinking level, keyed by tier name.
 * @param provider Provider id of the model to resolve a home tier for.
 * @param modelId Model id of the model to resolve a home tier for.
 * @returns The tier name, or null when the model is no tier's primary.
 */
export function resolveHomeTier(primaries: Record<string, TierHop>, provider: string, modelId: string): string | null {
	const qualified = `${provider}/${modelId}`;
	const matches = Object.keys(primaries)
		.filter((tier) => primaries[tier].model === qualified)
		.sort();
	return matches[0] ?? null;
}

/**
 * Picks the abandoned model that comes back first, so a resume waits the shortest time. Each
 * entry carries the thinking level that model was actually running at when it was abandoned,
 * so a climb-back restores exactly how the session was running, not a tier's current default.
 * @param resets Abandoned models this session, keyed by `provider/id`, valued by reset epoch
 * ms and the thinking level active on that model at abandon time.
 * @returns The soonest-returning model, its reset, and its thinking level, or null when
 * nothing was abandoned.
 */
export function earliestAvailable(
	resets: Record<string, { resetAtMs: number; thinking: ThinkingLevel }>,
	nowMs = Number.NEGATIVE_INFINITY,
): (TierHop & { resetAtMs: number }) | null {
	let soonest: (TierHop & { resetAtMs: number }) | null = null;
	for (const [model, entry] of Object.entries(resets)) {
		if (entry.resetAtMs <= nowMs) {
			continue;
		}
		if (!soonest || entry.resetAtMs < soonest.resetAtMs) {
			soonest = { model, resetAtMs: entry.resetAtMs, thinking: entry.thinking };
		}
	}
	return soonest;
}

async function loadFallbacks(): Promise<CompiledTiers> {
	try {
		const settings = JSON.parse(await readFile(settingsFile(), "utf8")) as {
			modelTierFallbacks?: TieredFallbackMap;
			tierPrimaries?: Record<string, TierHop>;
			tierClimb?: Record<string, string>;
		};
		return {
			tiered: settings.modelTierFallbacks ?? {},
			primaries: settings.tierPrimaries ?? {},
			climb: settings.tierClimb ?? {},
		};
	} catch {
		return { tiered: {}, primaries: {}, climb: {} };
	}
}

function extractErrorText(message: AssistantMessageLike): string {
	if (message.role !== "assistant") {
		return "";
	}
	if (message.stopReason !== "error") {
		return "";
	}
	return message.errorMessage ?? "";
}

async function resolveExecutableOnPath(name: string, pathEnv: string): Promise<string | null> {
	for (const directory of pathEnv.split(delimiter)) {
		if (!directory) {
			continue;
		}
		const candidate = resolve(directory, name);
		try {
			await access(candidate, constants.X_OK);
			return candidate;
		} catch {
			continue;
		}
	}
	return null;
}

async function loadPendingStore(): Promise<PendingStore> {
	try {
		const raw = await readFile(stateFile(), "utf8");
		return JSON.parse(raw) as PendingStore;
	} catch {
		return {};
	}
}

async function savePendingStore(store: PendingStore): Promise<void> {
	await mkdir(stateDir(), { recursive: true });
	await writeFile(stateFile(), JSON.stringify(store, null, 2));
}

function formatWait(waitMs: number): string {
	const totalMinutes = Math.max(Math.round(waitMs / 60000), 0);
	const hours = Math.floor(totalMinutes / 60);
	const minutes = totalMinutes % 60;
	if (hours === 0) {
		return `${minutes}m`;
	}
	return `${hours}h${minutes}m`;
}

/**
 * Schedules a detached resume of the given session once the usage limit resets.
 * @param sessionFile Path to the pi session file to resume.
 * @param resetAtMs Epoch ms of the usage-limit reset.
 * @returns The scheduled job, or null if a job for this session is already pending.
 */
export async function scheduleResume(sessionFile: string, resetAtMs: number, nowMs: number): Promise<PendingJob | null> {
	if (resetAtMs <= nowMs) {
		return null;
	}
	const store = await loadPendingStore();
	const existing = store[sessionFile];
	if (existing && existing.resetAtMs >= nowMs) {
		return null;
	}

	const waitMs = Math.min(Math.max(resetAtMs - nowMs, 0), MAX_SCHEDULABLE_WAIT_MS);
	const piPath = (await resolveExecutableOnPath("pi", process.env.PATH ?? "")) ?? "pi";
	const waitSeconds = Math.ceil(waitMs / 1000);
	const command = `sleep ${waitSeconds} && '${piPath}' --session '${sessionFile}' --print '${RESUME_PROMPT}' >> '${logFile()}' 2>&1`;

	await mkdir(stateDir(), { recursive: true });
	const child = spawn("/bin/sh", ["-c", command], {
		detached: true,
		stdio: "ignore",
	});
	child.unref();

	const job: PendingJob = { sessionFile, resetAtMs, scheduledAtMs: nowMs, pid: child.pid ?? -1 };
	store[sessionFile] = job;
	await savePendingStore(store);
	return job;
}

async function switchTo(pi: ExtensionAPI, ctx: ExtensionContext, hop: TierHop): Promise<boolean> {
	const separator = hop.model.indexOf("/");
	const candidate = ctx.modelRegistry.find(hop.model.slice(0, separator), hop.model.slice(separator + 1));
	if (!candidate || !(await pi.setModel(candidate))) {
		return false;
	}
	// Each hop applies its own tier entry's thinking level, not the prior model's.
	pi.setThinkingLevel(hop.thinking);
	return true;
}

// Resolved on the walk's first hop and reused after, so a model shared by two tiers
// never re-derives a different tier mid-walk.
type FallbackState = {
	sessionFile: string | undefined;
	tier?: string;
	attempted: Set<string>;
	lastProviderResponse?: { status: number; headers: Record<string, string> };
	abandoned: Record<string, { resetAtMs: number; thinking: ThinkingLevel }>;
};

const fallbackStateBySessionManager = new WeakMap<object, FallbackState>();

function fallbackState(ctx: ExtensionContext): FallbackState {
	const manager = ctx.sessionManager as object;
	const sessionFile = ctx.sessionManager.getSessionFile();
	let state = fallbackStateBySessionManager.get(manager);
	if (!state || state.sessionFile !== sessionFile) {
		state = { sessionFile, attempted: new Set<string>(), abandoned: {} };
		fallbackStateBySessionManager.set(manager, state);
	}
	return state;
}

function firstUntriedHop(map: FallbackMap, start: TierHop | undefined, attempted: Set<string>): TierHop | undefined {
	const seen = new Set<string>();
	let hop = start;
	while (hop && attempted.has(hop.model)) {
		if (seen.has(hop.model)) {
			return undefined;
		}
		seen.add(hop.model);
		const [provider, ...id] = hop.model.split("/");
		hop = findTierFallback(map, provider, id.join("/")) ?? undefined;
	}
	return hop;
}

async function applyTierFallback(pi: ExtensionAPI, ctx: ExtensionContext): Promise<string | null> {
	const active = ctx.model as ModelLike | undefined;
	if (!active?.id) {
		return null;
	}
	const { tiered, primaries, climb } = await loadFallbacks();
	// Home tier resolves only from a primary position; a session starting mid-chain
	// (manual /model switch) gets no fallback, same as a model outside the tier file.
	const state = fallbackState(ctx);
	const qualifiedActive = `${active.provider}/${active.id}`;
	let tier = state.tier ?? resolveHomeTier(primaries, active.provider, active.id);
	if (tier) {
		const map = tiered[tier] ?? {};
		const belongsToTier =
			primaries[tier]?.model === qualifiedActive ||
			qualifiedActive in map ||
			Object.values(map).some((candidate) => candidate.model === qualifiedActive);
		if (!belongsToTier) {
			tier = resolveHomeTier(primaries, active.provider, active.id);
			state.tier = tier ?? undefined;
		}
	}
	if (!tier) {
		return null;
	}
	const attempted = state.attempted;
	let hop = firstUntriedHop(
		tiered[tier] ?? {},
		findTierFallback(tiered[tier] ?? {}, active.provider, active.id) ?? undefined,
		attempted,
	);
	if (!hop && climb[tier]) {
		const nextTier = climb[tier];
		hop = firstUntriedHop(tiered[nextTier] ?? {}, primaries[nextTier], attempted);
		if (hop) {
			tier = nextTier;
		}
	}
	if (!hop) {
		return null;
	}
	if (!(await switchTo(pi, ctx, hop))) {
		return null;
	}
	state.tier = tier;
	return hop.model;
}

export async function handleMessageEnd(message: AssistantMessageLike, ctx: ExtensionContext, pi: ExtensionAPI): Promise<void> {
	if (message.role !== "assistant") {
		return;
	}
	const errorText = extractErrorText(message);
	const state = fallbackState(ctx);
	if (!errorText) {
		if (message.stopReason !== "error" && message.stopReason !== "aborted") {
			state.attempted.clear();
		}
		return;
	}

	const nowMs = Date.now();
	const lastResponse = state.lastProviderResponse ?? { status: 0, headers: {} };
	const plan = computeResetPlan({
		status: lastResponse.status,
		headers: lastResponse.headers,
		text: errorText,
		model: ctx.model,
		nowMs,
	});
	state.lastProviderResponse = undefined;
	if (!plan?.isDetected) {
		return;
	}
	logDiagnostic(`detected on ${ctx.model?.provider}/${ctx.model?.id} (mode=${ctx.mode}): ${plan.matchedText.slice(0, 200)}`);

	const active = ctx.model as ModelLike | undefined;
	if (active?.id) {
		state.attempted.add(`${active.provider}/${active.id}`);
	}
	const abandoned = state.abandoned;
	if (active?.id && plan.resetAtMs !== null && plan.resetAtMs > nowMs) {
		abandoned[`${active.provider}/${active.id}`] = { resetAtMs: plan.resetAtMs, thinking: pi.getThinkingLevel() };
	}

	const fallbackModel = await applyTierFallback(pi, ctx);
	if (fallbackModel) {
		ctx.ui.notify(`Usage limit hit. Switched to the tier fallback ${fallbackModel}; the session continues.`, "warning");
		// print/json workers exit on the LAST message's stopReason; retry it so that check sees the fallback model's work, not the stale error.
		if (ctx.mode === "print" || ctx.mode === "json") {
			logDiagnostic(`retrying same turn on ${fallbackModel} (mode=${ctx.mode})`);
			pi.sendUserMessage(RESUME_PROMPT, { deliverAs: "followUp" });
		} else {
			logDiagnostic(`fallback ${fallbackModel} applied, no retry (mode=${ctx.mode})`);
		}
		return;
	}
	logDiagnostic(
		`no tier fallback for ${active?.provider}/${active?.id ?? "unknown"}; falling back to a scheduled resume`,
	);

	const returning = earliestAvailable(abandoned, nowMs);
	const resumeAtMs = returning?.resetAtMs ?? (plan.resetAtMs !== null && plan.resetAtMs > nowMs ? plan.resetAtMs : null);
	if (resumeAtMs === null) {
		ctx.ui.notify("Usage limit hit, but the reset time could not be parsed. Not scheduling an automatic continue.", "warning");
		return;
	}

	const sessionFile = ctx.sessionManager.getSessionFile();
	if (!sessionFile) {
		logDiagnostic("no session file to resume; usage limit is fatal for this run");
		ctx.ui.notify("Usage limit hit, but this session has no file to resume (--no-session). Not scheduling an automatic continue.", "warning");
		return;
	}
	const job = await scheduleResume(sessionFile, resumeAtMs, nowMs);
	if (job === null) {
		logDiagnostic(`resume already pending for ${sessionFile}, not rescheduling`);
		return;
	}
	const climbedBack = returning ? await switchTo(pi, ctx, returning) : false;
	const destination = climbedBack ? ` on ${returning?.model}` : "";
	logDiagnostic(`scheduled resume of ${sessionFile} in ${formatWait(resumeAtMs - Date.now())}${destination} (pid ${job.pid})`);
	ctx.ui.notify(`Usage limit hit. Will continue this session${destination} in ${formatWait(resumeAtMs - Date.now())}.`, "warning");
}

async function handleStatusCommand(_args: string, ctx: ExtensionCommandContext): Promise<void> {
	const store = await loadPendingStore();
	const jobs = Object.values(store);
	if (jobs.length === 0) {
		ctx.ui.notify("No usage-limit continuations are pending.", "info");
		return;
	}
	const now = Date.now();
	for (const job of jobs) {
		const wait = job.resetAtMs > now ? formatWait(job.resetAtMs - now) : "due now";
		ctx.ui.notify(`${job.sessionFile}: resumes in ${wait} (pid ${job.pid})`, "info");
	}
}

export default async function usageLimitContinueExtension(pi: ExtensionAPI): Promise<void> {
	pi.on("after_provider_response", (event, ctx) => {
		fallbackState(ctx).lastProviderResponse = { status: event.status, headers: event.headers };
	});

	pi.on("message_end", async (event, ctx) => {
		await handleMessageEnd(event.message, ctx, pi);
	});

	pi.registerCommand("usage-limit-status", {
		description: "List sessions scheduled to auto-continue when their usage limit resets",
		handler: handleStatusCommand,
	});
}
