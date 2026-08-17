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

type TelemetryExtension = (pi: ExtensionAPI) => void;
