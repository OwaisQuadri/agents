/**
 * File-based IPC between the guard extension and the headless judge subprocess it
 * spawns, one span at a time. Mirrors observational-memory's own worker IPC
 * (`pi/extensions/observational-memory/src/spawn/runs.ts`) — a subprocess cannot
 * return a value in-process, so it writes its verdict to a transient result file the
 * orchestrator reads after the process exits.
 */
import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";

import { stateDir } from "../cache.ts";

export function runsDir(home?: string): string {
	return join(stateDir(home), "runs");
}

export function runResultPath(runId: string, home?: string): string {
	return join(runsDir(home), `${runId}.result.json`);
}

export type WorkerVerdictResult = {
	shape: string; // a whitelist shape name, or "none"
	reason: string;
};

/** Atomic write (temp + rename) so a reader never sees a half-written file. */
export function atomicWrite(path: string, content: string): void {
	mkdirSync(dirname(path), { recursive: true });
	const tmp = `${path}.tmp-${process.pid}-${Date.now()}`;
	writeFileSync(tmp, content, "utf-8");
	renameSync(tmp, path);
}

export function writeWorkerVerdict(path: string, result: WorkerVerdictResult): void {
	atomicWrite(path, JSON.stringify(result));
}

/** Read a completed result; missing or malformed results are not decisions. */
export function readWorkerVerdict(path: string): WorkerVerdictResult | undefined {
	if (!existsSync(path)) return undefined;
	try {
		const raw = JSON.parse(readFileSync(path, "utf-8")) as unknown;
		if (!raw || typeof raw !== "object") return undefined;
		const r = raw as Record<string, unknown>;
		if (Object.keys(r).sort().join(",") !== "reason,shape" || typeof r.shape !== "string" || typeof r.reason !== "string" || !r.reason.trim() || r.reason.length > 2000) return undefined;
		return { shape: r.shape, reason: r.reason };
	} catch {
		return undefined;
	}
}
