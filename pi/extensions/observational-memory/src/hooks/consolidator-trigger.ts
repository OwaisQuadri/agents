/**
 * Phase B consolidator clock. When the active observation pool crosses
 * `consolidateAtPoolTokens`, promote the oldest observations (above `poolTargetTokens`) into
 * durable `.memory/` topic files via a subprocess consolidator, then tombstone the batch only
 * after a durable memory change.
 *
 * Runs in the BACKGROUND, mirroring the observer trigger (turn_end / agent_start), strictly
 * one at a time (design risk 4). Compaction does not wait for it (R5).
 *
 * Tombstone safety (design risk 4): the orchestrator tombstones the batch it handed the
 * consolidator, intersected with what is STILL active at exit — never an observation an
 * observer committed during the run (those are not in the handed batch). A clean worker must
 * also change durable memory before any observation is dropped.
 */
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import {
	OM_OBSERVATIONS_DROPPED,
	foldLedger,
	lastSourceEntryId,
	observationToLine,
	poolTokens,
	selectPromotionOverflow,
	sortObservations,
	type Entry,
	type Observation,
} from "../ledger/index.js";
import { nowTimestamp } from "../ledger/serialize.js";
import { renderIndexFile } from "../memory/index-render.js";
import { atomicWrite, indexPath, listTopics, readJourney } from "../memory/paths.js";
import type { Runtime } from "../runtime.js";
import { buildWorkerArgv, buildWorkerEnv, spawnWorker } from "../spawn/launch.js";
import { recordWorkerCost } from "./observer-trigger.js";

type TriggerCtx = {
	hasUI: boolean;
	ui?: { notify: (message: string, level?: "info" | "warning" | "error") => void };
	sessionManager: { getBranch: () => Entry[]; getEntries: () => Entry[] };
	getContextUsage?: () => { tokens: number | null } | undefined;
};

let runCounter = 0;

function isContextActive(ctx: TriggerCtx): boolean {
	try {
		void ctx.hasUI;
		return true;
	} catch {
		return false;
	}
}

function hasActiveUI(ctx: TriggerCtx): boolean {
	return isContextActive(ctx) && ctx.hasUI;
}

function nextRunId(): string {
	runCounter += 1;
	const stamp = new Date().toISOString().replace(/[-:.TZ]/g, "").slice(0, 14);
	return `cons-${stamp}-${process.pid}-${runCounter}`;
}

/**
 * Build the consolidator's `-p` prompt: current time + current index + current journey + the
 * overflow lines. The journey is included verbatim so the consolidator updates it in place
 * (append a segment for this batch; compress the old tail only if over `journeyTargetTokens`).
 */
/**
 * Returns a content hash for the durable memory files under a session root.
 *
 * @param memoryRoot The session memory root to inspect.
 * @returns A stable hash of the journey and topic-file contents.
 * @throws {Error} When a listed topic file cannot be read.
 */
export function durableMemoryFingerprint(memoryRoot: string): string {
	const digest = createHash("sha256");
	const topics = listTopics(memoryRoot);
	const journey = readJourney(memoryRoot) ?? "";
	digest.update(`JOURNEY.md\0${journey}\0`);
	for (const topic of topics) {
		digest.update(`${topic.filename}\0${readFileSync(join(memoryRoot, topic.filename), "utf-8")}\0`);
	}
	return digest.digest("hex");
}

function buildConsolidatorPrompt(memoryRoot: string, promote: Observation[], journeyTargetTokens: number): string {
	const indexText = renderIndexFile(listTopics(memoryRoot));
	const journeyText = readJourney(memoryRoot);
	const journeyWords = Math.round((journeyTargetTokens * 3) / 4);
	const obsLines = sortObservations(promote).map(observationToLine).join("\n");
	return (
		`Current local time: ${nowTimestamp()}\n\n` +
		"You are folding the observations below into the durable topic files under .memory/. " +
		"Use this exact time string in the `updated` front-matter of any file you write, and in any new JOURNEY.md entry.\n\n" +
		"===== CURRENT MEMORY INDEX (generated; do not edit INDEX.md) =====\n" +
		`${indexText}\n` +
		"===== END MEMORY INDEX =====\n\n" +
		"===== CURRENT JOURNEY (.memory/JOURNEY.md — the running descriptive project history) =====\n" +
		`${journeyText ?? "(empty — no journey yet; start one)"}\n` +
		"===== END JOURNEY =====\n\n" +
		"===== OBSERVATIONS TO CONSOLIDATE (each line is `<timestamp-id>  <content>`) =====\n" +
		`${obsLines}\n` +
		"===== END OBSERVATIONS =====\n\n" +
		"Fold every observation above into topic files (create/merge/rewrite as needed). Then update " +
		`.memory/JOURNEY.md per your instructions — keep it under ~${journeyTargetTokens} tokens (~${journeyWords} words), ` +
		"purely descriptive, no advice or next steps. Finish with a one-sentence confirmation."
	);
}

export function evaluateConsolidatorTrigger(pi: ExtensionAPI, runtime: Runtime, ctx: TriggerCtx): void {
	if (!runtime.enabled || runtime.config.passive) return;
	if (runtime.consolidatorInFlight) return;

	const branch = ctx.sessionManager.getBranch();
	const active = foldLedger(branch).activeObservations;
	if (poolTokens(active) < runtime.config.consolidateAtPoolTokens) return;

	const { promote } = selectPromotionOverflow(active, runtime.config.poolTargetTokens);
	if (promote.length === 0) return;

	runtime.consolidatorInFlight = true;
	if (ctx.hasUI) {
		ctx.ui?.notify(`om: consolidator started (${promote.length} obs, ~${poolTokens(promote).toLocaleString()} tok)`, "info");
	}
	// Deliberately NOT tracked in observerTasks: compaction waits only for in-flight observers,
	// never the consolidator (design R5). The consolidatorInFlight flag enforces one-at-a-time.
	void dispatchConsolidator(pi, runtime, ctx, promote);
}

async function dispatchConsolidator(
	pi: ExtensionAPI,
	runtime: Runtime,
	ctx: TriggerCtx,
	promote: Observation[],
): Promise<void> {
	const runId = nextRunId();
	const controller = new AbortController();
	runtime.consolidatorController = controller;
	runtime.status.workerStart("consolidator", runId);

	try {
		const prompt = buildConsolidatorPrompt(runtime.memoryRoot, promote, runtime.config.journeyTargetTokens);
		const argv = buildWorkerArgv({
			model: runtime.config.models.consolidator,
			sessionName: `om-consolidator-${runId}`,
			kickoffPrompt: prompt,
		});
		const env = buildWorkerEnv("consolidator", { memoryRoot: runtime.memoryRoot, runId });
		const before = durableMemoryFingerprint(runtime.memoryRoot);
		const exit = await spawnWorker({ argv, cwd: runtime.memoryRoot, env, signal: controller.signal });
		if (runtime.consolidatorController !== controller || !isContextActive(ctx)) return;
		// Capture cost before the exit-code check so a partial run's spend is still recorded.
		recordWorkerCost(pi, runtime, ctx, "consolidator", runId);
		if (exit.code !== 0) {
			throw new Error(`consolidator exited with code ${exit.code}${exit.stderr ? `: ${exit.stderr.trim().slice(0, 200)}` : ""}`);
		}
		if (durableMemoryFingerprint(runtime.memoryRoot) === before) {
			throw new Error("consolidator exited without changing durable memory");
		}

		const branch = ctx.sessionManager.getBranch();
		const stillActive = new Set(foldLedger(branch).activeObservations.map((o) => o.timestamp));
		const toDrop = promote.map((o) => o.timestamp).filter((t) => stillActive.has(t));

		if (toDrop.length > 0) {
			const coversUpToId = lastSourceEntryId(branch);
			if (coversUpToId) {
				pi.appendEntry(OM_OBSERVATIONS_DROPPED, { observationTimestamps: toDrop, coversUpToId });
			}
		}

		// Re-render INDEX.md so live ls/grep truth leads the pushed map (design risk 3).
		atomicWrite(indexPath(runtime.memoryRoot), renderIndexFile(listTopics(runtime.memoryRoot)));

		runtime.status.workerDone(runId, toDrop.length);
		runtime.refreshFooterGauges(ctx.sessionManager.getBranch(), ctx.getContextUsage?.()?.tokens ?? null);
		if (hasActiveUI(ctx) && ctx.ui) {
			runtime.queueToast(`om: consolidator promoted ${toDrop.length} obs`, "info", ctx.ui.notify.bind(ctx.ui));
		}
	} catch (error) {
		if (runtime.consolidatorController !== controller) return;
		const message = error instanceof Error ? error.message : String(error);
		runtime.lastWorkerError = message;
		runtime.status.workerError(runId);
		if (hasActiveUI(ctx)) ctx.ui?.notify(`om: consolidator failed: ${message}`, "error");
	} finally {
		if (runtime.consolidatorController === controller) {
			runtime.consolidatorController = undefined;
			runtime.consolidatorInFlight = false;
		}
	}
}

export function registerConsolidatorTrigger(pi: ExtensionAPI, runtime: Runtime): void {
	const handler = (_event: unknown, ctx: TriggerCtx) => evaluateConsolidatorTrigger(pi, runtime, ctx);
	pi.on("turn_end", handler as never);
	pi.on("agent_start", handler as never);
}
