import assert from "node:assert/strict";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { test } from "node:test";
import transcriptDirectedVideoProcessorExtension, {
	type CommandResult,
} from "./transcript-directed-video-processor.ts";

type ExecCall = { command: string; args: string[]; options?: { cwd?: string; signal?: AbortSignal } };

type RegisteredTool = {
	name: string;
	parameters: {
		required?: readonly string[];
		properties: Record<string, unknown>;
	};
	execute(
		toolCallId: string,
		params: Record<string, unknown>,
		signal?: AbortSignal,
	): Promise<{ content: Array<{ type: string; text: string }>; details: Record<string, unknown> }>;
};

function registerWith(exec: (command: string, args: string[], options?: { cwd?: string; signal?: AbortSignal }) => Promise<CommandResult>) {
	const tools = new Map<string, RegisteredTool>();
	const pi = {
		registerTool(candidate: RegisteredTool) {
			tools.set(candidate.name, candidate);
		},
		exec,
	};
	transcriptDirectedVideoProcessorExtension(pi as never);
	assert.ok(tools.get("video_analyze"));
	assert.ok(tools.get("video_review"));
	return tools;
}

function fakeResult(overrides: Partial<CommandResult> = {}): CommandResult {
	return { stdout: "", stderr: "", code: 0, isKilled: false, ...overrides };
}

test("video_analyze creates a scratch work_dir, runs analyze there, and returns parsed chapters", async () => {
	const calls: ExecCall[] = [];
	const tools = registerWith(async (command, args, options) => {
		calls.push({ command, args, options });
		await writeFile(
			join(options?.cwd as string, "chapters.json"),
			JSON.stringify({ source: { origin: "youtube" }, moments: [{ index: 0, start_s: 0, end_s: 3, title: "intro" }] }),
		);
		return fakeResult();
	});

	const tool = tools.get("video_analyze");
	assert.ok(tool);
	const result = await tool.execute("call-1", { url: "https://youtube.com/watch?v=abc" });

	assert.equal(calls.length, 1);
	assert.equal(calls[0]?.command, "transcript-directed-video-processor");
	assert.deepEqual(calls[0]?.args.slice(0, 3), ["analyze", "--url", "https://youtube.com/watch?v=abc"]);
	assert.deepEqual(calls[0]?.args.slice(3), ["--out", "."]);

	const workDir = (result.details as { work_dir: string }).work_dir;
	assert.ok(workDir.includes("tdvp-sessions"));
	const parsed = JSON.parse(result.content[0]?.text ?? "{}");
	assert.equal(parsed.work_dir, workDir);
	assert.deepEqual(parsed.moments, [{ index: 0, start_s: 0, end_s: 3, title: "intro" }]);

	await rm(workDir, { recursive: true, force: true });
});

test("video_analyze rejects zero or both of url and input", async () => {
	const tools = registerWith(async () => fakeResult());
	const tool = tools.get("video_analyze");
	assert.ok(tool);
	await assert.rejects(tool.execute("call-2", {}), /exactly one of url or input/);
	await assert.rejects(
		tool.execute("call-3", { url: "https://x", input: "/tmp/y.mp4" }),
		/exactly one of url or input/,
	);
});

test("video_review runs review inside the given work_dir and returns parsed evidence", async () => {
	const workDir = await mkdtemp(join(tmpdir(), "tdvp-sessions", "session-"));
	try {
		const calls: ExecCall[] = [];
		const tools = registerWith(async (command, args, options) => {
			calls.push({ command, args, options });
			await writeFile(
				join(options?.cwd as string, "evidence.json"),
				JSON.stringify([{ moment_index: 0, model_response: "a red frame" }]),
			);
			return fakeResult();
		});

		const tool = tools.get("video_review");
		assert.ok(tool);
		const result = await tool.execute("call-4", {
			work_dir: workDir,
			moments: [0, 5],
			model: "gpt-5.1",
			clip: true,
		});

		assert.equal(calls[0]?.options?.cwd, workDir);
		assert.deepEqual(calls[0]?.args, [
			"review",
			"--dir",
			".",
			"--moments",
			"0,5",
			"--model",
			"gpt-5.1",
			"--clip",
			"yes",
		]);
		const evidence = JSON.parse(result.content[0]?.text ?? "[]");
		assert.deepEqual(evidence, [{ moment_index: 0, model_response: "a red frame" }]);
	} finally {
		await rm(workDir, { recursive: true, force: true });
	}
});

test("video_review omits --clip when clip is not set", async () => {
	const workDir = await mkdtemp(join(tmpdir(), "tdvp-sessions", "session-"));
	try {
		const calls: ExecCall[] = [];
		const tools = registerWith(async (command, args, options) => {
			calls.push({ command, args, options });
			await writeFile(join(options?.cwd as string, "evidence.json"), "[]");
			return fakeResult();
		});

		const tool = tools.get("video_review");
		assert.ok(tool);
		await tool.execute("call-5", { work_dir: workDir, moments: [0], model: "gpt-5.1" });

		assert.deepEqual(calls[0]?.args, ["review", "--dir", ".", "--moments", "0", "--model", "gpt-5.1"]);
	} finally {
		await rm(workDir, { recursive: true, force: true });
	}
});

test("video_review rejects a work_dir outside the managed scratch root", async () => {
	const tools = registerWith(async () => fakeResult());
	const tool = tools.get("video_review");
	assert.ok(tool);
	await assert.rejects(
		tool.execute("call-6", { work_dir: "/etc", moments: [0], model: "gpt-5.1" }),
		/work_dir must be a directory returned by video_analyze/,
	);
	await assert.rejects(
		tool.execute("call-7", { work_dir: resolve(tmpdir(), "tdvp-sessions-evil"), moments: [0], model: "gpt-5.1" }),
		/work_dir must be a directory returned by video_analyze/,
	);
});

test("propagates missing command, nonzero exit, and cancellation failures", async (context) => {
	await context.test("missing command", async () => {
		const error = Object.assign(new Error("spawn transcript-directed-video-processor ENOENT"), { code: "ENOENT" });
		const tools = registerWith(async () => Promise.reject(error));
		const tool = tools.get("video_analyze");
		assert.ok(tool);
		await assert.rejects(tool.execute("call-8", { url: "https://x" }), /was not found on PATH/);
	});

	await context.test("nonzero exit surfaces stderr", async () => {
		const tools = registerWith(async () => fakeResult({ stderr: "invalid --url and --input together", code: 2 }));
		const tool = tools.get("video_analyze");
		assert.ok(tool);
		await assert.rejects(tool.execute("call-9", { url: "https://x" }), /invalid --url and --input together/);
	});

	await context.test("nonzero exit without stderr falls back to exit code", async () => {
		const tools = registerWith(async () => fakeResult({ code: 1 }));
		const tool = tools.get("video_analyze");
		assert.ok(tool);
		await assert.rejects(tool.execute("call-10", { url: "https://x" }), /failed with exit code 1/);
	});

	await context.test("cancellation", async () => {
		const controller = new AbortController();
		controller.abort();
		const tools = registerWith(async () => fakeResult({ isKilled: true }));
		const tool = tools.get("video_analyze");
		assert.ok(tool);
		await assert.rejects(tool.execute("call-11", { url: "https://x" }, controller.signal), /was cancelled/);
	});
});
