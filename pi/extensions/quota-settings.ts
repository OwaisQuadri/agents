import { execFile } from "node:child_process";
import { readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { promisify } from "node:util";

import { getSettingsListTheme, type ExtensionAPI, type ExtensionContext } from "@earendil-works/pi-coding-agent";
import { SettingsList, type Component, type SettingItem, type TUI } from "@earendil-works/pi-tui";

import { parseQuotaOverride, updateQuotaOverride, type QuotaProvider } from "./quota-settings/model.ts";

const execFileAsync = promisify(execFile);
const OVERRIDE_LABELS = ["No override", "Anthropic", "OpenAI Codex"];

function providerLabel(provider: QuotaProvider | null): string {
	if (provider === "anthropic") return "Anthropic";
	if (provider === "openai-codex") return "OpenAI Codex";
	return "No override";
}

function providerFromLabel(label: string): QuotaProvider | null {
	if (label === "Anthropic") return "anthropic";
	if (label === "OpenAI Codex") return "openai-codex";
	return null;
}

async function resolveRepoRoot(cwd: string): Promise<string | null> {
	try {
		const { stdout } = await execFileAsync("git", ["rev-parse", "--show-toplevel"], { cwd });
		const root = stdout.trim();
		await readFile(join(root, "config", "pi-settings.json"), "utf8");
		return root;
	} catch {
		return null;
	}
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

function applyFlow(
	tui: TUI,
	repoRoot: string,
	provider: QuotaProvider | null,
	close: (summary?: string) => void,
): Component {
	let isDone = false;
	let result: { isOk: boolean; message: string } | null = null;

	void (async () => {
		try {
			const path = join(repoRoot, "config", "pi-settings.json");
			const currentSettings = await readFile(path, "utf8");
			await writeFile(path, updateQuotaOverride(currentSettings, provider), "utf8");
			result = await runInstall(repoRoot);
		} catch (error) {
			const message = error instanceof Error ? error.message : String(error);
			result = { isOk: false, message: `write failed:\n${message.slice(0, 2000)}` };
		}
		isDone = true;
		tui.requestRender();
	})();

	return {
		invalidate(): void {},
		render(): string[] {
			if (!isDone) return ["writing config/pi-settings.json and running install.sh…"];
			const applied = result as { isOk: boolean; message: string };
			return [
				applied.isOk ? "install.sh applied the change" : "install.sh failed",
				"",
				...applied.message.split("\n"),
				"",
				"press any key to close",
			];
		},
		handleInput(): void {
			if (isDone) close(providerLabel(provider));
		},
	};
}

function quotaList(tui: TUI, repoRoot: string, rawSettings: string, onExit: () => void): SettingsList {
	let draftProvider = parseQuotaOverride(rawSettings);
	const items: SettingItem[] = [
		{
			id: "provider",
			label: "Provider override",
			currentValue: providerLabel(draftProvider),
			values: OVERRIDE_LABELS,
		},
		{
			id: "apply",
			label: "Apply changes",
			currentValue: "write config/pi-settings.json + rerun install.sh",
			submenu: (_currentValue, close) => applyFlow(tui, repoRoot, draftProvider, close),
		},
	];
	return new SettingsList(
		items,
		items.length,
		getSettingsListTheme(),
		(id, value) => {
			if (id === "provider") draftProvider = providerFromLabel(value);
		},
		onExit,
	);
}

export default function quotaSettings(pi: ExtensionAPI): void {
	pi.registerCommand("quota", {
		description: "Choose a provider to exempt from quota admission",
		handler: async (_args: string, ctx: ExtensionContext) => {
			if (!ctx.hasUI || ctx.mode !== "tui") return;
			const repoRoot = await resolveRepoRoot(ctx.cwd);
			if (!repoRoot) {
				ctx.ui.notify("/quota only works inside the agents repo.", "warning");
				return;
			}
			let rawSettings: string;
			try {
				rawSettings = await readFile(join(repoRoot, "config", "pi-settings.json"), "utf8");
				parseQuotaOverride(rawSettings);
			} catch (error) {
				ctx.ui.notify(`/quota could not read config/pi-settings.json: ${error instanceof Error ? error.message : String(error)}`, "error");
				return;
			}
			await ctx.ui.custom<undefined>((tui, _theme, _keybindings, done) =>
				quotaList(tui, repoRoot, rawSettings, () => done(undefined)),
			);
		},
	});
}
