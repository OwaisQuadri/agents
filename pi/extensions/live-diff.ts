import { execFile } from "node:child_process";

import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent";

import { captureSnapshot, diffStats, filePatch, type Exec } from "./live-diff/engine.ts";
import { openInNvim } from "./live-diff/nvim.ts";
import {
	applyPatch,
	badgeText,
	initialModel,
	rebuildRows,
	reduce,
	renderLines,
} from "./live-diff/overlay.ts";
import type {
	Hunk,
	LiveDiffState,
	OverlayEffect,
	OverlayKey,
	WatcherFactory,
	WorktreeWatcher,
} from "./live-diff/types.ts";
import { createWatcher, isRefreshWorthy } from "./live-diff/watch.ts";

const MAX_FILES = 400;
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

function mapKey(data: string): OverlayKey | null {
	switch (data) {
		case "\x1b[A":
		case "k":
			return "up";
		case "\x1b[B":
		case "j":
			return "down";
		case "\t":
		case "t":
			return "toggle-mode";
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

	function setBadge(ctx: ExtensionContext, text: string): void {
		if (isSessionGone) {
			return;
		}
		try {
			ctx.ui.setStatus("live-diff", text);
		} catch {}
	}

	async function ensureBaseline(ctx: ExtensionContext): Promise<void> {
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
			state.overallStats = state.overallBaselineSha
				? await diffStats(exec, ctx.cwd, state.overallBaselineSha, MAX_FILES)
				: null;
			setBadge(ctx, badgeText(state.requestStats, state.overallStats));
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
				// TODO(AGNT-0015.T19): use the theme (currently ignored as _theme) and
				// pass overlayOptions + a Box background so nothing bleeds through.
				(tui, _theme, _keybindings, done) => {
					async function runEffect(effect: OverlayEffect): Promise<void> {
						if (effect.kind === "close") {
							done(undefined);
							return;
						}
						if (effect.kind === "load-patch") {
							const baseTreeSha =
								effect.mode === "request"
									? state.requestSnapshot?.treeSha ?? null
									: state.overallBaselineSha;
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
							return renderLines(model, width, stats?.isTruncated ?? false);
						},
						handleInput(data: string): void {
							const key = mapKey(data);
							if (key === null) {
								return;
							}
							const step = reduce(model, key);
							model = step.model;
							if (key === "toggle-mode") {
								model = rebuildRows(model, state.requestStats, state.overallStats);
							}
							if (step.effect !== null) {
								void runEffect(step.effect);
							}
							tui.requestRender();
						},
					};
				},
				{ overlay: true },
			);
		},
	});
}
