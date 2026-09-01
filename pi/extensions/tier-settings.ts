// /tiers: an interactive settings-like UI for editing config/model-tiers.json's tiers and
// their models/thinking levels, instead of hand-editing the file.
//
// This is built on the exact same pieces the built-in /settings command is: pi-tui's
// SettingsList (list + submenu + search) and Input, styled with getSettingsListTheme(), shown
// via ctx.ui.custom() WITHOUT overlay:true. Omitting overlay is what makes it swap in for the
// editor in place (see interactive-mode's showSelector/custom path) instead of floating a
// bordered box on top of everything the way live-diff.ts's /diff does — that in-place swap is
// also why it coexists with a turn or a background agent still running: nothing about it pauses
// the session, it only changes what currently owns the input row.
//
// pi/extensions/tier-settings/model.ts owns the pure schema/validation/edit logic; this file
// only owns wiring that logic into real pi-tui components and the disk/process I/O.

import { execFile } from "node:child_process";
import { readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { promisify } from "node:util";

import { getSettingsListTheme, type ExtensionAPI, type ExtensionContext } from "@earendil-works/pi-coding-agent";
import { Input, SettingsList, type Component, type SettingItem, type TUI } from "@earendil-works/pi-tui";

import {
	applyEdit,
	entryAt,
	isValidModelId,
	isValidThinking,
	parseTierFile,
	slotsOf,
	THINKING_LEVELS,
	tierNames,
	type Slot,
	type TierFile,
} from "./tier-settings/model.ts";

const execFileAsync = promisify(execFile);
const MAX_VISIBLE = 10;

/**
 * Resolves the agents repo checkout this session is running in, so the command knows which
 * config/model-tiers.json and install.sh to touch. There is no recorded repo path anywhere
 * else an extension can read (`~/.pi/agent/agents/*.md` are plain generated copies, not
 * symlinks back to a source repo) — the session's own cwd is the only signal, so this only
 * works invoked from inside the repo (or a worktree of it), which matches how the command is
 * actually used: editing THIS repo's tiers, from within it.
 */
async function resolveRepoRoot(cwd: string): Promise<string | null> {
	try {
		const { stdout } = await execFileAsync("git", ["rev-parse", "--show-toplevel"], { cwd });
		const root = stdout.trim();
		await readFile(join(root, "config", "model-tiers.json"), "utf8");
		return root;
	} catch {
		return null;
	}
}

async function loadTierFile(repoRoot: string): Promise<TierFile> {
	const raw = await readFile(join(repoRoot, "config", "model-tiers.json"), "utf8");
	return parseTierFile(raw);
}

async function writeTierFile(repoRoot: string, file: TierFile): Promise<void> {
	await writeFile(join(repoRoot, "config", "model-tiers.json"), `${JSON.stringify(file, null, 2)}\n`, "utf8");
}

async function runInstall(repoRoot: string): Promise<{ isOk: boolean; message: string }> {
	try {
		const { stdout } = await execFileAsync(join(repoRoot, "install.sh"), [], {
			cwd: repoRoot,
			env: { ...process.env, REPO_TARGET: repoRoot },
		});
		const lines = stdout.trim().split("\n");
		return { isOk: true, message: lines.slice(-8).join("\n") || "install.sh finished with no output" };
	} catch (error) {
		const stderr = error instanceof Error && "stderr" in error ? String((error as { stderr?: unknown }).stderr ?? "") : "";
		const detail = stderr.trim() || (error instanceof Error ? error.message : String(error));
		return { isOk: false, message: `install.sh failed:\n${detail.slice(0, 2000)}` };
	}
}

function tierSummary(tier: TierFile["tiers"][string]): string {
	const fbCount = tier.fallbacks.length;
	return `${tier.pi.model} @ ${tier.pi.thinking}  (+${fbCount} fallback${fbCount === 1 ? "" : "s"})`;
}

function slotLabel(slot: Slot): string {
	return slot.kind === "pi" ? "primary" : `fallback ${slot.index + 1}`;
}

function slotSummary(entry: { model: string; thinking: string }): string {
	return `${entry.model} @ ${entry.thinking}`;
}

/**
 * A free-text model-id prompt used as a SettingsList submenu. Mirrors the submenu contract
 * settings-list.js documents: it owns its own Escape handling and is responsible for calling
 * `close` exactly once, either with the accepted new value or with the original on cancel.
 *
 * Deliberately starts EMPTY rather than prefilled with `currentValue`: pi-tui's `Input.setValue`
 * only clamps the cursor to the new text's length, it never moves it there (cursor starts and
 * stays at 0), so a prefilled field made every keystroke insert at the front and produced
 * mangled text like "newmodelold-model" instead of replacing it. Starting empty sidesteps that
 * whole class of cursor bugs outright: the current value is shown as a reference line above the
 * field, an empty submit is treated as "keep it", and Escape/any non-empty invalid submit always
 * falls back to the ORIGINAL string (never `input.getValue()`), so this can never hand a
 * SettingItem an `undefined`/empty `currentValue`.
 */
function modelIdPrompt(tui: TUI, currentValue: string, close: (value: string) => void): Component {
	const input = new Input();
	input.focused = true;
	let error: string | null = null;
	input.onSubmit = (value) => {
		const trimmed = value.trim();
		if (trimmed.length === 0) {
			close(currentValue);
			return;
		}
		if (!isValidModelId(trimmed)) {
			error = `invalid model id (expected provider/id): "${trimmed}"`;
			tui.requestRender();
			return;
		}
		close(trimmed);
	};
	input.onEscape = () => close(currentValue);
	return {
		invalidate(): void {
			input.invalidate();
		},
		render(width: number): string[] {
			const lines = [`Model id (provider/id) — current: ${currentValue}`, "", ...input.render(width)];
			if (error) {
				lines.push("", error);
			}
			lines.push("", "enter confirm (blank keeps current) · esc cancel");
			return lines;
		},
		handleInput(data: string): void {
			error = null;
			input.handleInput(data);
		},
	};
}

/**
 * Runs the actual write + install.sh once the user picks "Apply changes", and stays on screen
 * showing the result until any key is pressed. `onApplied` hands the new TierFile back up so
 * every open list (tier and slot) reflects it without needing to reload from disk.
 */
function applyFlow(
	tui: TUI,
	repoRoot: string,
	nextFile: TierFile,
	onApplied: (file: TierFile) => void,
	close: () => void,
): Component {
	let status: "running" | "done" = "running";
	let result: { isOk: boolean; message: string } | null = null;

	void (async () => {
		try {
			await writeTierFile(repoRoot, nextFile);
			onApplied(nextFile);
			result = await runInstall(repoRoot);
		} catch (error) {
			const message = error instanceof Error ? error.message : String(error);
			result = { isOk: false, message: `write failed:\n${message.slice(0, 2000)}` };
		}
		status = "done";
		tui.requestRender();
	})();

	return {
		invalidate(): void {},
		render(): string[] {
			if (status === "running") {
				return ["writing config/model-tiers.json and running install.sh…"];
			}
			const r = result as { isOk: boolean; message: string };
			return [r.isOk ? "install.sh applied the change" : "install.sh failed", "", ...r.message.split("\n"), "", "press any key to close"];
		},
		handleInput(): void {
			if (status === "done") {
				close();
			}
		},
	};
}

/**
 * The per-slot edit menu: model (free-text submenu), thinking (fixed-set cycle), and an
 * explicit "Apply changes" step — edits stay local until Apply runs, so cycling through
 * thinking levels or fixing a typo never triggers a write + install.sh run on every keystroke.
 */
function slotEditList(
	tui: TUI,
	repoRoot: string,
	getFile: () => TierFile,
	tierName: string,
	slot: Slot,
	onApplied: (file: TierFile) => void,
	closeToSlotList: (summary?: string) => void,
): SettingsList {
	const original = entryAt(getFile().tiers[tierName], slot);
	let draftModel = original.model;
	let draftThinking = original.thinking;

	const items: SettingItem[] = [
		{
			id: "model",
			label: "Model",
			currentValue: draftModel,
			submenu: (currentValue, close) =>
				modelIdPrompt(tui, currentValue, (value) => {
					draftModel = value;
					close(draftModel);
				}),
		},
		{
			id: "thinking",
			label: "Thinking",
			currentValue: draftThinking,
			values: THINKING_LEVELS,
		},
		{
			id: "apply",
			label: "Apply changes",
			currentValue: "write config/model-tiers.json + rerun install.sh",
			submenu: (_currentValue, close) =>
				applyFlow(
					tui,
					repoRoot,
					applyEdit(getFile(), tierName, slot, draftModel, draftThinking),
					onApplied,
					() => {
						close();
						closeToSlotList(slotSummary({ model: draftModel, thinking: draftThinking }));
					},
				),
		},
	];

	return new SettingsList(
		items,
		items.length,
		getSettingsListTheme(),
		(id, newValue) => {
			if (id === "thinking" && isValidThinking(newValue)) {
				draftThinking = newValue;
			}
		},
		() => closeToSlotList(),
	);
}

function slotList(
	tui: TUI,
	repoRoot: string,
	getFile: () => TierFile,
	tierName: string,
	onApplied: (file: TierFile) => void,
	closeToTierList: (summary?: string) => void,
): SettingsList {
	const items: SettingItem[] = slotsOf(getFile().tiers[tierName]).map(({ slot, entry }) => ({
		id: slot.kind === "pi" ? "pi" : `fallback-${slot.index}`,
		label: slotLabel(slot),
		currentValue: slotSummary(entry),
		submenu: (_currentValue, close) =>
			slotEditList(tui, repoRoot, getFile, tierName, slot, onApplied, (summary) => {
				close(summary);
			}),
	}));
	return new SettingsList(items, Math.min(items.length, MAX_VISIBLE), getSettingsListTheme(), () => {}, () =>
		closeToTierList(tierSummary(getFile().tiers[tierName])),
	);
}

function tiersList(tui: TUI, repoRoot: string, initialFile: TierFile, onExit: () => void): SettingsList {
	let file = initialFile;
	const items: SettingItem[] = tierNames(file).map((name) => ({
		id: name,
		label: name,
		currentValue: tierSummary(file.tiers[name]),
		submenu: (_currentValue, close) =>
			slotList(
				tui,
				repoRoot,
				() => file,
				name,
				(nextFile) => {
					file = nextFile;
				},
				(summary) => close(summary),
			),
	}));
	return new SettingsList(items, Math.min(items.length, MAX_VISIBLE), getSettingsListTheme(), () => {}, onExit, { enableSearch: true });
}

export default function tierSettings(pi: ExtensionAPI): void {
	pi.registerCommand("tiers", {
		description: "Browse and edit config/model-tiers.json's tiers and models",
		handler: async (_args: string, ctx: ExtensionContext) => {
			if (!ctx.hasUI || ctx.mode !== "tui") {
				return;
			}
			const repoRoot = await resolveRepoRoot(ctx.cwd);
			if (!repoRoot) {
				ctx.ui.notify("/tiers only works run from inside the agents repo (config/model-tiers.json not found from cwd).", "warning");
				return;
			}
			let file: TierFile;
			try {
				file = await loadTierFile(repoRoot);
			} catch (error) {
				ctx.ui.notify(`/tiers: could not read config/model-tiers.json: ${error instanceof Error ? error.message : String(error)}`, "error");
				return;
			}

			// No `overlay: true` here on purpose — see the file header. This swaps in for the
			// editor row, exactly like /settings, instead of floating a modal over everything.
			await ctx.ui.custom<undefined>((tui, _theme, _keybindings, done) => tiersList(tui, repoRoot, file, () => done(undefined)));
		},
	});
}
