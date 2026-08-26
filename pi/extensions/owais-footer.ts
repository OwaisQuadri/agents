import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent";
import { visibleWidth } from "@earendil-works/pi-tui";
import { homedir } from "node:os";
import { basename, relative } from "node:path";

const HERDR_WORKTREE_MARKER = "/.herdr/worktrees/";
const PR_POLL_INTERVAL_MS = 5 * 60 * 1000;
const BRAILLE_ORBIT = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

type RepositoryState =
	| { isGit: true; project: string; branch: string | undefined }
	| { isGit: false; path: string };

export type PullRequest = {
	url: string;
	number: number;
	state: string;
	isDraft: boolean;
	mergeStateStatus: string;
};

type PreInputSegment = {
	text: string;
	render: () => string;
};

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

const BRANCH_COLORS = ["#82b8ff", "#8ee7f5", "#a6dca8", "#c7b5ff", "#d394ff"];

function branchColor(name: string): string {
	return BRANCH_COLORS[name.length % BRANCH_COLORS.length]!;
}

function modelLabels(ctx: ExtensionContext): string[] {
	const provider = ctx.model?.provider ?? "unknown";
	const model = ctx.model?.id ?? "unknown";
	const thinking = ctx.thinkingLevel ?? "off";
	return [`${provider}/${model} (${thinking})`, `${model} (${thinking})`, model];
}

function fittingModelLabel(labels: string[], availableWidth: number): string {
	return labels.find((label) => visibleWidth(label) <= availableWidth) ?? "";
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

export default function owaisFooter(pi: ExtensionAPI): void {
	let activeContext: ExtensionContext | undefined;
	let repository: RepositoryState = { isGit: false, path: "unknown" };
	let pullRequest: PullRequest | undefined;
	let isWorking = false;
	let startedAt = 0;
	let requestRender: (() => void) | undefined;
	let refreshGeneration = 0;
	let pullRequestTimer: ReturnType<typeof setInterval> | undefined;

	async function hasGitHubRemote(cwd: string): Promise<boolean> {
		const result = await pi.exec("git", ["remote", "-v"], { cwd, timeout: 5_000 });
		return result.code === 0 && /github\.com[:/]/i.test(result.stdout);
	}

	async function refreshPullRequest(cwd: string, generation: number): Promise<void> {
		try {
			if (!(await hasGitHubRemote(cwd))) return;
			const result = await pi.exec(
				"gh",
				["pr", "view", "--json", "url,number,state,isDraft,mergeStateStatus"],
				{ cwd, timeout: 5_000 },
			);
			if (generation !== refreshGeneration || result.code !== 0) return;
			const candidate = JSON.parse(result.stdout) as Partial<PullRequest>;
			if (
				typeof candidate.url !== "string" || typeof candidate.number !== "number" ||
				typeof candidate.state !== "string" || typeof candidate.isDraft !== "boolean" ||
				typeof candidate.mergeStateStatus !== "string"
			) {
				return;
			}
			pullRequest = candidate as PullRequest;
		} catch {
			return;
		} finally {
			if (generation === refreshGeneration) requestRender?.();
		}
	}

	async function refreshRepository(cwd: string | undefined): Promise<void> {
		const generation = ++refreshGeneration;
		pullRequest = undefined;
		if (!cwd) {
			repository = { isGit: false, path: "unknown" };
			requestRender?.();
			return;
		}
		try {
			const root = await pi.exec("git", ["rev-parse", "--show-toplevel"], { cwd, timeout: 5_000 });
			if (generation !== refreshGeneration) return;
			if (root.code !== 0 || root.stdout.trim() === "") {
				repository = { isGit: false, path: compactPath(cwd) };
				requestRender?.();
				return;
			}
			const branchResult = await pi.exec("git", ["branch", "--show-current"], { cwd, timeout: 5_000 });
			if (generation !== refreshGeneration) return;
			const branch = branchResult.code === 0 ? branchResult.stdout.trim() || undefined : undefined;
			repository = { isGit: true, project: projectName(cwd, root.stdout.trim()), branch };
			requestRender?.();
			if (branch) await refreshPullRequest(cwd, generation);
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

	pi.on("session_start", (_event, ctx) => {
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
					if (quota) {
						const usageText = quota.left.match(/^\d+%/)?.[0] ?? "";
						const quotaLeft = quota.isOverPace
							? fgHex("#f28b9a", usageText) + theme.fg("muted", quota.left.slice(usageText.length))
							: theme.fg("muted", quota.left);
						const availableRightWidth = Math.max(0, width - visibleWidth(quotaLeft) - 1);
						lines.push(splitLine(quotaLeft, worldClockLabel(availableRightWidth), width));
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
						const activity = isWorking ? elapsedLabel(startedAt) : "Ready";
						const state = repository;
						let location: PreInputSegment;
						if (state.isGit) {
							const { project, branch } = state;
							location = branch
								? {
										text: `${project} > ${branch}`,
										render: () => theme.fg("muted", project) + theme.fg("dim", " > ") + fgHex(branchColor(branch), branch),
									}
								: {
										text: `${project} > detached`,
										render: () => theme.fg("muted", `${project} > detached`),
									};
						} else {
							const { path } = state;
							location = { text: path, render: () => theme.fg("muted", path) };
						}
						const segments: PreInputSegment[] = [location];
						const activePullRequest = pullRequest;
						if (activePullRequest) {
							const tone = pullRequestTone(activePullRequest);
							const { number, url } = activePullRequest;
							segments.push({
								text: `PR #${number}`,
								render: () => {
									const label = osc8(url, `PR #${number}`);
									return tone === "purple" ? fgHex("#c7b5ff", label) : theme.fg(tone, label);
								},
							});
						}
						const activityLabel = isWorking ? `${brailleOrbit(elapsedMilliseconds)} ${activity}` : activity;
						segments.push({
							text: activityLabel,
							render: () => isWorking ? fgHex(activityColor(elapsedSeconds), activityLabel) : theme.fg("success", activityLabel),
						});
						const labels = modelLabels(activeContext ?? ({} as ExtensionContext));
						const left = segments.map((segment) => segment.render()).join(theme.fg("dim", " · "));
						const right = fittingModelLabel(labels, Math.max(0, safeWidth - visibleWidth(left)));
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
	pi.on("agent_settled", () => setWorking(false));
	pi.on("model_select", (_event, ctx) => {
		activeContext = ctx;
		requestRender?.();
	});
	pi.on("session_shutdown", () => {
		if (pullRequestTimer) clearInterval(pullRequestTimer);
		pullRequestTimer = undefined;
		activeContext = undefined;
		requestRender = undefined;
	});
}
