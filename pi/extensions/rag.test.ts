import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { chmod, mkdtemp, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import ragExtension from "./rag.ts";

type CommandResult = {
	stdout: string;
	stderr: string;
	code: number;
	isKilled: boolean;
};

type RegisteredTool = {
	name: string;
	parameters: {
		required: readonly string[];
		properties: Record<string, unknown>;
	};
	execute(
		toolCallId: string,
		params: { query: string; k?: number; source_filter?: string },
		signal?: AbortSignal,
	): Promise<{
		content: Array<{ type: string; text: string }>;
		details: { hits: Array<Record<string, unknown>> };
	}>;
};

function registerWith(exec: (command: string, args: string[], options?: { signal?: AbortSignal }) => Promise<CommandResult>) {
	let tool: RegisteredTool | undefined;
	const pi = {
		registerTool(candidate: RegisteredTool) {
			tool = candidate;
		},
		exec,
	};
	ragExtension(pi as never);
	assert.ok(tool);
	return tool;
}

function executeFile(command: string, args: string[], options?: { signal?: AbortSignal }): Promise<CommandResult> {
	return new Promise((resolve, reject) => {
		execFile(command, args, { signal: options?.signal }, (error, stdout, stderr) => {
			if (error) {
				reject(error);
				return;
			}
			resolve({ stdout, stderr, code: 0, isKilled: false });
		});
	});
}

test("registers search_memory and returns fake rag results", async () => {
	const directory = await mkdtemp(join(tmpdir(), "rag-extension-"));
	const argumentsPath = join(directory, "arguments.txt");
	const commandPath = join(directory, "rag");
	await writeFile(
		commandPath,
		`#!/bin/sh\nprintf '%s\\n' "$@" > "${argumentsPath}"\nprintf '%s\\n' '{"text":"first","source":"notes"}' '{"text":"second","source":"notes"}'\n`,
	);
	await chmod(commandPath, 0o755);

	const originalPath = process.env.PATH;
	process.env.PATH = `${directory}:${originalPath ?? ""}`;
	try {
		const tool = registerWith(executeFile);
		assert.equal(tool.name, "search_memory");
		assert.deepEqual(tool.parameters.required, ["query"]);
		assert.deepEqual(Object.keys(tool.parameters.properties), ["query", "k", "source_filter"]);

		const result = await tool.execute("call-1", {
			query: "pi extensions",
			k: 2,
			source_filter: "notes",
		});

		assert.deepEqual((await readFile(argumentsPath, "utf8")).trim().split("\n"), [
			"search",
			"pi extensions",
			"--k",
			"2",
			"--source",
			"notes",
			"--json",
		]);
		assert.equal(
			result.content[0]?.text,
			'[{"text":"first","source":"notes"},{"text":"second","source":"notes"}]',
		);
	} finally {
		process.env.PATH = originalPath;
	}
});

test("propagates command, output, and cancellation failures", async (context) => {
	await context.test("missing command", async () => {
		const error = Object.assign(new Error("spawn rag ENOENT"), { code: "ENOENT" });
		const tool = registerWith(async () => Promise.reject(error));
		await assert.rejects(tool.execute("call-2", { query: "query" }), /rag command was not found/);
	});

	await context.test("nonzero exit", async () => {
		const tool = registerWith(async () => ({
			stdout: "",
			stderr: "index unavailable",
			code: 2,
			isKilled: false,
		}));
		await assert.rejects(tool.execute("call-3", { query: "query" }), /index unavailable/);
	});

	await context.test("invalid output", async () => {
		const tool = registerWith(async () => ({
			stdout: "not-json\n",
			stderr: "",
			code: 0,
			isKilled: false,
		}));
		await assert.rejects(tool.execute("call-4", { query: "query" }), /invalid JSON output/);
	});

	await context.test("cancellation", async () => {
		const controller = new AbortController();
		controller.abort();
		const tool = registerWith(async () => ({ stdout: "", stderr: "", code: 0, isKilled: true }));
		await assert.rejects(tool.execute("call-5", { query: "query" }, controller.signal), /cancelled/);
	});
});
