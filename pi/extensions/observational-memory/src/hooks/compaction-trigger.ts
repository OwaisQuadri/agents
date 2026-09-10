import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { OM_RESUME, rawTokensSinceLastCompaction, type Entry } from "../ledger/index.js";
import type { Runtime } from "../runtime.js";

export const MEMORY_COMPACTION_POLICY_EVENT = "om:compaction-policy";

export interface MemoryCompactionPolicy {
	isEnabled: boolean;
	contextWindow: number;
	threshold: number;
	isApplied: boolean;
	keepRecentTokens?: number;
	error?: string;
}

/** Pi's retryable-error detection: don't compact between an auto-retried turn's attempts. */
const RETRYABLE_ERROR_RE =
	/overloaded|provider.?returned.?error|rate.?limit|too many requests|429|500|502|503|504|service.?unavailable|server.?error|internal.?error|network.?error|connection.?error|connection.?refused|connection.?lost|websocket.?closed|websocket.?error|other side closed|fetch failed|upstream.?connect|reset before headers|socket hang up|ended without|http2 request did not get a response|timed? out|timeout|terminated|retry delay/i;

const RESUME_PROMPT =
	"[automatic] Your context was just compacted to free space; no user message was sent. " +
	"Continue exactly where you left off, as if the compaction had not happened.";

function contextPressureTokens(
	ctx: { getContextUsage?: () => { tokens: number | null } | undefined; sessionManager: { getBranch: () => Entry[] } },
	threshold: number,
): { tokens: number; isDue: boolean } {
	const live = ctx.getContextUsage?.()?.tokens;
	if (live != null) return { tokens: live, isDue: live >= threshold };
	const raw = rawTokensSinceLastCompaction(ctx.sessionManager.getBranch());
	return { tokens: raw, isDue: raw >= threshold };
}

function isTurnContinuing(event: any): boolean {
	const toolResults = event?.toolResults;
	if (Array.isArray(toolResults) && toolResults.length > 0) return true;
	const stopReason = event?.message?.stopReason;
	return stopReason === "tool_use" || stopReason === "tool_calls";
}

function keepRecentTokens(event: any, ctx: { getContextUsage?: () => { tokens: number | null } | undefined }): number | undefined {
	const contextTokens = ctx.getContextUsage?.()?.tokens;
	const usage = event?.message?.usage;
	const usageTokens = usage?.totalTokens || (usage?.input ?? 0) + (usage?.output ?? 0) + (usage?.cacheRead ?? 0) + (usage?.cacheWrite ?? 0);
	if (contextTokens == null || !Number.isSafeInteger(contextTokens) || !Number.isSafeInteger(usageTokens) || usageTokens < 0) {
		return undefined;
	}
	return Math.max(1, contextTokens - usageTokens + 1);
}

/**
 * Publishes the current automatic-compaction policy to a child-local runner.
 *
 * @param pi - The extension API whose event bus delivers the policy synchronously.
 * @param runtime - The observational-memory state that provides the current threshold.
 * @param ctx - The active extension context that provides the selected model window.
 * @param isPolicyEnabled - Whether the receiver should own or release automatic compaction.
 * @returns Whether a receiver accepted enabled automatic compaction.
 * @throws Never throws; a host without a receiver leaves the policy unacknowledged.
 */
export function publishCompactionPolicy(
	pi: ExtensionAPI,
	runtime: Runtime,
	ctx: { model?: { contextWindow?: number } } | undefined,
	isPolicyEnabled = runtime.enabled && !runtime.config.passive,
	minimumKeepRecentTokens: number | undefined = undefined,
): boolean {
	const eventBus = (pi as unknown as { events?: { emit: (channel: string, data: unknown) => void } }).events;
	if (!eventBus) return false;
	const policy: MemoryCompactionPolicy = {
		isEnabled: isPolicyEnabled,
		contextWindow: ctx?.model?.contextWindow ?? 0,
		threshold: runtime.config.compactAtContextTokens,
		isApplied: false,
		...(minimumKeepRecentTokens !== undefined && { keepRecentTokens: minimumKeepRecentTokens }),
	};
	eventBus.emit(MEMORY_COMPACTION_POLICY_EVENT, policy);
	if (policy.error) runtime.lastWorkerError = policy.error;
	return policy.isEnabled && policy.isApplied;
}

/**
 * Trigger compaction on `turn_end` once live context usage crosses `compactAtContextTokens`.
 *
 * A supporting child runner takes ownership through `om:compaction-policy`, enabling Pi's
 * between-turn compaction path. Standalone hosts leave the policy unacknowledged and retain the
 * existing manual call.
 */
export function registerCompactionTrigger(pi: ExtensionAPI, runtime: Runtime): void {
	pi.on("session_start", (_event, ctx) => {
		publishCompactionPolicy(pi, runtime, ctx);
	});

	pi.on("session_shutdown", () => {
		publishCompactionPolicy(pi, runtime, undefined, false);
	});

	pi.on("turn_end", (event: any, ctx: any) => {
		if (!runtime.enabled || runtime.config.passive) return;
		if (runtime.compactInFlight) return;

		const message = event?.message;
		if (
			message?.role === "assistant" &&
			message.stopReason === "error" &&
			message.errorMessage &&
			RETRYABLE_ERROR_RE.test(message.errorMessage)
		) {
			return;
		}

		const pressure = contextPressureTokens(ctx, runtime.config.compactAtContextTokens);
		if (publishCompactionPolicy(pi, runtime, ctx, true, pressure.isDue ? keepRecentTokens(event, ctx) : undefined)) return;
		if (!pressure.isDue) return;

		const isResumeNeeded = runtime.config.resumeAfterMidRunCompaction && isTurnContinuing(event);
		const hasUI = ctx.hasUI;
		const ui = ctx.ui;
		runtime.compactInFlight = true;
		if (hasUI) ui?.notify("om: context threshold reached — compacting (waiting for in-flight observers)…", "info");

		ctx.compact({
			onComplete: () => {
				runtime.compactInFlight = false;
				if (hasUI) ui?.notify("om: compaction complete", "info");
				if (!isResumeNeeded || !runtime.enabled || runtime.config.passive) return;
				try {
					pi.sendMessage(
						{ customType: OM_RESUME, content: RESUME_PROMPT, display: false },
						{ triggerTurn: true },
					);
				} catch (error) {
					const message = error instanceof Error ? error.message : String(error);
					runtime.lastWorkerError = `resume failed: ${message}`;
					if (hasUI) ui?.notify(`om: resume failed — ${message}`, "error");
				}
			},
			onError: (error: { message: string }) => {
				runtime.compactInFlight = false;
				if (error.message === "Compaction cancelled") return;
				if (hasUI) ui?.notify(`om: ${error.message}`, "error");
			},
		});
	});
}
