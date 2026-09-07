import { randomUUID } from "node:crypto";
import { mkdtemp, readFile, realpath, rm, stat } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { checkDeadline, ValidationCancelled, ValidationExpired, type Proposal } from "./direct.ts";
import { judgmentKey, readDecision, writeDecision } from "./decision-cache.ts";
import { blockingMessage, validateResponse, type JudgmentRequest, type Request } from "./protocol.ts";
import { runProcess, type ProcessResult } from "./process.ts";
import { buildKickoffPrompt, JUDGE_SYSTEM } from "./judge/prompt.ts";
import { buildWorkerArgv, buildWorkerEnv, resolveJudgeModel, spawnWorker } from "./spawn/launch.ts";
import { readWorkerVerdict } from "./spawn/runs.ts";

const shapes = new Set(["none", "inexpressible concept or architecture", "standard-violation exception", "TODO", "advanced math / physics / formula", "docstring on a public API declaration", "commented-out reference template"]);
let pending: Promise<void> = Promise.resolve();

export async function judgeRequests(judgments: JudgmentRequest[], root: string, deadline: number, signal?: AbortSignal): Promise<number[]> {
	checkDeadline(deadline, signal);
	const tiersPath = join(root, "config", "model-tiers.json");
	const tiers = await readFile(tiersPath, "utf8");
	checkDeadline(deadline, signal);
	const model = resolveJudgeModel(tiersPath);
	const failed: number[] = [];
	for (const judgment of judgments) {
		checkDeadline(deadline, signal);
		const input = { ...judgment.input, judgment_configuration: JSON.stringify([judgment.input.judgment_configuration, tiers, model, JUDGE_SYSTEM, buildKickoffPrompt.toString()]) };
		const key = judgmentKey(input);
		let decision = readDecision(key);
		checkDeadline(deadline, signal);
		if (!decision) {
			const directory = await mkdtemp(join(tmpdir(), "comment-judgment-"));
			try {
				checkDeadline(deadline, signal);
				const resultPath = join(directory, "result.json");
				const prompt = buildKickoffPrompt({ commentText: input.comment, followingContext: input.code_context || undefined, whitelistDocText: input.rule_document, language: input.language });
				const argv = buildWorkerArgv({ model, sessionName: `comment-shape-${randomUUID()}`, kickoffPrompt: prompt });
				const exit = await spawnWorker({ argv, cwd: directory, env: buildWorkerEnv(resultPath), signal, deadline });
				checkDeadline(deadline, signal);
				if (exit.code !== 0 || (await stat(resultPath)).size > 4096) throw new Error("Required comment-shape judgment failed.");
				checkDeadline(deadline, signal);
				const verdict = readWorkerVerdict(resultPath);
				if (!verdict || !shapes.has(verdict.shape)) throw new Error("Invalid comment-shape judgment.");
				decision = { version: 1, key, decision: verdict.shape === "none" ? "block" : "pass", reason: verdict.shape === "none" ? "No approved comment shape." : "Approved comment shape." };
			} finally {
				await rm(directory, { recursive: true, force: true });
			}
			checkDeadline(deadline, signal);
			writeDecision(decision);
		}
		if (decision.decision === "block") failed.push(judgment.line);
	}
	checkDeadline(deadline, signal);
	return failed;
}

export function validateCheckerResult(result: ProcessResult, request: Request, start: number) {
	const response = validateResponse(result.stdout, request);
	const expectedExit = response.decision === "block" ? 1 : response.decision === "error" ? 2 : 0;
	if (result.exitCode !== expectedExit) throw new Error("Checker exit status does not match its decision.");
	if (response.decision === "block" || response.decision === "error") throw new Error(blockingMessage(response.diagnostics, performance.now() - start));
	return response;
}

async function waitForPrevious(previous: Promise<void>, deadline: number, signal?: AbortSignal): Promise<void> {
	checkDeadline(deadline, signal);
	await new Promise<void>((resolve, reject) => {
		const finish = (error?: Error) => {
			clearTimeout(timer);
			signal?.removeEventListener("abort", abort);
			if (error) reject(error); else resolve();
		};
		const abort = () => finish(new ValidationCancelled());
		const timer = setTimeout(() => finish(new ValidationExpired()), Math.max(1, deadline - performance.now()));
		signal?.addEventListener("abort", abort, { once: true });
		if (signal?.aborted) abort();
		previous.then(() => finish());
	});
	checkDeadline(deadline, signal);
}

export async function validateProposal(proposal: Proposal, start: number, deadline: number, signal?: AbortSignal): Promise<number> {
	deadline = Math.min(deadline, start + 20_000);
	const previous = pending;
	let release!: () => void;
	const slot = new Promise<void>((resolvePromise) => { release = resolvePromise; });
	pending = previous.then(() => slot);
	try {
		await waitForPrevious(previous, deadline, signal);
		const root = resolve(dirname(await realpath(fileURLToPath(import.meta.url))), "..", "..", "..");
		checkDeadline(deadline, signal);
		const judgmentConfiguration = await readFile(join(root, "config", "model-tiers.json"), "utf8");
		checkDeadline(deadline, signal);
		const args = ["--config", join(root, "config", "edit-time.toml"), "--rule-document", join(root, "docs", "comment-style.md"), "--code-style", join(root, "docs", "code-style.md"), "--judgment-configuration", judgmentConfiguration];
		const workerDeadline = Math.min(deadline, performance.now() + 3000);
		const request: Request = { version: 1, request_id: randomUUID(), ...proposal, changed_ranges: [], budget_ms: Math.min(20_000, Math.ceil(deadline - start)) };
		const response = validateCheckerResult(await runProcess({ resultMode: "exit-status", command: join(root, "tools", "edit-time-check", "target", "release", "edit-time-check"), args, input: JSON.stringify(request), cwd: root, deadline: workerDeadline, signal }), request, start);
		deadline = Math.min(deadline, start + response.budget_ms);
		checkDeadline(deadline, signal);
		if (response.decision === "needs_judgment") {
			const failures = await judgeRequests(response.judgments, root, deadline, signal);
			if (failures.length) throw new Error(blockingMessage(failures.map((line) => ({ path: proposal.path, line, rule: "comment-shape" })), performance.now() - start));
		}
		checkDeadline(deadline, signal);
		return deadline;
	} catch (error) {
		checkDeadline(deadline, signal);
		if (error instanceof ValidationCancelled || error instanceof ValidationExpired) throw error;
		if (error instanceof Error && error.message.startsWith("Blocked before write")) throw error;
		throw new Error(`Blocked before write (${Math.ceil(performance.now() - start)}ms): required validation failed. Build or repair the checker, rules, configuration, or judgment worker and retry. 0 diagnostics omitted.`);
	} finally {
		release();
	}
}
