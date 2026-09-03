/**
 * Headless judge worker launch — the same `pi -e <ext> -p <prompt>` subprocess pattern
 * `pi/extensions/observational-memory/src/spawn/launch.ts` already uses for its
 * observer/consolidator workers. Not shared code (observational-memory is a separately
 * versioned package) but the same shape on purpose, so a reader who already knows one
 * recognizes the other.
 */
import { spawn } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, realpathSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { parseTierFile, type TierEntry } from "../../tier-settings/model.ts";

const REPO_ROOT = resolve(dirname(realpathSync(fileURLToPath(import.meta.url))), "..", "..", "..", "..");
export const JUDGE_AGENT_EXTENSION_PATH = join(REPO_ROOT, "pi", "extensions", "comment-shape-guard", "judge", "agent", "index.ts");
export const COMMENT_STYLE_DOC_PATH = join(REPO_ROOT, "docs", "comment-style.md");
const MODEL_TIERS_PATH = join(REPO_ROOT, "config", "model-tiers.json");

/** T2 ("cheap summarization, classification, boilerplate" per docs/routing.md's own
 * tier table) is the judge's tier — resolved from config/model-tiers.json at call time,
 * never hand-picked, per this repo's model-routing rule. Falls back to a fixed model
 * only if the tiers file is missing or malformed (should not happen post-install.sh,
 * but a judge worker must never crash the guard extension over a config read). */
export function resolveJudgeModel(tiersPath = MODEL_TIERS_PATH): TierEntry {
	try {
		const file = parseTierFile(readFileSync(tiersPath, "utf-8"));
		return file.tiers.T2.pi;
	} catch {
		return { model: "anthropic/claude-haiku-4-5", thinking: "medium" };
	}
}

/** Resolve the `pi` entry point (same trick observational-memory uses), falling back to
 * `pi` on PATH. */
export function resolvePiBinary(): { command: string; baseArgs: string[] } {
	const entry = process.argv[1];
	if (entry) {
		try {
			const realEntry = realpathSync(entry);
			if (/\.(?:mjs|cjs|js)$/i.test(realEntry)) {
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

/** Spawn the judge; resolve when it exits or `signal` aborts it (the guard extension's
 * 20-second fail-open timeout). Never rejects — a spawn error resolves with a non-zero
 * code instead, so the caller's single fail-open path handles every failure mode. */
export function spawnWorker(opts: { argv: string[]; cwd: string; env: NodeJS.ProcessEnv; signal?: AbortSignal }): Promise<WorkerExit> {
	const [command, ...rest] = opts.argv;
	if (!existsSync(opts.cwd)) mkdirSync(opts.cwd, { recursive: true });
	return new Promise<WorkerExit>((resolvePromise) => {
		const proc = spawn(command, rest, { cwd: opts.cwd, env: opts.env, stdio: ["ignore", "ignore", "pipe"] });
		let stderr = "";
		proc.stderr?.on("data", (d: Buffer) => {
			stderr += d.toString();
		});
		proc.on("error", () => resolvePromise({ code: 1, signal: null, stderr: stderr || "spawn error" }));
		proc.on("close", (code, signal) => resolvePromise({ code, signal, stderr }));

		if (opts.signal) {
			const kill = () => {
				proc.kill("SIGTERM");
				setTimeout(() => {
					if (!proc.killed) proc.kill("SIGKILL");
				}, 3000).unref?.();
			};
			if (opts.signal.aborted) kill();
			else opts.signal.addEventListener("abort", kill, { once: true });
		}
	});
}
