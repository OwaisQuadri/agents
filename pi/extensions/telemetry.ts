import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

type TelemetryStatus = "succeeded" | "failed" | "cancelled";

type FeedbackValue = "accepted" | "corrected" | "rejected";

type TokenUsage = {
	input: number | null;
	output: number | null;
	cacheRead: number | null;
	cacheWrite: number | null;
};

type RunRecord = {
	recordType: "run";
	runId: string;
	parentRunId: string | null;
	packageName: string;
	packageVersion: string;
	agentName: string | null;
	startedAt: string;
	settledAt: string;
	durationMs: number;
	status: TelemetryStatus;
	tokens: TokenUsage;
	costUsd: number | null;
};

type FeedbackRecord = {
	recordType: "feedback";
	runId: string;
	value: FeedbackValue;
	createdAt: string;
};

type TelemetryRecord = RunRecord | FeedbackRecord;

type TelemetryFilter = {
	packageName?: string;
	packageVersion?: string;
	agentName?: string;
	status?: TelemetryStatus;
	minimumDurationMs?: number;
	maximumCostUsd?: number;
	feedback?: FeedbackValue;
};

type TelemetryCounts = {
	active: number;
	failed: number;
};

type TelemetryStore = {
	path: string;
	records: TelemetryRecord[];
};

type TelemetryRuntime = {
	activeRuns: Map<string, { startedAt: string; packageName: string; agentName: string | null }>;
	store: TelemetryStore;
};

type TelemetryExtension = (pi: ExtensionAPI) => Promise<void>;

function loadStore(path: string): Promise<TelemetryStore> {
	throw new Error("unimplemented");
}

function appendRecord(store: TelemetryStore, record: TelemetryRecord): Promise<void> {
	throw new Error("unimplemented");
}

/**
 * Filters settled run records through the approved drill-down dimensions.
 *
 * @param store - The loaded content-free telemetry store.
 * @param filter - The package, agent, status, duration, cost, or feedback constraints.
 * @returns Matching settled runs in storage order.
 * @throws An error when a stored feedback record refers to a missing run.
 */
export function filterRuns(store: TelemetryStore, filter: TelemetryFilter): RunRecord[] {
	throw new Error("unimplemented");
}

/**
 * Counts active runtime entries and failed stored runs.
 *
 * @param runtime - The active-run map and loaded telemetry store.
 * @returns The default active and failed counts.
 * @throws An error when stored records violate the telemetry schema.
 */
export function telemetryCounts(runtime: TelemetryRuntime): TelemetryCounts {
	throw new Error("unimplemented");
}

function startRun(
	runtime: TelemetryRuntime,
	runId: string,
	packageName: string,
	agentName: string | null,
	startedAt: string,
): void {
	throw new Error("unimplemented");
}

function settleRun(
	runtime: TelemetryRuntime,
	runId: string,
	parentRunId: string | null,
	packageVersion: string,
	status: TelemetryStatus,
	tokens: TokenUsage,
	costUsd: number | null,
	settledAt: string,
): Promise<RunRecord> {
	throw new Error("unimplemented");
}

function attachFeedback(
	runtime: TelemetryRuntime,
	runId: string,
	value: FeedbackValue,
	createdAt: string,
): Promise<FeedbackRecord> {
	throw new Error("unimplemented");
}

function registerCommands(pi: ExtensionAPI, runtime: TelemetryRuntime): void {
	throw new Error("unimplemented");
}

function registerLifecycle(pi: ExtensionAPI, runtime: TelemetryRuntime): void {
	throw new Error("unimplemented");
}

/**
 * Registers private wide-event telemetry for Pi and pi-subagents runs.
 *
 * @param pi - The Pi extension interface that supplies lifecycle events, commands, and status output.
 * @returns A promise that resolves after the local store loads and handlers register.
 * @throws An error when the telemetry store cannot be read or validated.
 */
const telemetryExtension: TelemetryExtension = async (pi) => {
	throw new Error("unimplemented");
};

export default telemetryExtension;
