import { execFile } from "node:child_process";

import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent";

import {
	branchStats,
	captureSnapshot,
	diffStats,
	filePatch,
	resolveBranchPointTree,
	type Exec,
} from "./live-diff/engine.ts";
import { openInNvim } from "./live-diff/nvim.ts";
import {
	applyPatch,
	badgeText,
	initialModel,
	rebuildRows,
	reduce,
	renderRows,
} from "./live-diff/overlay.ts";
import type {
	Hunk,
	LiveDiffState,
	OverlayEffect,
	OverlayKey,
	RenderRow,
	RowTone,
	WatcherFactory,
	WorktreeWatcher,
} from "./live-diff/types.ts";
import { createWatcher, isRefreshWorthy } from "./live-diff/watch.ts";

const MAX_FILES = 400;
const OVERLAY_WIDTH = "80%";
const OVERLAY_MIN_WIDTH = 48;
const OVERLAY_PADDING_X = 1;
const OVERLAY_PADDING_Y = 1;
// pi-tui's Component#render(width) receives no live terminal height, and
// TUI exposes no getter for one either — overlayOptions.maxHeight is a
// REQUEST to the framework, not a queryable actual. So the visible row
// budget is a shell-owned constant, matching OVERLAY_WIDTH/OVERLAY_MIN_WIDTH's
// own pattern, and the same figure is what we ask the framework for via
// maxHeight. Kept modest so it holds on a small terminal, not just a tall one.
const OVERLAY_MAX_HEIGHT = 24;
const SELECTED_GUTTER = "▌";
const UNSELECTED_GUTTER = " ";
const DEBOUNCE_MS = 300;
const WATCH_COALESCE_MS = 300;
const WATCH_BATCH_LIMIT = 500;
const WRITE_TOOLS = new Set(["edit", "write", "bash"]);

let watcherFactory: WatcherFactory = createWatcher;

/**
 * Replace the watcher factory the extension starts on session_start.
 *
 * @param factory factory to use, or createWatcher to restore the default
 */
export function setWatcherFactory(factory: WatcherFactory): void {
	watcherFactory = factory;
}

function runCheckIgnore(cwd: string, paths: string[]): Promise<string> {
	return new Promise((resolvePromise) => {
		const child = execFile(
			"git",
			["check-ignore", "--stdin", "-z", "-n", "-v"],
			{ cwd, maxBuffer: 64 * 1024 * 1024 },
			(error, stdout) => {
				const rawCode = (error as NodeJS.ErrnoException | null)?.code;
				const code = error ? (typeof rawCode === "number" ? rawCode : 1) : 0;
				resolvePromise(code === 0 || code === 1 ? String(stdout) : "");
			},
		);
		child.stdin?.on("error", () => {});
		child.stdin?.end(`${paths.join("\0")}\0`);
	});
}

async function selectIgnoredPaths(
	cwd: string,
	paths: string[],
): Promise<Set<string>> {
	const ignored = new Set<string>();
	if (paths.length === 0) {
		return ignored;
	}
	const stdout = await runCheckIgnore(cwd, paths);
	const fields = stdout.split("\0");
	for (let index = 0; index + 3 < fields.length; index += 4) {
		if (fields[index] !== "") {
			ignored.add(fields[index + 3]);
		}
	}
	return ignored;
}

const exec: Exec = (command, args, options) =>
	new Promise((resolvePromise) => {
		execFile(
			command,
			args,
			{
				cwd: options?.cwd,
				env: options?.env ? { ...process.env, ...options.env } : process.env,
				maxBuffer: 64 * 1024 * 1024,
			},
			(error, stdout, stderr) => {
				const rawCode = (error as NodeJS.ErrnoException | null)?.code;
				const code = error ? (typeof rawCode === "number" ? rawCode : 1) : 0;
				resolvePromise({ code, stdout: String(stdout), stderr: String(stderr) });
			},
		);
	});

type ThemeLike = {
	fg(color: string, text: string): string;
	bg(background: string, text: string): string;
};

const TONE_COLORS: Record<RowTone, string> = {
	header: "borderAccent",
	path: "text",
	added: "toolDiffAdded",
	removed: "toolDiffRemoved",
	binary: "dim",
	hunkHeader: "accent",
	hunkAdd: "toolDiffAdded",
	hunkRemove: "toolDiffRemoved",
	hunkContext: "toolDiffContext",
	hint: "muted",
	truncation: "dim",
	originCommitted: "dim",
	originUncommitted: "accent",
};

function paintBackground(
	theme: ThemeLike,
	background: string,
	line: string,
): string {
	try {
		return theme.bg(background, line);
	} catch {
		return line;
	}
}

function styleRow(theme: ThemeLike, row: RenderRow, padding: string): string {
	const gutter = row.isSelected ? SELECTED_GUTTER : UNSELECTED_GUTTER;
	let body = "";
	for (const span of row.spans) {
		try {
			body += theme.fg(TONE_COLORS[span.tone], span.text);
		} catch {
			body += span.text;
		}
	}
	const line = padding + gutter + body + padding;
	return paintBackground(
		theme,
		row.isSelected ? "selectedBg" : "customMessageBg",
		line,
	);
}

/**
 * Map one raw input byte sequence to an overlay key.
 *
 * @param data raw bytes from the terminal
 * @returns the mapped key, or null when the input is unbound
 */
export function mapKey(data: string): OverlayKey | null {
	switch (data) {
		case "\x1b[A":
		case "k":
			return "up";
		case "\x1b[B":
		case "j":
			return "down";
		case "h":
			return "mode-left";
		case "l":
			return "mode-right";
		case "\r":
		case "\n":
			return "open";
		case " ":
		case "f":
			return "fold";
		case "q":
		case "\x1b":
			return "close";
		default:
			return null;
	}
}

/**
 * Live request and overall diffs: snapshot at request start, badge refresh
 * after write-capable tools and on settle, /diff overlay with in-place hunk
 * folding and open-in-nvim.
 *
 * @param pi extension API
 */
export default function liveDiff(pi: ExtensionAPI): void {
	const state: LiveDiffState = {
		requestSnapshot: null,
		overallBaselineSha: null,
		requestStats: null,
		overallStats: null,
		refreshTimer: null,
		isRefreshing: false,
		watcher: null,
		watchTimer: null,
	};
	let isDirtyPending = false;
	let lastRefreshAt = 0;
	let isSessionGone = false;
	let pendingWatchPaths = new Set<string>();
	let branchPointTree: string | null = null;
	let isBranchPointResolved = false;

	function setBadge(ctx: ExtensionContext, text: string): void {
		if (isSessionGone) {
			return;
		}
		try {
			ctx.ui.setStatus("live-diff", text);
		} catch {}
	}

	async function ensureBaseline(ctx: ExtensionContext): Promise<void> {
		if (!isBranchPointResolved) {
			branchPointTree = await resolveBranchPointTree(exec, ctx.cwd);
			isBranchPointResolved = true;
		}
		if (branchPointTree !== null) {
			return;
		}
		if (state.overallBaselineSha !== null) {
			return;
		}
		const snapshot = await captureSnapshot(exec, ctx.cwd);
		state.overallBaselineSha = snapshot.baselineSha;
	}

	async function refresh(ctx: ExtensionContext): Promise<void> {
		if (state.isRefreshing) {
			isDirtyPending = true;
			return;
		}
		state.isRefreshing = true;
		try {
			await ensureBaseline(ctx);
			state.requestStats = state.requestSnapshot
				? await diffStats(exec, ctx.cwd, state.requestSnapshot.treeSha, MAX_FILES)
				: null;
			state.overallStats =
				branchPointTree !== null
					? await branchStats(exec, ctx.cwd, branchPointTree, MAX_FILES)
					: state.overallBaselineSha
						? await diffStats(exec, ctx.cwd, state.overallBaselineSha, MAX_FILES)
						: null;
			setBadge(
				ctx,
				badgeText(
					state.requestStats,
					state.overallStats,
					branchPointTree !== null ? "branch" : "all",
				),
			);
		} catch {
			setBadge(ctx, "diff ?");
		} finally {
			state.isRefreshing = false;
		}
	}

	function consumeDirty(ctx: ExtensionContext): void {
		if (isDirtyPending) {
			isDirtyPending = false;
			void refresh(ctx);
		}
	}

	function startWatcher(ctx: ExtensionContext): void {
		if (state.watcher !== null) {
			return;
		}
		const root = ctx.cwd;
		let watcher: WorktreeWatcher | null = null;
		try {
			watcher = watcherFactory(root, (relativePath) => {
				onWatchedChange(ctx, root, relativePath);
			});
		} catch {
			watcher = null;
		}
		state.watcher = watcher;
	}

	function onWatchedChange(
		ctx: ExtensionContext,
		root: string,
		relativePath: string,
	): void {
		if (isSessionGone) {
			return;
		}
		if (!isRefreshWorthy(relativePath, () => false)) {
			return;
		}
		if (pendingWatchPaths.size < WATCH_BATCH_LIMIT) {
			pendingWatchPaths.add(relativePath);
		}
		if (state.watchTimer !== null) {
			return;
		}
		state.watchTimer = setTimeout(() => {
			state.watchTimer = null;
			const batch = [...pendingWatchPaths];
			pendingWatchPaths = new Set();
			if (isSessionGone || batch.length === 0) {
				return;
			}
			void flushWatchBatch(ctx, root, batch);
		}, WATCH_COALESCE_MS);
	}

	async function flushWatchBatch(
		ctx: ExtensionContext,
		root: string,
		batch: string[],
	): Promise<void> {
		let ignored: Set<string>;
		try {
			ignored = await selectIgnoredPaths(root, batch);
		} catch {
			ignored = new Set();
		}
		if (isSessionGone) {
			return;
		}
		const hasWorthyChange = batch.some((candidate) =>
			isRefreshWorthy(candidate, (path) => ignored.has(path)),
		);
		if (!hasWorthyChange) {
			return;
		}
		await refresh(ctx);
	}

	pi.on("session_start", (_event, ctx) => {
		isSessionGone = false;
		startWatcher(ctx);
		void refresh(ctx);
	});

	pi.on("agent_start", async (_event, ctx) => {
		consumeDirty(ctx);
		try {
			await ensureBaseline(ctx);
			state.requestSnapshot = await captureSnapshot(exec, ctx.cwd);
		} catch {
			setBadge(ctx, "diff ?");
		}
	});

	pi.on("tool_execution_end", (event, ctx) => {
		if (isDirtyPending) {
			isDirtyPending = false;
			void refresh(ctx);
			return;
		}
		if (!WRITE_TOOLS.has(event.toolName)) {
			return;
		}
		const now = Date.now();
		if (now - lastRefreshAt >= DEBOUNCE_MS) {
			lastRefreshAt = now;
			void refresh(ctx);
			return;
		}
		if (state.refreshTimer === null) {
			state.refreshTimer = setTimeout(() => {
				state.refreshTimer = null;
				isDirtyPending = true;
			}, DEBOUNCE_MS);
		}
	});

	pi.on("agent_settled", (_event, ctx) => {
		isDirtyPending = false;
		void refresh(ctx);
	});

	pi.on("session_shutdown", () => {
		isSessionGone = true;
		if (state.refreshTimer !== null) {
			clearTimeout(state.refreshTimer);
			state.refreshTimer = null;
		}
		if (state.watchTimer !== null) {
			clearTimeout(state.watchTimer);
			state.watchTimer = null;
		}
		if (state.watcher !== null) {
			try {
				state.watcher.close();
			} catch {}
			state.watcher = null;
		}
		pendingWatchPaths = new Set();
		isDirtyPending = false;
		state.requestSnapshot = null;
		state.overallBaselineSha = null;
		state.requestStats = null;
		state.overallStats = null;
		state.isRefreshing = false;
		branchPointTree = null;
		isBranchPointResolved = false;
	});

	pi.registerCommand("diff", {
		description: "Show live request/overall diff overlay",
		handler: async (_args, ctx) => {
			if (!ctx.hasUI || ctx.mode !== "tui") {
				return;
			}
			await refresh(ctx);
			if (isSessionGone) {
				return;
			}
			let model = initialModel(state.requestStats, state.overallStats);
			await ctx.ui.custom<undefined>(
				(tui, theme, _keybindings, done) => {
					async function runEffect(effect: OverlayEffect): Promise<void> {
						if (effect.kind === "close") {
							done(undefined);
							return;
						}
						if (effect.kind === "load-patch") {
							const baseTreeSha =
								effect.mode === "request"
									? state.requestSnapshot?.treeSha ?? null
									: branchPointTree ?? state.overallBaselineSha;
							if (baseTreeSha === null) {
								return;
							}
							let hunks: Hunk[];
							try {
								hunks = await filePatch(exec, ctx.cwd, baseTreeSha, effect.path);
							} catch {
								hunks = [{ header: "patch unavailable", lines: [] }];
							}
							model = applyPatch(model, effect.mode, effect.path, hunks);
							tui.requestRender();
							return;
						}
						const isOpened = await openInNvim(exec, ctx.cwd, effect.path);
						if (isOpened) {
							done(undefined);
						}
					}
					return {
						render(width: number): string[] {
							const stats =
								model.mode === "request" ? state.requestStats : state.overallStats;
							const contentWidth = Math.max(
								1,
								width - OVERLAY_PADDING_X * 2 - SELECTED_GUTTER.length,
							);
							// The frame adds OVERLAY_PADDING_Y blank lines above and below
							// the body, so the body's own budget is the requested maximum
							// MINUS that padding. Getting this wrong by the padding amount
							// is exactly how the earlier edge-bleed bug happened.
							const visibleHeight = Math.max(
								1,
								OVERLAY_MAX_HEIGHT - OVERLAY_PADDING_Y * 2,
							);
							const rows = renderRows(
								model,
								contentWidth,
								stats?.isTruncated ?? false,
								visibleHeight,
							);
							const pad = " ".repeat(OVERLAY_PADDING_X);
							const body = rows.map((row) => styleRow(theme, row, pad));
							const blank = paintBackground(
								theme,
								"customMessageBg",
								" ".repeat(width),
							);
							const frame: string[] = [];
							for (let index = 0; index < OVERLAY_PADDING_Y; index += 1) {
								frame.push(blank);
							}
							frame.push(...body);
							for (let index = 0; index < OVERLAY_PADDING_Y; index += 1) {
								frame.push(blank);
							}
							return frame;
						},
						handleInput(data: string): void {
							const key = mapKey(data);
							if (key === null) {
								return;
							}
							const step = reduce(model, key);
							model = step.model;
							if (key === "mode-left" || key === "mode-right") {
								model = rebuildRows(model, state.requestStats, state.overallStats);
							}
							if (step.effect !== null) {
								void runEffect(step.effect);
							}
							tui.requestRender();
						},
					};
				},
				{
					overlay: true,
					overlayOptions: {
						width: OVERLAY_WIDTH,
						minWidth: OVERLAY_MIN_WIDTH,
						maxHeight: OVERLAY_MAX_HEIGHT,
					},
				},
			);
		},
	});
}
