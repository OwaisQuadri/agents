import type { ExtensionAPI, ExtensionCommandContext, ExtensionContext } from "@earendil-works/pi-coding-agent";
import { spawn } from "node:child_process";
import { access, mkdir, readFile, writeFile } from "node:fs/promises";
import { constants } from "node:fs";
import { delimiter, resolve } from "node:path";
import { homedir } from "node:os";
import { join } from "node:path";

// ExtensionContext.model has no exported type name from @earendil-works/pi-coding-agent
// (it re-exports Model only structurally). This mirrors the fields this file reads.
type ModelLike = {
	provider: string;
	baseUrl?: string;
	cost: { input: number; output: number; cacheRead: number };
};

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
	const totalMinutes = Math.round(waitMs / 60000);
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

async function handleMessageEnd(message: AssistantMessageLike, ctx: ExtensionContext): Promise<void> {
	const errorText = extractErrorText(message);
	if (!errorText) {
		return;
	}

	const lastResponse = lastProviderResponseByContext.get(ctx) ?? { status: 0, headers: {} };
	const plan = computeResetPlan({
		status: lastResponse.status,
		headers: lastResponse.headers,
		text: errorText,
		model: ctx.model,
		nowMs: Date.now(),
	});
	lastProviderResponseByContext.delete(ctx);
	if (!plan?.isDetected) {
		return;
	}

	if (plan.resetAtMs === null) {
		ctx.ui.notify("Usage limit hit, but the reset time could not be parsed. Not scheduling an automatic continue.", "warning");
		return;
	}

	const sessionFile = ctx.sessionManager.getSessionFile();
	if (!sessionFile) {
		ctx.ui.notify("Usage limit hit, but this session has no file to resume (--no-session). Not scheduling an automatic continue.", "warning");
		return;
	}
	const job = await scheduleResume(sessionFile, plan.resetAtMs, Date.now());
	if (job === null) {
		return;
	}
	ctx.ui.notify(`Usage limit hit. Will continue this session in ${formatWait(plan.resetAtMs - Date.now())}.`, "warning");
}

const lastProviderResponseByContext = new WeakMap<ExtensionContext, { status: number; headers: Record<string, string> }>();

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
		lastProviderResponseByContext.set(ctx, { status: event.status, headers: event.headers });
	});

	pi.on("message_end", async (event, ctx) => {
		await handleMessageEnd(event.message, ctx);
	});

	pi.registerCommand("usage-limit-status", {
		description: "List sessions scheduled to auto-continue when their usage limit resets",
		handler: handleStatusCommand,
	});
}
