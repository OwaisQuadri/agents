import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { randomUUID } from "node:crypto";
import { appendFile, lstat, mkdir, readFile, realpath } from "node:fs/promises";
import { homedir } from "node:os";
import { dirname, isAbsolute, join, relative, resolve } from "node:path";

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

type ActiveRunState = {
	startedAt: string;
	packageName: string;
	parentRunId: string | null;
	agentName: string | null;
};

type TelemetryRuntime = {
	activeRuns: Map<string, ActiveRunState>;
	store: TelemetryStore;
	packageName: string;
	packageVersion: string;
	currentParentRunId: string | null;
};

type PendingTelemetryMutations = {
	settlementRunIds: Set<string>;
	feedbackRunIds: Set<string>;
};

const pendingTelemetryMutations = new WeakMap<TelemetryRuntime, PendingTelemetryMutations>();

type NormalizedMetrics = {
	tokens: TokenUsage;
	costUsd: number | null;
};

type TelemetryExtension = (pi: ExtensionAPI) => Promise<void>;

type CompletionEvent = {
	runId?: unknown;
	id?: unknown;
	source?: unknown;
	agent?: unknown;
	success?: unknown;
	state?: unknown;
	cancelled?: unknown;
	interrupted?: unknown;
	stopped?: unknown;
	timedOut?: unknown;
	turnBudgetExceeded?: unknown;
	timestamp?: unknown;
	durationMs?: unknown;
	totalCost?: unknown;
	inputTokens?: unknown;
	outputTokens?: unknown;
	costUsd?: unknown;
	cacheRead?: unknown;
	cacheWrite?: unknown;
};

type AsyncStartedEvent = {
	id?: unknown;
	agent?: unknown;
	agents?: unknown;
};

type ShutdownEvent = {
	type?: unknown;
	reason?: unknown;
};

const TELEMETRY_PACKAGE_NAME = "@earendil-works/pi-coding-agent";
const TELEMETRY_PACKAGE_VERSION = "0.84.2";
const PinnedSubagentPackageName = "pi-subagents";
const PinnedSubagentPackageVersion = "0.50.0";

const runRecordKeys = [
	"recordType",
	"runId",
	"parentRunId",
	"packageName",
	"packageVersion",
	"agentName",
	"startedAt",
	"settledAt",
	"durationMs",
	"status",
	"tokens",
	"costUsd",
] as const;

const tokenUsageKeys = ["input", "output", "cacheRead", "cacheWrite"] as const;
const feedbackRecordKeys = ["recordType", "runId", "value", "createdAt"] as const;
const totalCostKeys = ["inputTokens", "outputTokens", "costUsd"] as const;
const telemetryFilterKeys = ["packageName", "packageVersion", "agentName", "status", "minimumDurationMs", "maximumCostUsd", "feedback"] as const;

function telemetryRootPath(): string {
	const configuredDirectory = process.env.PI_CODING_AGENT_DIR?.trim();
	return resolve(configuredDirectory && configuredDirectory.length > 0 ? configuredDirectory : join(homedir(), ".pi", "agent"));
}

function telemetryStorePath(): string {
	return join(telemetryRootPath(), "telemetry.jsonl");
}

function hasParentTraversal(path: string): boolean {
	return path.split(/[\\/]+/).includes("..");
}

function isWithinRoot(path: string, root: string): boolean {
	const relativePath = relative(root, path);
	return relativePath !== "" && !relativePath.startsWith("..") && !isAbsolute(relativePath);
}

function isNoSuchFileError(error: unknown): boolean {
	return error instanceof Error && "code" in error && (error as { code?: string }).code === "ENOENT";
}

async function validateTelemetryStorePath(path: string): Promise<string> {
	if (hasParentTraversal(path)) {
		throw new Error("telemetry store path must not contain parent traversal");
	}

	const rootPath = telemetryRootPath();
	const targetPath = resolve(path);

	if (!isWithinRoot(targetPath, rootPath)) {
		throw new Error("telemetry store path must stay within the configured root");
	}

	let allowedRoot = rootPath;

	try {
		const rootStat = await lstat(rootPath);
		if (rootStat.isSymbolicLink()) {
			throw new Error("telemetry store root must not be a symlink");
		}
		allowedRoot = await realpath(rootPath);
	} catch (error) {
		if (!isNoSuchFileError(error)) {
			throw error;
		}
	}

	const relativePath = relative(rootPath, targetPath);
	const segments = relativePath.split(/[\\/]+/).filter(Boolean);
	let currentPath = rootPath;

	for (let index = 0; index < segments.length - 1; index++) {
		currentPath = join(currentPath, segments[index]!);

		try {
			await lstat(currentPath);
		} catch (error) {
			if (isNoSuchFileError(error)) {
				return targetPath;
			}
			throw error;
		}

		const realCurrentPath = await realpath(currentPath);
		if (!isWithinRoot(realCurrentPath, allowedRoot)) {
			throw new Error("telemetry store path escapes the configured root");
		}
	}

	try {
		const targetStat = await lstat(targetPath);
		if (targetStat.isSymbolicLink()) {
			throw new Error("telemetry store target must not be a symlink");
		}

		const realTargetPath = await realpath(targetPath);
		if (!isWithinRoot(realTargetPath, allowedRoot)) {
			throw new Error("telemetry store path escapes the configured root");
		}
	} catch (error) {
		if (!isNoSuchFileError(error)) {
			throw error;
		}
	}

	return targetPath;
}

function isObject(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isExactKeys(value: Record<string, unknown>, keys: readonly string[]): boolean {
	const actualKeys = Object.keys(value);
	return actualKeys.length === keys.length && keys.every((key) => Object.prototype.hasOwnProperty.call(value, key));
}

function isString(value: unknown): value is string {
	return typeof value === "string";
}

function isNonEmptyString(value: unknown): value is string {
	return isString(value) && value.trim().length > 0;
}

function isNullableString(value: unknown): value is string | null {
	return value === null || isNonEmptyString(value);
}

function isFiniteNumber(value: unknown): value is number {
	return typeof value === "number" && Number.isFinite(value);
}

function isFiniteNonNegativeNumber(value: unknown): value is number {
	return isFiniteNumber(value) && value >= 0;
}

function isNullableNumber(value: unknown): value is number | null {
	return value === null || isFiniteNonNegativeNumber(value);
}

function isTokenUsage(value: unknown): value is TokenUsage {
	if (!isObject(value) || !isExactKeys(value, tokenUsageKeys)) {
		return false;
	}

	return (
		isNullableNumber(value.input) &&
		isNullableNumber(value.output) &&
		isNullableNumber(value.cacheRead) &&
		isNullableNumber(value.cacheWrite)
	);
}

function normalizeTimestamp(value: unknown, fieldName: string): string {
	if (!isString(value) || value.trim().length === 0) {
		throw new Error(`${fieldName} must be a valid ISO timestamp`);
	}

	const timestamp = Date.parse(value);
	if (!Number.isFinite(timestamp)) {
		throw new Error(`${fieldName} must be a valid ISO timestamp`);
	}

	return new Date(timestamp).toISOString();
}

function normalizeEventTimestamp(value: unknown, fieldName: string): string {
	if (isString(value)) {
		return normalizeTimestamp(value, fieldName);
	}

	if (isFiniteNonNegativeNumber(value)) {
		return new Date(value).toISOString();
	}

	throw new Error(`${fieldName} must be a valid ISO timestamp or non-negative epoch milliseconds`);
}

function isTelemetryStatus(value: unknown): value is TelemetryStatus {
	return value === "succeeded" || value === "failed" || value === "cancelled";
}

function isFeedbackValue(value: unknown): value is FeedbackValue {
	return value === "accepted" || value === "corrected" || value === "rejected";
}

function emptyTokenUsage(): TokenUsage {
	return {
		input: null,
		output: null,
		cacheRead: null,
		cacheWrite: null,
	};
}

function isRunRecord(value: unknown): value is RunRecord {
	if (!isObject(value) || !isExactKeys(value, runRecordKeys)) {
		return false;
	}

	const tokens = value.tokens;
	const startedAt = Date.parse(value.startedAt as string);
	const settledAt = Date.parse(value.settledAt as string);

	return (
		value.recordType === "run" &&
		isNonEmptyString(value.runId) &&
		isNullableString(value.parentRunId) &&
		isNonEmptyString(value.packageName) &&
		isNonEmptyString(value.packageVersion) &&
		isNullableString(value.agentName) &&
		Number.isFinite(startedAt) &&
		Number.isFinite(settledAt) &&
		settledAt >= startedAt &&
		value.durationMs === settledAt - startedAt &&
		isTelemetryStatus(value.status) &&
		isTokenUsage(tokens) &&
		isNullableNumber(value.costUsd)
	);
}

function isFeedbackRecord(value: unknown): value is FeedbackRecord {
	if (!isObject(value) || !isExactKeys(value, feedbackRecordKeys)) {
		return false;
	}

	return (
		value.recordType === "feedback" &&
		isNonEmptyString(value.runId) &&
		isFeedbackValue(value.value) &&
		Number.isFinite(Date.parse(value.createdAt as string))
	);
}

function validateTelemetryRecord(value: unknown, lineNumber?: number): TelemetryRecord {
	if (isRunRecord(value) || isFeedbackRecord(value)) {
		return value;
	}

	if (lineNumber === undefined) {
		throw new Error("telemetry record does not match the closed schema");
	}

	throw new Error(`telemetry record line ${lineNumber} does not match the closed schema`);
}

function validateFilterBoundary(value: number | undefined, fieldName: string): void {
	if (value === undefined) {
		return;
	}

	if (!Number.isFinite(value) || value < 0) {
		throw new RangeError(`${fieldName} must be a non-negative number`);
	}
}

function buildFeedbackIndex(records: TelemetryRecord[]): Map<string, Set<FeedbackValue>> {
	const settledRunIds = new Set<string>();

	for (const record of records) {
		if (record.recordType === "run") {
			if (settledRunIds.has(record.runId)) {
				throw new Error(`telemetry run record runId ${record.runId} already exists`);
			}
			settledRunIds.add(record.runId);
		}
	}

	const feedbackIndex = new Map<string, Set<FeedbackValue>>();

	for (const record of records) {
		if (record.recordType !== "feedback") {
			continue;
		}

		if (!settledRunIds.has(record.runId)) {
			throw new Error(`telemetry feedback record run ${record.runId} has no settled run`);
		}

		if (feedbackIndex.has(record.runId)) {
			throw new Error(`telemetry feedback record run ${record.runId} already exists`);
		}

		feedbackIndex.set(record.runId, new Set([record.value]));
	}

	return feedbackIndex;
}

function isRunMatch(run: RunRecord, filter: TelemetryFilter, feedbackIndex: Map<string, Set<FeedbackValue>>): boolean {
	if (filter.packageName !== undefined && run.packageName !== filter.packageName) {
		return false;
	}

	if (filter.packageVersion !== undefined && run.packageVersion !== filter.packageVersion) {
		return false;
	}

	if (filter.agentName !== undefined && run.agentName !== filter.agentName) {
		return false;
	}

	if (filter.status !== undefined && run.status !== filter.status) {
		return false;
	}

	if (filter.minimumDurationMs !== undefined && run.durationMs < filter.minimumDurationMs) {
		return false;
	}

	if (filter.maximumCostUsd !== undefined) {
		if (run.costUsd === null || run.costUsd > filter.maximumCostUsd) {
			return false;
		}
	}

	if (filter.feedback !== undefined) {
		const feedbackValues = feedbackIndex.get(run.runId);

		if (feedbackValues === undefined || !feedbackValues.has(filter.feedback)) {
			return false;
		}
	}

	return true;
}

/**
 * Loads the telemetry store from a JSON Lines file.
 *
 * @param path - Optional file path for the store. Defaults to the Pi user directory telemetry file.
 * @returns The store path and validated telemetry records.
 * @throws An error when the file cannot be read, parsed, or validated.
 */
export async function loadStore(path = telemetryStorePath()): Promise<TelemetryStore> {
	const validatedPath = await validateTelemetryStorePath(path);
	let contents: string;

	try {
		contents = await readFile(validatedPath, "utf8");
	} catch (error) {
		if (error instanceof Error && "code" in error && (error as { code?: string }).code === "ENOENT") {
			return { path, records: [] };
		}
		throw error;
	}

	const records: TelemetryRecord[] = [];
	const lines = contents.split(/\r?\n/);

	while (lines.length > 0 && lines[lines.length - 1] === "") {
		lines.pop();
	}

	for (const [index, line] of lines.entries()) {
		if (line.trim().length === 0) {
			throw new Error(`telemetry record line ${index + 1} is empty`);
		}

		let parsed: unknown;

		try {
			parsed = JSON.parse(line);
		} catch {
			throw new Error(`telemetry record line ${index + 1} is not valid JSON`);
		}

		records.push(validateTelemetryRecord(parsed, index + 1));
	}

	buildFeedbackIndex(records);
	return { path, records };
}

/**
 * Appends a telemetry record to the store JSON Lines file.
 *
 * @param store - The telemetry store path and in-memory records.
 * @param record - A validated run or feedback record.
 * @returns A promise that resolves after the append succeeds and memory updates.
 * @throws An error when the record does not match the closed schema or the append fails.
 */
export async function appendRecord(store: TelemetryStore, record: TelemetryRecord): Promise<void> {
	const validatedRecord = validateTelemetryRecord(record);
	const validatedPath = await validateTelemetryStorePath(store.path);
	await mkdir(dirname(validatedPath), { recursive: true });
	await appendFile(validatedPath, `${JSON.stringify(validatedRecord)}\n`, "utf8");
	store.records.push(validatedRecord);
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
	validateFilterBoundary(filter.minimumDurationMs, "minimumDurationMs");
	validateFilterBoundary(filter.maximumCostUsd, "maximumCostUsd");

	const feedbackIndex = buildFeedbackIndex(store.records);
	const matchingRuns: RunRecord[] = [];

	for (const record of store.records) {
		if (record.recordType === "run" && isRunMatch(record, filter, feedbackIndex)) {
			matchingRuns.push(record);
		}
	}

	return matchingRuns;
}

/**
 * Counts active runtime entries and failed stored runs.
 *
 * @param runtime - The active-run map and loaded telemetry store.
 * @returns The default active and failed counts.
 * @throws An error when stored records violate the telemetry schema.
 */
export function telemetryCounts(runtime: TelemetryRuntime): TelemetryCounts {
	let failed = 0;

	for (const record of runtime.store.records) {
		if (record.recordType === "run" && record.status === "failed") {
			failed++;
		}
	}

	return {
		active: runtime.activeRuns.size,
		failed,
	};
}

export function createTelemetryRuntime(store: TelemetryStore): TelemetryRuntime {
	return {
		activeRuns: new Map(),
		store,
		packageName: TELEMETRY_PACKAGE_NAME,
		packageVersion: TELEMETRY_PACKAGE_VERSION,
		currentParentRunId: null,
	};
}

function pendingMutations(runtime: TelemetryRuntime): PendingTelemetryMutations {
	let pending = pendingTelemetryMutations.get(runtime);
	if (pending === undefined) {
		pending = {
			settlementRunIds: new Set(),
			feedbackRunIds: new Set(),
		};
		pendingTelemetryMutations.set(runtime, pending);
	}
	return pending;
}

function ensureAvailableRunId(runtime: TelemetryRuntime, runId: string): void {
	if (runtime.activeRuns.has(runId) || runtime.store.records.some((record) => record.recordType === "run" && record.runId === runId)) {
		throw new Error(`telemetry run ${runId} already exists`);
	}
}

function validateActiveRunInputs(runId: string, packageName: string, agentName: string | null, startedAt: string): string {
	if (!isNonEmptyString(runId)) {
		throw new Error("runId must be a non-empty identifier");
	}

	if (!isNonEmptyString(packageName)) {
		throw new Error("packageName must be a non-empty identifier");
	}

	if (agentName !== null && !isNonEmptyString(agentName)) {
		throw new Error("agentName must be a non-empty identifier or null");
	}

	return normalizeTimestamp(startedAt, "startedAt");
}

function ensureActiveRun(runtime: TelemetryRuntime, runId: string): ActiveRunState {
	const active = runtime.activeRuns.get(runId);
	if (active === undefined) {
		throw new Error(`telemetry run ${runId} is missing`);
	}
	return active;
}

function resolveCompletionRunId(event: CompletionEvent): string {
	const runId = event.runId ?? event.id;
	if (!isNonEmptyString(runId)) {
		throw new Error("runId must be a non-empty identifier");
	}
	return runId;
}

function requireOptionalBoolean(value: unknown, fieldName: string): void {
	if (value !== undefined && typeof value !== "boolean") {
		throw new Error(`${fieldName} must be a boolean`);
	}
}

function resolveCompletionStatus(event: CompletionEvent): TelemetryStatus {
	if (event.source !== undefined && event.source !== "async" && event.source !== "foreground") {
		throw new Error("completion source must be async or foreground");
	}

	for (const field of ["success", "cancelled", "interrupted", "stopped", "timedOut", "turnBudgetExceeded"] as const) {
		requireOptionalBoolean(event[field], field);
	}

	if (event.state !== undefined && !["complete", "completed", "failed", "paused", "stopped", "cancelled"].includes(String(event.state))) {
		throw new Error("state must be complete, completed, failed, paused, stopped, or cancelled");
	}

	if (event.cancelled === true || event.interrupted === true || event.stopped === true || event.timedOut === true || event.turnBudgetExceeded === true || event.state === "paused" || event.state === "stopped" || event.state === "cancelled") {
		return "cancelled";
	}

	if (event.success === false || event.state === "failed") {
		return "failed";
	}

	return "succeeded";
}

function resolveCompletionSettledAt(event: CompletionEvent, active: ActiveRunState): string {
	if (event.timestamp !== undefined) {
		return normalizeEventTimestamp(event.timestamp, "timestamp");
	}

	if (event.durationMs !== undefined) {
		if (!isFiniteNonNegativeNumber(event.durationMs)) {
			throw new Error("durationMs must be a non-negative finite number");
		}
		const startedAt = Date.parse(active.startedAt);
		return new Date(startedAt + event.durationMs).toISOString();
	}

	return new Date().toISOString();
}

function normalizeNullableMetric(value: unknown, fieldName: string): number | null {
	if (value === undefined || value === null) {
		return null;
	}
	if (!isFiniteNonNegativeNumber(value)) {
		throw new Error(`${fieldName} must be a finite non-negative number or null`);
	}
	return value;
}

function normalizeMetrics(value: CompletionEvent): NormalizedMetrics {
	const metricsSource = value.totalCost === undefined || value.totalCost === null ? value : value.totalCost;

	if (metricsSource !== value) {
		if (!isObject(metricsSource)) {
			throw new Error("totalCost must contain only inputTokens, outputTokens, and costUsd");
		}

		for (const key of Object.keys(metricsSource)) {
			if (!totalCostKeys.includes(key as (typeof totalCostKeys)[number])) {
				throw new Error("totalCost must contain only inputTokens, outputTokens, and costUsd");
			}
		}
	}

	const inputTokens = metricsSource === value ? value.inputTokens : metricsSource.inputTokens;
	const outputTokens = metricsSource === value ? value.outputTokens : metricsSource.outputTokens;
	const costUsd = metricsSource === value ? value.costUsd : metricsSource.costUsd;

	return {
		tokens: {
			input: normalizeNullableMetric(inputTokens, "inputTokens"),
			output: normalizeNullableMetric(outputTokens, "outputTokens"),
			cacheRead: normalizeNullableMetric(value.cacheRead, "cacheRead"),
			cacheWrite: normalizeNullableMetric(value.cacheWrite, "cacheWrite"),
		},
		costUsd: normalizeNullableMetric(costUsd, "costUsd"),
	};
}

function resolveStoredPackageVersion(runtime: TelemetryRuntime, packageName: string): string {
	if (packageName === runtime.packageName) {
		return runtime.packageVersion;
	}

	if (packageName === PinnedSubagentPackageName) {
		return PinnedSubagentPackageVersion;
	}

	return "";
}

type SettledRunPreparation = {
	active: ActiveRunState;
	record: RunRecord;
};

function prepareSettledRun(
	runtime: TelemetryRuntime,
	runId: string,
	parentRunId: string | null,
	packageVersion: string,
	status: TelemetryStatus,
	tokens: TokenUsage,
	costUsd: number | null,
	settledAt: string,
): SettledRunPreparation {
	if (!isNonEmptyString(runId)) {
		throw new Error("runId must be a non-empty identifier");
	}

	if (parentRunId !== null && !isNonEmptyString(parentRunId)) {
		throw new Error("parentRunId must be a non-empty identifier or null");
	}

	if (!isNonEmptyString(packageVersion)) {
		throw new Error("packageVersion must be a non-empty identifier");
	}

	if (!isTelemetryStatus(status)) {
		throw new Error("status must be succeeded, failed, or cancelled");
	}

	if (!isTokenUsage(tokens) || !isNullableNumber(costUsd)) {
		throw new Error("metrics tokens must contain finite non-negative counts or null");
	}

	const active = ensureActiveRun(runtime, runId);
	const expectedPackageVersion = resolveStoredPackageVersion(runtime, active.packageName);
	if (expectedPackageVersion !== "" && expectedPackageVersion !== packageVersion) {
		throw new Error(`telemetry run ${runId} package version does not match`);
	}

	if (active.parentRunId !== parentRunId) {
		throw new Error(`telemetry run ${runId} parent run does not match`);
	}

	const normalizedSettledAt = normalizeTimestamp(settledAt, "settledAt");
	const startedAtMs = Date.parse(active.startedAt);
	const settledAtMs = Date.parse(normalizedSettledAt);
	if (settledAtMs < startedAtMs) {
		throw new Error("settledAt must not be earlier than startedAt");
	}

	return {
		active,
		record: {
			recordType: "run",
			runId,
			parentRunId,
			packageName: active.packageName,
			packageVersion,
			agentName: active.agentName,
			startedAt: active.startedAt,
			settledAt: normalizedSettledAt,
			durationMs: settledAtMs - startedAtMs,
			status,
			tokens,
			costUsd,
		},
	};
}

function finalizeSettledRun(runtime: TelemetryRuntime, runId: string): void {
	runtime.activeRuns.delete(runId);
	if (runtime.currentParentRunId === runId) {
		runtime.currentParentRunId = null;
	}
}

async function appendSettledRun(runtime: TelemetryRuntime, record: RunRecord): Promise<RunRecord> {
	const pending = pendingMutations(runtime);
	if (pending.settlementRunIds.has(record.runId)) {
		throw new Error(`telemetry run ${record.runId} settlement is already pending`);
	}

	pending.settlementRunIds.add(record.runId);
	try {
		await appendRecord(runtime.store, record);
		finalizeSettledRun(runtime, record.runId);
		return record;
	} finally {
		pending.settlementRunIds.delete(record.runId);
	}
}

/**
 * Starts an in-memory telemetry run.
 *
 * @param runtime - The active-run runtime and loaded telemetry store.
 * @param runId - The opaque run identifier.
 * @param packageName - The package name to store.
 * @param agentName - The agent name to store, or null for parent runs.
 * @param startedAt - The start timestamp.
 * @returns Nothing.
 * @throws When the run id is missing, duplicated, or the timestamp is invalid.
 */
export function startRun(
	runtime: TelemetryRuntime,
	runId: string,
	packageName: string,
	agentName: string | null,
	startedAt: string,
): void {
	const normalizedStartedAt = validateActiveRunInputs(runId, packageName, agentName, startedAt);
	ensureAvailableRunId(runtime, runId);

	runtime.activeRuns.set(runId, {
		startedAt: normalizedStartedAt,
		packageName,
		parentRunId: runtime.currentParentRunId,
		agentName,
	});
}

/**
 * Settles an in-memory telemetry run and appends the closed record.
 *
 * @param runtime - The active-run runtime and loaded telemetry store.
 * @param runId - The opaque run identifier.
 * @param parentRunId - The opaque parent run identifier, or null for root runs.
 * @param packageVersion - The package version to store.
 * @param status - The terminal status.
 * @param tokens - The normalized token counts.
 * @param costUsd - The normalized cost value.
 * @param settledAt - The settlement timestamp.
 * @returns The settled run record.
 * @throws When the run is missing, duplicated, malformed, out of order, or the append fails.
 */
export function settleRun(
	runtime: TelemetryRuntime,
	runId: string,
	parentRunId: string | null,
	packageVersion: string,
	status: TelemetryStatus,
	tokens: TokenUsage,
	costUsd: number | null,
	settledAt: string,
): Promise<RunRecord> {
	const { record } = prepareSettledRun(runtime, runId, parentRunId, packageVersion, status, tokens, costUsd, settledAt);
	return appendSettledRun(runtime, record);
}

function parseRunId(value: AsyncStartedEvent): string {
	const runId = value.id;
	if (!isNonEmptyString(runId)) {
		throw new Error("runId must be a non-empty identifier");
	}
	return runId;
}

function parseAgentName(value: AsyncStartedEvent): string | null {
	if (value.agent !== undefined) {
		if (!isNonEmptyString(value.agent)) {
			throw new Error("agent must be a non-empty identifier");
		}
		return value.agent;
	}

	if (!Array.isArray(value.agents)) {
		return null;
	}

	if (value.agents.length === 0) {
		return null;
	}

	if (!value.agents.every(isNonEmptyString)) {
		throw new Error("agents must contain non-empty identifiers");
	}

	return value.agents[0] ?? null;
}

function settleRunFromCompletion(runtime: TelemetryRuntime, event: CompletionEvent): Promise<RunRecord> {
	const runId = resolveCompletionRunId(event);
	const active = ensureActiveRun(runtime, runId);
	const status = resolveCompletionStatus(event);
	const settledAt = resolveCompletionSettledAt(event, active);
	const metrics = normalizeMetrics(event);
	return settleRun(
		runtime,
		runId,
		active.parentRunId,
		resolveStoredPackageVersion(runtime, active.packageName),
		status,
		metrics.tokens,
		metrics.costUsd,
		settledAt,
	);
}

function recordForegroundCompletion(runtime: TelemetryRuntime, event: CompletionEvent): Promise<RunRecord> {
	const runId = resolveCompletionRunId(event);
	ensureAvailableRunId(runtime, runId);

	if (!isNonEmptyString(event.agent)) {
		throw new Error("agent must be a non-empty identifier");
	}

	const settledAt = normalizeEventTimestamp(event.timestamp, "timestamp");
	const status = resolveCompletionStatus(event);
	const metrics = normalizeMetrics(event);
	const parentRunId = runtime.currentParentRunId;
	const record: RunRecord = {
		recordType: "run",
		runId,
		parentRunId,
		packageName: PinnedSubagentPackageName,
		packageVersion: PinnedSubagentPackageVersion,
		agentName: event.agent,
		startedAt: settledAt,
		settledAt,
		durationMs: 0,
		status,
		tokens: metrics.tokens,
		costUsd: metrics.costUsd,
	};

	return appendRecord(runtime.store, record).then(() => record);
}

function settleParentRun(runtime: TelemetryRuntime, status: TelemetryStatus): Promise<RunRecord> {
	const runId = runtime.currentParentRunId;
	if (!runId) {
		return Promise.reject(new Error("telemetry parent run is missing"));
	}

	const active = ensureActiveRun(runtime, runId);
	return settleRun(
		runtime,
		runId,
		active.parentRunId,
		resolveStoredPackageVersion(runtime, active.packageName),
		status,
		emptyTokenUsage(),
		null,
		new Date().toISOString(),
	);
}

async function settleAllActiveRuns(runtime: TelemetryRuntime, status: TelemetryStatus): Promise<RunRecord[]> {
	const runIds = [...runtime.activeRuns.keys()];
	const settledAt = new Date().toISOString();
	const records: RunRecord[] = [];
	const failures: unknown[] = [];

	for (const runId of runIds) {
		const active = runtime.activeRuns.get(runId);
		if (active === undefined) {
			continue;
		}

		try {
			const record = await settleRun(
				runtime,
				runId,
				active.parentRunId,
				resolveStoredPackageVersion(runtime, active.packageName),
				status,
				emptyTokenUsage(),
				null,
				settledAt,
			);
			records.push(record);
		} catch (error) {
			failures.push(error);
		}
	}

	if (failures.length > 0) {
		throw new AggregateError(failures, `telemetry failed to settle ${failures.length} active run(s)`);
	}

	return records;
}

function parseShutdownEvent(event: ShutdownEvent): void {
	if (event.type !== undefined && event.type !== "session_shutdown") {
		throw new Error("shutdown event type must be session_shutdown");
	}

	if (event.reason !== undefined && event.reason !== "quit" && event.reason !== "reload" && event.reason !== "new" && event.reason !== "resume" && event.reason !== "fork") {
		throw new Error("shutdown reason must be quit, reload, new, resume, or fork");
	}
}

type TelemetryStatusTarget = {
	setStatus(key: string, text: string | undefined): void;
};

function telemetryStatusText(counts: TelemetryCounts): string | undefined {
	if (counts.active === 0 && counts.failed === 0) {
		return undefined;
	}

	return `active: ${counts.active} failed: ${counts.failed}`;
}

function syncTelemetryStatus(target: TelemetryStatusTarget | null, runtime: TelemetryRuntime): void {
	if (target === null) {
		return;
	}

	target.setStatus("telemetry", telemetryStatusText(telemetryCounts(runtime)));
}

function projectRunRecord(record: RunRecord): RunRecord {
	return {
		recordType: record.recordType,
		runId: record.runId,
		parentRunId: record.parentRunId,
		packageName: record.packageName,
		packageVersion: record.packageVersion,
		agentName: record.agentName,
		startedAt: record.startedAt,
		settledAt: record.settledAt,
		durationMs: record.durationMs,
		status: record.status,
		tokens: {
			input: record.tokens.input,
			output: record.tokens.output,
			cacheRead: record.tokens.cacheRead,
			cacheWrite: record.tokens.cacheWrite,
		},
		costUsd: record.costUsd,
	};
}

function parseTelemetryFilterArgument(args: string): TelemetryFilter {
	const trimmed = args.trim();
	if (trimmed.length === 0) {
		return {};
	}

	let parsed: unknown;

	try {
		parsed = JSON.parse(trimmed);
	} catch {
		throw new Error("telemetry filter arguments must be valid JSON");
	}

	if (!isObject(parsed) || Array.isArray(parsed)) {
		throw new Error("telemetry filter arguments must be a JSON object");
	}

	const filter: TelemetryFilter = {};

	for (const [key, value] of Object.entries(parsed)) {
		if (!telemetryFilterKeys.includes(key as (typeof telemetryFilterKeys)[number])) {
			throw new Error(`unknown telemetry filter key: ${key}`);
		}

		switch (key) {
			case "packageName":
			case "packageVersion":
			case "agentName":
				if (!isNonEmptyString(value)) {
					throw new Error(`${key} must be a non-empty string`);
				}
				filter[key] = value;
				break;
			case "status":
				if (!isTelemetryStatus(value)) {
					throw new Error("status must be succeeded, failed, or cancelled");
				}
				filter.status = value;
				break;
			case "minimumDurationMs":
				if (!isFiniteNonNegativeNumber(value)) {
					throw new Error("minimumDurationMs must be a non-negative finite number");
				}
				filter.minimumDurationMs = value;
				break;
			case "maximumCostUsd":
				if (!isFiniteNonNegativeNumber(value)) {
					throw new Error("maximumCostUsd must be a non-negative finite number");
				}
				filter.maximumCostUsd = value;
				break;
			case "feedback":
				if (!isFeedbackValue(value)) {
					throw new Error("feedback must be accepted, corrected, or rejected");
				}
				filter.feedback = value;
				break;
		}
	}

	return filter;
}

function parseFeedbackCommandArguments(args: string): { runId: string; value: FeedbackValue } {
	const trimmed = args.trim();
	if (trimmed.length === 0) {
		throw new Error("telemetry feedback command expects exactly runId and accepted|corrected|rejected");
	}

	const parts = trimmed.split(/\s+/);
	if (parts.length !== 2) {
		throw new Error("telemetry feedback command expects exactly runId and accepted|corrected|rejected");
	}

	const [runId, value] = parts;
	if (!isNonEmptyString(runId)) {
		throw new Error("runId must be a non-empty identifier");
	}

	if (!isFeedbackValue(value)) {
		throw new Error("feedback value must be accepted, corrected, or rejected");
	}

	return { runId, value };
}

export async function attachFeedback(
	runtime: TelemetryRuntime,
	runId: string,
	value: FeedbackValue,
	createdAt: string,
): Promise<FeedbackRecord> {
	if (!isNonEmptyString(runId)) {
		throw new Error("runId must be a non-empty identifier");
	}

	if (!isFeedbackValue(value)) {
		throw new Error("feedback value must be accepted, corrected, or rejected");
	}

	const normalizedCreatedAt = normalizeTimestamp(createdAt, "createdAt");
	const isSettledRun = runtime.store.records.some((record) => record.recordType === "run" && record.runId === runId);
	if (!isSettledRun) {
		throw new Error(`telemetry feedback run ${runId} has no settled run`);
	}

	const isFeedbackAttached = runtime.store.records.some((record) => record.recordType === "feedback" && record.runId === runId);
	if (isFeedbackAttached) {
		throw new Error(`telemetry feedback run ${runId} already exists`);
	}

	const pending = pendingMutations(runtime);
	if (pending.feedbackRunIds.has(runId)) {
		throw new Error(`telemetry feedback run ${runId} is already pending`);
	}

	const record: FeedbackRecord = {
		recordType: "feedback",
		runId,
		value,
		createdAt: normalizedCreatedAt,
	};

	pending.feedbackRunIds.add(runId);
	try {
		await appendRecord(runtime.store, record);
		return record;
	} finally {
		pending.feedbackRunIds.delete(runId);
	}
}

export function registerCommands(pi: ExtensionAPI, runtime: TelemetryRuntime): void {
	pi.registerCommand("telemetry-status", {
		description: "Show telemetry counts",
		handler: async (args, ctx) => {
			if (args.trim().length > 0) {
				throw new Error("telemetry-status takes no arguments");
			}

			const counts = telemetryCounts(runtime);
			ctx.ui.notify(`active: ${counts.active} failed: ${counts.failed}`);
		},
	});

	pi.registerCommand("telemetry-runs", {
		description: "List telemetry runs",
		handler: async (args, ctx) => {
			const runs = filterRuns(runtime.store, parseTelemetryFilterArgument(args)).map(projectRunRecord);
			ctx.ui.notify(JSON.stringify(runs));
		},
	});

	pi.registerCommand("telemetry-feedback", {
		description: "Attach telemetry feedback",
		handler: async (args) => {
			const { runId, value } = parseFeedbackCommandArguments(args);
			await attachFeedback(runtime, runId, value, new Date().toISOString());
		},
	});
}

/**
 * Registers private wide-event telemetry for Pi and pi-subagents runs.
 *
 * @param pi - The Pi extension interface that supplies lifecycle events, commands, and status output.
 * @returns A promise that resolves after the local store loads and handlers register.
 * @throws An error when the telemetry store cannot be read or validated.
 */
export function registerLifecycle(pi: ExtensionAPI, runtime: TelemetryRuntime): void {
	let statusTarget: TelemetryStatusTarget | null = null;

	pi.on("session_start", (_event, ctx) => {
		statusTarget = ctx.ui;
		syncTelemetryStatus(statusTarget, runtime);
	});

	pi.on("agent_start", async (_event, ctx) => {
		statusTarget = ctx.ui;
		try {
			const runId = randomUUID();
			startRun(runtime, runId, runtime.packageName, null, new Date().toISOString());
			runtime.currentParentRunId = runId;
		} finally {
			syncTelemetryStatus(statusTarget, runtime);
		}
	});

	pi.on("agent_settled", async (_event, ctx) => {
		statusTarget = ctx.ui;
		try {
			await settleParentRun(runtime, "succeeded");
		} finally {
			syncTelemetryStatus(statusTarget, runtime);
		}
	});

	pi.on("session_shutdown", async (event: ShutdownEvent, ctx) => {
		statusTarget = ctx.ui;
		try {
			parseShutdownEvent(event);
			await settleAllActiveRuns(runtime, "cancelled");
		} finally {
			syncTelemetryStatus(statusTarget, runtime);
		}
	});

	pi.events.on("subagent:async-started", async (payload: AsyncStartedEvent) => {
		try {
			const runId = parseRunId(payload);
			const agentName = parseAgentName(payload);
			startRun(runtime, runId, PinnedSubagentPackageName, agentName, new Date().toISOString());
		} finally {
			syncTelemetryStatus(statusTarget, runtime);
		}
	});

	pi.events.on("subagent:async-complete", async (payload: CompletionEvent) => {
		try {
			await settleRunFromCompletion(runtime, { ...payload, source: "async" });
		} finally {
			syncTelemetryStatus(statusTarget, runtime);
		}
	});
	pi.events.on("subagent:foreground-complete", async (payload: CompletionEvent) => {
		try {
			await recordForegroundCompletion(runtime, { ...payload, source: "foreground" });
		} finally {
			syncTelemetryStatus(statusTarget, runtime);
		}
	});
}

/**
 * Loads the telemetry store and registers lifecycle handlers.
 *
 * @param pi - The Pi extension interface that supplies lifecycle events, commands, and status output.
 * @returns A promise that resolves after the local store loads and handlers register.
 * @throws An error when the telemetry store cannot be read or validated.
 */
const telemetryExtension: TelemetryExtension = async (pi) => {
	const store = await loadStore();
	const runtime = createTelemetryRuntime(store);
	registerLifecycle(pi, runtime);
	registerCommands(pi, runtime);
};

export default telemetryExtension;
