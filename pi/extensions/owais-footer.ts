import type { ExtensionAPI, ExtensionContext, SessionEntry } from "@earendil-works/pi-coding-agent";
import { visibleWidth } from "@earendil-works/pi-tui";
import { homedir } from "node:os";
import { basename, relative } from "node:path";
import { resolveBranchPointCommit, type Exec } from "./live-diff/engine.ts";

const HERDR_WORKTREE_MARKER = "/.herdr/worktrees/";
const PR_POLL_INTERVAL_MS = 15 * 1000;
const BRAILLE_ORBIT = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const BRANCH_SUMMARY_MAX_COMMITS = 20;
const BRANCH_SUMMARY_MAX_WORKING_TREE_FILES = 20;
const BRANCH_SUMMARY_MAX_WIDTH = 40;
const BRANCH_SUMMARY_MAX_TRANSCRIPT_ASKS = 12;
const BRANCH_SUMMARY_MAX_TRANSCRIPT_ASK_WIDTH = 160;
export const NEW_SESSION_HEADLINE = "New session";
const BRANCH_SUMMARY_INSTRUCTIONS =
	"Reply with exactly one short noun phrase, 1 to 4 words total, naming the overall work described below. Never one phrase per commit or file. Present tense or no verb, never past tense. No markdown, no quotes, no trailing period, single line only.";

type RepositoryState =
	| { isGit: true; project: string; branch: string | undefined }
	| { isGit: false; path: string };

type ActivityKind = "working" | "delegated" | "ready";

type SubagentLifecycleEvent = {
	id?: unknown;
};

export type PullRequest = {
	url: string;
	number: number;
	state: string;
	isDraft: boolean;
	mergeStateStatus: string;
};

export type FooterSegmentId = "branch" | "headline" | "workspace" | "pr" | "status" | "provider" | "thinking" | "model";

export type FooterForm = {
	text: string;
	render: () => string;
};

export type FooterDegradableSegment = {
	id: FooterSegmentId;
	full: FooterForm;
	shorten?: FooterForm;
	truncate?: FooterForm;
};

// low-to-high priority: branch, headline, workspace, pr, status, provider, thinking, model. each pass
// (shorten, then truncate, then hide) touches low-priority items first and stops once the line fits.
export function assembleFooterLine(
	forms: Map<FooterSegmentId, FooterForm | undefined>,
	pick: (form: FooterForm) => string,
	dot: string,
	arrow: string,
): { left: string; right: string } {
	const get = (id: FooterSegmentId): string | undefined => {
		const form = forms.get(id);
		return form ? pick(form) : undefined;
	};
	const workspace = get("workspace");
	const branch = get("branch");
	const leftParts: string[] = [];
	if (workspace !== undefined && branch !== undefined) leftParts.push(`${workspace}${arrow}${branch}`);
	else if (workspace !== undefined) leftParts.push(workspace);
	else if (branch !== undefined) leftParts.push(branch);
	for (const id of ["pr", "headline", "status"] as const) {
		const text = get(id);
		if (text !== undefined) leftParts.push(text);
	}
	const left = leftParts.join(dot);

	const provider = get("provider");
	const thinking = get("thinking");
	const model = get("model");
	const right = model !== undefined
		? `${provider !== undefined ? `${provider}/` : ""}${model}${thinking !== undefined ? ` (${thinking})` : ""}`
		: "";
	return { left, right };
}

export function resolveFooterSegments(
	items: FooterDegradableSegment[],
	maxWidth: number,
): Map<FooterSegmentId, FooterForm | undefined> {
	const forms = new Map<FooterSegmentId, FooterForm | undefined>(items.map((item) => [item.id, item.full]));
	const fits = () => {
		const { left, right } = assembleFooterLine(forms, (form) => form.text, " · ", " > ");
		return visibleWidth(left) + visibleWidth(right) <= maxWidth;
	};
	if (fits()) return forms;
	for (const item of items) {
		if (fits()) break;
		if (item.shorten) forms.set(item.id, item.shorten);
	}
	if (fits()) return forms;
	for (const item of items) {
		if (fits()) break;
		if (item.truncate) forms.set(item.id, item.truncate);
	}
	if (fits()) return forms;
	for (const item of items) {
		if (fits()) break;
		forms.set(item.id, undefined);
	}
	return forms;
}

function compactPath(path: string | undefined): string {
	if (!path) return "unknown";
	const local = relative(homedir(), path);
	if (local === "") return "~";
	if (local.startsWith("..")) return path;
	const parts = local.split("/");
	return parts.length > 4 ? `…/${parts.slice(-4).join("/")}` : `~/${local}`;
}

export function projectName(cwd: string, repositoryRoot: string): string {
	const worktreeIndex = cwd.indexOf(HERDR_WORKTREE_MARKER);
	if (worktreeIndex >= 0) {
		const project = cwd.slice(worktreeIndex + HERDR_WORKTREE_MARKER.length).split("/").find(Boolean);
		if (project) return project;
	}
	return basename(repositoryRoot) || "unknown";
}

export function isPullRequest(value: unknown): value is PullRequest {
	if (value === null || typeof value !== "object") return false;
	const candidate = value as Partial<PullRequest>;
	return typeof candidate.url === "string" && typeof candidate.number === "number" &&
		typeof candidate.state === "string" && typeof candidate.isDraft === "boolean" &&
		typeof candidate.mergeStateStatus === "string";
}

export function pullRequestTone(pullRequest: PullRequest): "success" | "warning" | "error" | "purple" | "muted" {
	if (pullRequest.isDraft) return "muted";
	if (pullRequest.state === "MERGED") return "purple";
	if (pullRequest.state === "CLOSED" || pullRequest.state === "REJECTED") return "error";
	if (pullRequest.state !== "OPEN") return "muted";
	if (["CLEAN", "MERGEABLE"].includes(pullRequest.mergeStateStatus)) return "success";
	if (["DIRTY", "CONFLICT", "CONFLICTING"].includes(pullRequest.mergeStateStatus)) return "warning";
	return "muted";
}

function osc8(url: string, text: string): string {
	return `\x1b]8;;${url}\x1b\\\x1b[4m${text}\x1b[24m\x1b]8;;\x1b\\`;
}

function textForm(text: string, colorize: (text: string) => string): FooterForm {
	return { text, render: () => colorize(text) };
}

const BRANCH_COLORS = ["#82b8ff", "#8ee7f5", "#a6dca8", "#c7b5ff", "#d394ff"];

function branchColor(name: string): string {
	return BRANCH_COLORS[name.length % BRANCH_COLORS.length]!;
}

function blend(start: string, end: string, fraction: number): string {
	const amount = Math.max(0, Math.min(1, fraction));
	const channels = [1, 3, 5].map((offset) => {
		const from = Number.parseInt(start.slice(offset, offset + 2), 16);
		const to = Number.parseInt(end.slice(offset, offset + 2), 16);
		return Math.round(from + (to - from) * amount).toString(16).padStart(2, "0");
	});
	return `#${channels.join("")}`;
}

type QuotaState = {
	provider: string;
	usedPercent: number;
	pacePercent: number;
	label: string;
	reset: string;
};

type OmFooterState = {
	enabled: boolean;
	nextValue?: number;
	nextMax?: number;
	poolValue?: number;
	poolMax?: number;
	isObserving?: boolean;
	isConsolidating?: boolean;
};

function splitLine(left: string, right: string, width: number): string {
	return left + " ".repeat(Math.max(1, width - visibleWidth(left) - visibleWidth(right))) + right;
}

function formatTokens(tokens: number): string {
	if (tokens >= 1_000_000) return `${(tokens / 1_000_000).toFixed(1)}M`;
	return tokens >= 1_000 ? `${(tokens / 1_000).toFixed(1)}k` : `${tokens}`;
}

export function omLabel(): string {
	const state = (globalThis as { __owaisOmFooterState?: OmFooterState }).__owaisOmFooterState;
	if (!state?.enabled) return "Observational Memory OFF";
	const next = `${formatTokens(state.nextValue ?? 0)}/${formatTokens(state.nextMax ?? 0)} O`;
	const pool = `${formatTokens(state.poolValue ?? 0)}/${formatTokens(state.poolMax ?? 0)} C`;
	return `${next} -> ${pool}`;
}

type WorldClockState = {
	render: (availableWidth: number) => string;
};

function worldClockLabel(availableWidth: number): string {
	const state = (globalThis as { __owaisWorldClockState?: WorldClockState }).__owaisWorldClockState;
	return state?.render(availableWidth) ?? "";
}

function quotaLabel(): { left: string; isOverPace: boolean } | undefined {
	const quota = (globalThis as { __owaisQuotaState?: QuotaState }).__owaisQuotaState;
	if (!quota) return undefined;
	const provider = quota.provider === "openai-codex" ? "OpenAI" : "Claude";
	return {
		left: `${quota.usedPercent}%/${quota.pacePercent}% ${quota.label} · ${provider} quota · resets ${quota.reset}`,
		isOverPace: quota.usedPercent > quota.pacePercent,
	};
}

function activityColor(elapsedSeconds: number): string {
	if (elapsedSeconds <= 60) return "#a6dca8";
	if (elapsedSeconds <= 300) return blend("#a6dca8", "#f4cf88", (elapsedSeconds - 60) / 240);
	if (elapsedSeconds <= 600) return blend("#f4cf88", "#f28da5", (elapsedSeconds - 300) / 300);
	return "#f28da5";
}

function elapsedLabel(startedAt: number): string {
	const tenths = Math.floor(Math.max(0, Date.now() - startedAt) / 100);
	const minutes = Math.floor(tenths / 600);
	const seconds = Math.floor(tenths / 10) % 60;
	return `${minutes.toString().padStart(2, "0")}:${seconds.toString().padStart(2, "0")}.${tenths % 10}`;
}

export function brailleOrbit(elapsedMilliseconds: number): string {
	return BRAILLE_ORBIT[Math.floor(elapsedMilliseconds / 80) % BRAILLE_ORBIT.length]!;
}

const PULSE_STEP_MILLISECONDS = 150;

// index of the character to bold this tick, cycling once through the whole string per pass.
export function pulsingCharacterIndex(elapsedMilliseconds: number, length: number): number {
	if (length <= 0) return 0;
	return Math.floor(elapsedMilliseconds / PULSE_STEP_MILLISECONDS) % length;
}

// bolds one character of `text` per tick, cycling through the string — the loading cue for a
// headline that already has an incumbent value shown while a challenger regenerates in the background.
export function pulsingHeadlineText(text: string, elapsedMilliseconds: number): string {
	if (text.length === 0) return text;
	const index = pulsingCharacterIndex(elapsedMilliseconds, text.length);
	return `${text.slice(0, index)}\x1b[1m${text[index]}\x1b[22m${text.slice(index + 1)}`;
}

function fgHex(hex: string, text: string): string {
	const [r, g, b] = [1, 3, 5].map((offset) => Number.parseInt(hex.slice(offset, offset + 2), 16));
	return `\x1b[38;2;${r};${g};${b}m${text}\x1b[39m`;
}

function contextColor(percent: number): string {
	if (percent <= 40) return "#7f8caa";
	if (percent <= 50) return blend("#7f8caa", "#f1c97a", (percent - 40) / 10);
	if (percent <= 70) return "#f1c97a";
	if (percent <= 80) return blend("#f1c97a", "#f28b9a", (percent - 70) / 10);
	return "#f28b9a";
}

export const BRANCH_SUMMARY_RELEVANCE_THRESHOLD = 0.5;

// symmetric ratio: growth or shrinkage both erode it; 1 = commit count unchanged, 0 = no incumbent.
export function computeBranchSummaryRelevance(incumbentCommitCount: number | undefined, currentCommitCount: number): number {
	if (incumbentCommitCount === undefined || incumbentCommitCount === 0 || currentCommitCount === 0) return 0;
	return Math.min(incumbentCommitCount, currentCommitCount) / Math.max(incumbentCommitCount, currentCommitCount);
}

export function isBranchSummaryChallengerBetter(incumbent: string | undefined, challenger: string): boolean {
	if (challenger.trim() === "") return false;
	if (incumbent === undefined) return true;
	return challenger.trim().toLowerCase() !== incumbent.trim().toLowerCase();
}

// file names / diffstat only, never full diff bodies — keeps the prompt small for the 15s-timeout on-device call.
export function parseWorkingTreeFiles(porcelainOutput: string): string[] {
	return porcelainOutput
		.split("\n")
		.map((line) => line.trim())
		.filter(Boolean)
		.map((line) => line.slice(2).trim())
		.filter(Boolean);
}

// the pi transcript is the only signal for read-only sessions (research, review, Q&A) — commits and
// working-tree files stay empty the whole time, so without this the headline never has anything to draw on.
export function extractTranscriptAsks(entries: SessionEntry[], maxCount = BRANCH_SUMMARY_MAX_TRANSCRIPT_ASKS): string[] {
	const asks: string[] = [];
	for (const entry of entries) {
		if (entry.type !== "message" || entry.message.role !== "user") continue;
		const content = entry.message.content;
		const text = typeof content === "string"
			? content
			: content.filter((part) => part.type === "text").map((part) => part.text).join(" ");
		const trimmed = text.replace(/\s+/g, " ").trim();
		if (trimmed) asks.push(truncateSegmentText(trimmed, BRANCH_SUMMARY_MAX_TRANSCRIPT_ASK_WIDTH));
	}
	return asks.slice(-maxCount);
}

export function buildBranchSummaryPrompt(subjects: string[], workingTreeFiles: string[] = [], transcriptAsks: string[] = []): string {
	const blocks: string[] = [];
	if (subjects.length > 0) blocks.push(`Commits so far:\n${subjects.join("\n")}`);
	if (workingTreeFiles.length > 0) blocks.push(`Files with uncommitted changes right now:\n${workingTreeFiles.join("\n")}`);
	if (transcriptAsks.length > 0) blocks.push(`What the user asked for in this conversation:\n${transcriptAsks.join("\n")}`);
	return `Name what this branch's work is about, as one short label, not a sentence:\n${blocks.join("\n\n")}`;
}

export function truncateSegmentText(text: string, maxWidth: number, fromStart = false): string {
	const flattened = text.replace(/\s+/g, " ").trim();
	if (visibleWidth(flattened) <= maxWidth) return flattened;
	const ellipsis = "…";
	let truncated = flattened;
	if (fromStart) {
		while (truncated.length > 0 && visibleWidth(truncated) + visibleWidth(ellipsis) > maxWidth) {
			truncated = truncated.slice(1);
		}
		return `${ellipsis}${truncated.trimStart()}`;
	}
	while (truncated.length > 0 && visibleWidth(truncated) + visibleWidth(ellipsis) > maxWidth) {
		truncated = truncated.slice(0, -1);
	}
	return `${truncated.trimEnd()}${ellipsis}`;
}

async function isFoundationModelsAvailable(exec: ExtensionAPI["exec"]): Promise<boolean> {
	try {
		const result = await exec("fm", ["available", "--model", "system"], { timeout: 5_000 });
		return result.code === 0 && result.stdout.trim() === "System model available";
	} catch {
		return false;
	}
}

async function runFoundationModelsRespond(exec: ExtensionAPI["exec"], prompt: string): Promise<string | undefined> {
	try {
		const result = await exec(
			"fm",
			["respond", "--model", "system", "--no-stream", "--instructions", BRANCH_SUMMARY_INSTRUCTIONS, prompt],
			{ timeout: 15_000 },
		);
		if (result.code !== 0 || result.stdout.trim() === "") return undefined;
		return result.stdout.trim();
	} catch {
		return undefined;
	}
}

export default function owaisFooter(pi: ExtensionAPI): void {
	let activeContext: ExtensionContext | undefined;
	let repository: RepositoryState = { isGit: false, path: "unknown" };
	let pullRequest: PullRequest | undefined;
	let isWorking = false;
	let startedAt = 0;
	const activeSubagentIds = new Set<string>();
	let requestRender: (() => void) | undefined;
	let refreshGeneration = 0;
	let pullRequestTimer: ReturnType<typeof setInterval> | undefined;
	let branchSummary: string | undefined;
	let branchSummaryCommitCount: number | undefined;
	let branchSummaryTranscriptAskCount: number | undefined;
	let branchSummaryWorkingTreeFingerprint: string | undefined;
	let isBranchSummaryGenerating = false;
	let isFoundationModelsAvailableCache: boolean | undefined;
	const execForBranchPoint: Exec = (command, args, options) => pi.exec(command, args, { cwd: options?.cwd, timeout: 5_000 });

	async function hasGitHubRemote(cwd: string): Promise<boolean> {
		const result = await pi.exec("git", ["remote", "-v"], { cwd, timeout: 5_000 });
		return result.code === 0 && /github\.com[:/]/i.test(result.stdout);
	}

	// checked on every agent_settled/branch change. the incumbent stays visible for the whole call —
	// only swapped if the challenger is non-empty and different; the commit-count anchor advances either way.
	async function refreshBranchSummary(cwd: string, generation: number): Promise<void> {
		try {
			const branchPoint = await resolveBranchPointCommit(execForBranchPoint, cwd);
			if (generation !== refreshGeneration || branchPoint === null) return;

			const logResult = await pi.exec(
				"git",
				["log", "--oneline", `-${BRANCH_SUMMARY_MAX_COMMITS}`, `${branchPoint}..HEAD`],
				{ cwd, timeout: 5_000 },
			);
			if (generation !== refreshGeneration || logResult.code !== 0) return;
			const subjects = logResult.stdout.split("\n").map((line) => line.trim()).filter(Boolean);

			// mid-session uncommitted work never shows up in commit count, so a second signal watches the
			// working tree directly — any change to the changed-file list counts as "the branch moved" too.
			const statusResult = await pi.exec("git", ["status", "--porcelain"], { cwd, timeout: 5_000 });
			if (generation !== refreshGeneration || statusResult.code !== 0) return;
			const workingTreeFiles = parseWorkingTreeFiles(statusResult.stdout).slice(0, BRANCH_SUMMARY_MAX_WORKING_TREE_FILES);
			const workingTreeFingerprint = workingTreeFiles.join("\n");

			// a read-only session has no commits or working-tree diff — the transcript is the only signal.
			const transcriptAsks = extractTranscriptAsks(activeContext?.sessionManager?.getEntries?.() ?? []);

			if (subjects.length === 0 && workingTreeFiles.length === 0 && transcriptAsks.length === 0) return;

			// an all-zero signal is excluded from the gate, not read as "always moved". the transcript
			// signal skips the ratio throttle entirely: any new ask regenerates on the next settle.
			const commitRelevance = computeBranchSummaryRelevance(branchSummaryCommitCount, subjects.length);
			const isWorkingTreeMoved = workingTreeFingerprint !== (branchSummaryWorkingTreeFingerprint ?? "");
			const isCommitSignalStale = subjects.length === 0 || commitRelevance >= BRANCH_SUMMARY_RELEVANCE_THRESHOLD;
			const isTranscriptSignalStale = transcriptAsks.length === 0 || transcriptAsks.length === (branchSummaryTranscriptAskCount ?? 0);
			if (isCommitSignalStale && isTranscriptSignalStale && !isWorkingTreeMoved) return;

			if (isFoundationModelsAvailableCache === undefined) {
				isFoundationModelsAvailableCache = await isFoundationModelsAvailable(pi.exec);
			}
			if (generation !== refreshGeneration || !isFoundationModelsAvailableCache) return;

			isBranchSummaryGenerating = true;
			requestRender?.();
			let response: string | undefined;
			try {
				response = await runFoundationModelsRespond(pi.exec, buildBranchSummaryPrompt(subjects, workingTreeFiles, transcriptAsks));
			} finally {
				isBranchSummaryGenerating = false;
			}
			if (generation !== refreshGeneration || response === undefined) return;
			const challenger = truncateSegmentText(response, BRANCH_SUMMARY_MAX_WIDTH);
			if (isBranchSummaryChallengerBetter(branchSummary, challenger)) {
				branchSummary = challenger;
			}
			branchSummaryCommitCount = subjects.length;
			branchSummaryTranscriptAskCount = transcriptAsks.length;
			branchSummaryWorkingTreeFingerprint = workingTreeFingerprint;
		} catch {
			return;
		} finally {
			if (generation === refreshGeneration) requestRender?.();
		}
	}

	async function refreshPullRequest(cwd: string, generation: number): Promise<void> {
		try {
			const hasRemote = await hasGitHubRemote(cwd);
			if (generation !== refreshGeneration) return;
			if (!hasRemote) {
				pullRequest = undefined;
				return;
			}
			const result = await pi.exec(
				"gh",
				["pr", "view", "--json", "url,number,state,isDraft,mergeStateStatus"],
				{ cwd, timeout: 5_000 },
			);
			if (generation !== refreshGeneration) return;
			// invariant: the footer only ever shows a pull request the newest concluded lookup found, so a
			// concluded lookup owns the segment outright and a superseded generation never writes to it.
			const candidate = result.code === 0 ? JSON.parse(result.stdout) as unknown : undefined;
			pullRequest = isPullRequest(candidate) ? candidate : undefined;
		} catch {
			return;
		} finally {
			if (generation === refreshGeneration) requestRender?.();
		}
	}

	async function refreshRepository(cwd: string | undefined): Promise<void> {
		const generation = ++refreshGeneration;
		if (!cwd) {
			pullRequest = undefined;
			repository = { isGit: false, path: "unknown" };
			requestRender?.();
			return;
		}
		try {
			const root = await pi.exec("git", ["rev-parse", "--show-toplevel"], { cwd, timeout: 5_000 });
			if (generation !== refreshGeneration) return;
			if (root.code !== 0 || root.stdout.trim() === "") {
				pullRequest = undefined;
				repository = { isGit: false, path: compactPath(cwd) };
				requestRender?.();
				return;
			}
			const branchResult = await pi.exec("git", ["branch", "--show-current"], { cwd, timeout: 5_000 });
			if (generation !== refreshGeneration) return;
			const branch = branchResult.code === 0 ? branchResult.stdout.trim() || undefined : undefined;
			const isBranchChanged = !repository.isGit || repository.branch !== branch;
			repository = { isGit: true, project: projectName(cwd, root.stdout.trim()), branch };
			if (isBranchChanged || !branch) {
				pullRequest = undefined;
				branchSummary = undefined;
				branchSummaryCommitCount = undefined;
				branchSummaryTranscriptAskCount = undefined;
				branchSummaryWorkingTreeFingerprint = undefined;
			}
			requestRender?.();
			if (branch) {
				await refreshPullRequest(cwd, generation);
				// steady state runs off agent_settled; this only fires on first population or branch switch.
				if (isBranchChanged) void refreshBranchSummary(cwd, generation);
			}
		} catch {
			if (generation !== refreshGeneration) return;
			repository = { isGit: false, path: compactPath(cwd) };
			requestRender?.();
		}
	}

	function setWorking(isActive: boolean): void {
		isWorking = isActive;
		if (isActive) startedAt = Date.now();
		requestRender?.();
	}

	function setSubagentActive(payload: unknown, isActive: boolean): void {
		if (payload === null || typeof payload !== "object") return;
		const { id } = payload as SubagentLifecycleEvent;
		if (typeof id !== "string" || id.trim().length === 0) return;
		const sizeBefore = activeSubagentIds.size;
		if (isActive) activeSubagentIds.add(id);
		else activeSubagentIds.delete(id);
		if (activeSubagentIds.size !== sizeBefore) requestRender?.();
	}

	pi.on("session_start", (_event, ctx) => {
		isWorking = false;
		activeSubagentIds.clear();
		if (ctx.mode !== "tui") return;
		activeContext = ctx;
		ctx.ui.setWorkingVisible?.(false);
		void refreshRepository(ctx.cwd);
		if (pullRequestTimer) clearInterval(pullRequestTimer);
		pullRequestTimer = setInterval(() => void refreshRepository(ctx.cwd), PR_POLL_INTERVAL_MS);
		ctx.ui.setFooter((tui, theme, footerData) => {
			const timer = setInterval(() => tui.requestRender(), 120);
			const unsubscribeBranch = footerData.onBranchChange(() => void refreshRepository(ctx.cwd));
			return {
				dispose: () => {
					clearInterval(timer);
					unsubscribeBranch();
				},
				invalidate() {},
				render(width) {
					const usage = ctx.getContextUsage();
					const context = usage?.tokens != null && ctx.model?.contextWindow
						? theme.fg("muted", "Context ") +
							fgHex(contextColor((usage.tokens / ctx.model.contextWindow) * 100), `${Math.round((usage.tokens / ctx.model.contextWindow) * 100)}%`) +
							theme.fg("muted", `/${formatTokens(ctx.model.contextWindow)}`)
						: theme.fg("muted", "context unknown");
					const quota = quotaLabel();
					const lines = [splitLine(context, theme.fg("muted", omLabel()), width)];
					let quotaLeft = "";
					if (quota) {
						const usageText = quota.left.match(/^\d+%/)?.[0] ?? "";
						quotaLeft = quota.isOverPace
							? fgHex("#f28b9a", usageText) + theme.fg("muted", quota.left.slice(usageText.length))
							: theme.fg("muted", quota.left);
					}
					const availableRightWidth = Math.max(0, width - visibleWidth(quotaLeft) - 1);
					const worldClock = worldClockLabel(availableRightWidth);
					if (quotaLeft || worldClock) {
						lines.push(splitLine(quotaLeft, worldClock, width));
					}
					return lines;
				},
			};
		});
		ctx.ui.setWidget(
			"owais-pre-input",
			(tui, theme) => {
				requestRender = () => tui.requestRender();
				const timer = setInterval(() => tui.requestRender(), 120);
				return {
					dispose: () => {
						clearInterval(timer);
						requestRender = undefined;
					},
					invalidate() {},
					render(width) {
						const safeWidth = Math.max(0, width);
						const elapsedMilliseconds = Math.max(0, Date.now() - startedAt);
						const elapsedSeconds = elapsedMilliseconds / 1000;
						let activityKind: ActivityKind = "ready";
						let activity = "Ready";
						if (activeSubagentIds.size > 0) {
							activityKind = "delegated";
							activity = "Delegated";
						}
						if (isWorking) {
							activityKind = "working";
							activity = elapsedLabel(startedAt);
						}
						const activityLabel = activityKind === "working" ? `${brailleOrbit(elapsedMilliseconds)} ${activity}` : activity;
						const state = repository;
						const ctx = activeContext ?? ({} as ExtensionContext);
						const provider = ctx.model?.provider ?? "unknown";
						const model = ctx.model?.id ?? "unknown";
						const thinking = ctx.thinkingLevel ?? "off";

						const items: FooterDegradableSegment[] = [];

						if (state.isGit) {
							const branch = state.branch;
							if (branch) {
								const colorize = (text: string) => fgHex(branchColor(branch), text);
								items.push({
									id: "branch",
									full: textForm(branch, colorize),
									shorten: textForm(truncateSegmentText(branch, 24, true), colorize),
									truncate: textForm(truncateSegmentText(branch, 12, true), colorize),
								});
							} else {
								items.push({ id: "branch", full: textForm("detached", (text) => theme.fg("muted", text)) });
							}
						}

						if (branchSummary) {
							const summaryText = branchSummary;
							const isPulsing = isBranchSummaryGenerating;
							const colorize = (text: string) =>
								theme.fg("muted", isPulsing ? pulsingHeadlineText(text, Date.now()) : text);
							items.push({
								id: "headline",
								full: textForm(summaryText, colorize),
								shorten: textForm(truncateSegmentText(summaryText, 24), colorize),
								truncate: textForm(truncateSegmentText(summaryText, 14), colorize),
							});
						} else if (isBranchSummaryGenerating) {
							const colorize = (text: string) => theme.fg("muted", text);
							items.push({
								id: "headline",
								full: textForm(`${brailleOrbit(Date.now())} Generating headline\u2026`, colorize),
							});
						} else {
							// idle with no summary yet (new session, or fm unavailable) — show a placeholder, not a blank slot.
							const colorize = (text: string) => theme.fg("muted", text);
							items.push({ id: "headline", full: textForm(NEW_SESSION_HEADLINE, colorize) });
						}

						const workspaceText = state.isGit ? state.project : state.path;
						const workspaceColorize = (text: string) => theme.fg("muted", text);
						items.push({
							id: "workspace",
							full: textForm(workspaceText, workspaceColorize),
							shorten: textForm(truncateSegmentText(workspaceText, 12), workspaceColorize),
							truncate: textForm(truncateSegmentText(workspaceText, 6), workspaceColorize),
						});

						const activePullRequest = pullRequest;
						if (activePullRequest) {
							const tone = pullRequestTone(activePullRequest);
							const { number, url } = activePullRequest;
							const colorForTone = (label: string) => tone === "purple" ? fgHex("#c7b5ff", label) : theme.fg(tone, label);
							const colorize = (label: string) => colorForTone(osc8(url, label));
							items.push({
								id: "pr",
								full: textForm(`PR #${number}`, colorize),
								shorten: textForm(`#${number}`, colorize),
							});
						}

						const statusColorize = (text: string) => {
							if (activityKind === "working") return fgHex(activityColor(elapsedSeconds), text);
							if (activityKind === "delegated") return theme.fg("accent", text);
							return theme.fg("success", text);
						};
						items.push({
							id: "status",
							full: textForm(activityLabel, statusColorize),
							shorten: activityKind === "working" ? textForm(activity, statusColorize) : undefined,
						});

						const identity = (text: string) => text;
						items.push({ id: "provider", full: textForm(provider, identity) });
						items.push({ id: "thinking", full: textForm(thinking, identity) });
						items.push({
							id: "model",
							full: textForm(model, identity),
							truncate: textForm(truncateSegmentText(model, 12, true), identity),
						});

						const forms = resolveFooterSegments(items, safeWidth);
						const { left, right } = assembleFooterLine(
							forms,
							(form) => form.render(),
							theme.fg("dim", " · "),
							theme.fg("dim", " > "),
						);
						const spacerWidth = Math.max(0, safeWidth - visibleWidth(left) - visibleWidth(right));
						return [
							theme.fg("border", "─".repeat(safeWidth)),
							left + " ".repeat(spacerWidth) + theme.fg("muted", right),
						];
					},
				};
			},
			{ placement: "aboveEditor" },
		);
	});

	pi.on("agent_start", () => setWorking(true));
	pi.on("agent_settled", () => {
		setWorking(false);
		if (repository.isGit && repository.branch && activeContext?.cwd) {
			void refreshBranchSummary(activeContext.cwd, refreshGeneration);
		}
	});
	pi.events.on("subagents:started", (payload) => setSubagentActive(payload, true));
	pi.events.on("subagents:completed", (payload) => setSubagentActive(payload, false));
	pi.events.on("subagents:failed", (payload) => setSubagentActive(payload, false));
	pi.on("model_select", (_event, ctx) => {
		activeContext = ctx;
		requestRender?.();
	});
	pi.on("session_shutdown", () => {
		if (pullRequestTimer) clearInterval(pullRequestTimer);
		pullRequestTimer = undefined;
		isWorking = false;
		activeSubagentIds.clear();
		requestRender?.();
		activeContext = undefined;
		requestRender = undefined;
	});
}
