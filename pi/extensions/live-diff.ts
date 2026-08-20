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
} from "./live-diff/types.ts";

const MAX_FILES = 400;
const DEBOUNCE_MS = 300;
const WRITE_TOOLS = new Set(["edit", "write", "bash"]);

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
	};
	let isDirtyPending = false;
	let lastRefreshAt = 0;
	let isSessionGone = false;

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
		state.overallBaselineSha = snapshot.treeSha;
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

	pi.on("session_start", (_event, ctx) => {
		isSessionGone = false;
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
