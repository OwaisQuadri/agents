import { isToolCallEventType, type ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { spawn } from "node:child_process";
import { existsSync, readFileSync, realpathSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { appendUnverified, appendVerdict, hashCommentText, readVerdict, unverifiedPath, verdictsPath, type Verdict } from "./comment-shape-guard/cache.ts";
import { readWorkerVerdict, runResultPath } from "./comment-shape-guard/spawn/runs.ts";
import { buildWorkerArgv, buildWorkerEnv, COMMENT_STYLE_DOC_PATH, resolveJudgeModel, spawnWorker } from "./comment-shape-guard/spawn/launch.ts";
import { buildKickoffPrompt } from "./comment-shape-guard/judge/prompt.ts";
import { editFragments, extensionOf, followingContextOnDisk, followingContextWithinFragment, writeFragment, type CommentSpan } from "./comment-shape-guard/policy.ts";

const JUDGE_TIMEOUT_MS = 20_000;

type Fragment = { path: string; text: string; oldText: string | undefined };

/** Runs `comment-check --list-json --lang EXT` over `text` via stdin. Missing binary
 * or a nonzero exit degrades to an empty span list (fail open) — same posture
 * `preferred-cli-guard.ts` takes on a missing build: never a false block over a
 * checker that has not been compiled yet. */
function listCommentSpans(binary: string, ext: string, text: string): Promise<CommentSpan[]> {
	return new Promise((resolvePromise) => {
		if (!existsSync(binary)) return resolvePromise([]);
		const proc = spawn(binary, ["--list-json", "--lang", ext], { stdio: ["pipe", "pipe", "ignore"] });
		let stdout = "";
		proc.stdout.on("data", (d: Buffer) => {
			stdout += d.toString();
		});
		proc.on("error", () => resolvePromise([]));
		proc.on("close", (code) => {
			if (code !== 0) return resolvePromise([]);
			try {
				resolvePromise(JSON.parse(stdout) as CommentSpan[]);
			} catch {
				resolvePromise([]);
			}
		});
		// EPIPE lands on the stdin stream, not the process 'error' event; unhandled it
		// kills the whole pi session when the child exits before consuming stdin.
		proc.stdin.on("error", () => resolvePromise([]));
		proc.stdin.write(text);
		proc.stdin.end();
	});
}

/** Dispatches one span to the headless judge worker with a fail-open timeout. Returns
 * undefined on timeout, spawn failure, or a malformed result — the caller's single
 * fail-open path (allow + log to unverified.jsonl) handles every case identically. */
async function judgeSpan(opts: { commentText: string; followingContext: string | undefined; whitelistDocText: string; runId: string; home: string }): Promise<{ shape: string; reason: string } | undefined> {
	const resultPath = runResultPath(opts.runId, opts.home);
	const kickoffPrompt = buildKickoffPrompt({ commentText: opts.commentText, followingContext: opts.followingContext, whitelistDocText: opts.whitelistDocText });
	const argv = buildWorkerArgv({ model: resolveJudgeModel(), sessionName: `comment-shape-guard-${opts.runId}`, kickoffPrompt });
	const env = buildWorkerEnv(resultPath);
	const controller = new AbortController();
	const timeout = setTimeout(() => controller.abort(), JUDGE_TIMEOUT_MS);
	try {
		const exit = await spawnWorker({ argv, cwd: dirname(resultPath), env, signal: controller.signal });
		if (exit.code !== 0) return undefined;
		return readWorkerVerdict(resultPath);
	} finally {
		clearTimeout(timeout);
	}
}

async function judgeFragment(opts: { fragment: Fragment; ext: string; binary: string; whitelistDocText: string; home: string }): Promise<{ blocked: string[] }> {
	const unverified = unverifiedPath(opts.home);
	const verdicts = verdictsPath(opts.home);
	const spans = await listCommentSpans(opts.binary, opts.ext, opts.fragment.text);
	const blocked: string[] = [];
	await Promise.all(
		spans.map(async (span) => {
			let followingContext = followingContextWithinFragment(opts.fragment.text, span) || undefined;
			if (!followingContext && opts.fragment.oldText !== undefined) {
				followingContext = followingContextOnDisk(readFileSync(opts.fragment.path, "utf-8"), opts.fragment.oldText);
			}
			const hash = hashCommentText(span.text);
			let shape: string;
			let reason: string;
			const cached = readVerdict(hash, verdicts);
			if (cached) {
				shape = cached.shape;
				reason = cached.reason;
			} else {
				const runId = `${hash}-${Math.random().toString(36).slice(2)}`;
				const verdict = await judgeSpan({ commentText: span.text, followingContext, whitelistDocText: opts.whitelistDocText, runId, home: opts.home });
				if (!verdict) {
					try {
						appendUnverified({ hash, reason: "judge worker timed out, failed to spawn, or returned no result", at: new Date().toISOString() }, unverified);
					} catch {}
					return;
				}
				const record: Verdict = { hash, shape: verdict.shape, reason: verdict.reason, judgedAt: new Date().toISOString() };
				try {
					appendVerdict(record, verdicts);
				} catch {}
				shape = verdict.shape;
				reason = verdict.reason;
			}
			if (shape.toLowerCase() === "none") {
				blocked.push(`${opts.fragment.path}:${span.startLine}-${span.endLine}: "${span.text}" does not fit any docs/comment-style.md whitelist shape (${reason})`);
			}
		}),
	);
	return { blocked };
}

export default function commentShapeGuard(pi: ExtensionAPI, home = process.env.HOME ?? homedir()): void {
	const extensionPath = realpathSync(fileURLToPath(import.meta.url));
	const repositoryRoot = resolve(dirname(extensionPath), "..", "..");
	const binary = resolve(repositoryRoot, "tools/comment-check/target/release/comment-check");

	pi.on("tool_call", async (event) => {
		let fragments: Fragment[] = [];
		if (isToolCallEventType("edit", event)) {
			fragments = editFragments(event.input).map((f) => ({ path: f.path, text: f.newText, oldText: f.oldText }));
		} else if (isToolCallEventType("write", event)) {
			const f = writeFragment(event.input);
			fragments = [{ path: f.path, text: f.content, oldText: undefined }];
		} else {
			return;
		}

		const inScope = fragments.filter((f) => extensionOf(f.path) !== undefined);
		if (inScope.length === 0) return;

		if (!existsSync(COMMENT_STYLE_DOC_PATH)) return; // no whitelist to judge against — fail open
		const whitelistDocText = readFileSync(COMMENT_STYLE_DOC_PATH, "utf-8");

		const results = await Promise.all(
			inScope.map((fragment) => {
				const ext = extensionOf(fragment.path);
				if (!ext) return Promise.resolve({ blocked: [] as string[] });
				return judgeFragment({ fragment, ext, binary, whitelistDocText, home });
			}),
		);
		const blocked = results.flatMap((r) => r.blocked);
		if (blocked.length > 0) {
			return { block: true, reason: `Blocked: comment does not fit docs/comment-style.md's whitelist.\n${blocked.join("\n")}` };
		}
	});
}
