/**
 * Headless judge worker launch — the same `pi -e <ext> -p <prompt>` subprocess pattern
 * `pi/extensions/observational-memory/src/spawn/launch.ts` already uses for its
 * observer/consolidator workers. Not shared code (observational-memory is a separately
 * versioned package) but the same shape on purpose, so a reader who already knows one
 * recognizes the other.
 */
import { readFileSync, realpathSync } from "node:fs";
import { runProcess } from "../process.ts";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { parseTierFile, type TierEntry } from "../../tier-settings/model.ts";

const REPO_ROOT = resolve(dirname(realpathSync(fileURLToPath(import.meta.url))), "..", "..", "..", "..");
export const JUDGE_AGENT_EXTENSION_PATH = join(REPO_ROOT, "pi", "extensions", "comment-shape-guard", "judge", "agent", "index.ts");
export const COMMENT_STYLE_DOC_PATH = join(REPO_ROOT, "docs", "comment-style.md");
const MODEL_TIERS_PATH = join(REPO_ROOT, "config", "model-tiers.json");

export function resolveJudgeModel(tiersPath = MODEL_TIERS_PATH): TierEntry {
	return parseTierFile(readFileSync(tiersPath, "utf-8")).tiers.T2.pi;
}

/** Resolve the `pi` entry point (same trick observational-memory uses), falling back to
 * `pi` on PATH. */
export function resolvePiBinary(): { command: string; baseArgs: string[] } {
	const entry = process.argv[1];
	if (entry) {
		try {
			const realEntry = realpathSync(entry);
			if (realEntry.endsWith("/pi-coding-agent/dist/bundle/cli.js")) {
				return { command: process.execPath, baseArgs: [realEntry] };
			}
		} catch {
			// fall through
		}
	}
	return { command: "pi", baseArgs: [] };
}

export function buildWorkerArgv(opts: { model: TierEntry; sessionName: string; kickoffPrompt: string }): string[] {
	const pi = resolvePiBinary();
	const args = [...pi.baseArgs, "--no-extensions", "--no-skills", "--no-prompt-templates", "--no-context-files", "--no-builtin-tools", "--model", opts.model.model, "--thinking", opts.model.thinking];
	args.push("-e", JUDGE_AGENT_EXTENSION_PATH);
	args.push("-n", opts.sessionName);
	args.push("-p", opts.kickoffPrompt);
	return [pi.command, ...args];
}

export function buildWorkerEnv(resultPath: string): NodeJS.ProcessEnv {
	return { ...process.env, CSG_RESULT_PATH: resultPath };
}

export type WorkerExit = { code: number | null; signal: NodeJS.Signals | null; stderr: string };

export async function spawnWorker(opts: { argv: string[]; cwd: string; env: NodeJS.ProcessEnv; signal?: AbortSignal; deadline: number }): Promise<WorkerExit> {
	const [command, ...args] = opts.argv;
	try {
		await runProcess({ command, args, cwd: opts.cwd, env: opts.env, signal: opts.signal, deadline: opts.deadline, input: "" });
		return { code: 0, signal: null, stderr: "" };
	} catch {
		return { code: 1, signal: null, stderr: "Required judgment worker failed." };
	}
}
