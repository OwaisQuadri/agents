import assert from "node:assert/strict";
import { access, mkdir, mkdtemp, readFile, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { recordSnippetUsage } from "./usage.ts";

async function withUsageDirectory(run: (directory: string) => Promise<void>): Promise<void> {
	const directory = await mkdtemp(join(tmpdir(), "prompt-snippets-usage-"));
	const originalDirectory = process.env.PI_CODING_AGENT_DIR;
	process.env.PI_CODING_AGENT_DIR = join(directory, "agent");

	try {
		await run(directory);
	} finally {
		if (originalDirectory === undefined) delete process.env.PI_CODING_AGENT_DIR;
		else process.env.PI_CODING_AGENT_DIR = originalDirectory;
		await rm(directory, { recursive: true, force: true });
	}
}

test("writes only the exact four usage-record fields", async () => {
	await withUsageDirectory(async (directory) => {
		await recordSnippetUsage("session-1", ["orient-wait.md"]);

		const stored = JSON.parse(await readFile(join(directory, "agent", "prompt-snippets-usage.jsonl"), "utf8"));
		assert.deepEqual(Object.keys(stored), ["version", "timestamp", "sessionId", "snippetIds"]);
		assert.equal(stored.version, 1);
		assert.match(stored.timestamp, /^\d{4}-\d{2}-\d{2}T/);
		assert.equal(stored.sessionId, "session-1");
	});
});

test("preserves selected identifiers in injection order", async () => {
	await withUsageDirectory(async (directory) => {
		await recordSnippetUsage("session-1", ["orient-wait.md", "clarify-verify.md"]);

		const stored = JSON.parse(await readFile(join(directory, "agent", "prompt-snippets-usage.jsonl"), "utf8"));
		assert.deepEqual(stored.snippetIds, ["orient-wait.md", "clarify-verify.md"]);
	});
});

test("creates the usage directory on first selected use", async () => {
	await withUsageDirectory(async (directory) => {
		await recordSnippetUsage("session-1", ["orient-wait.md"]);
		await access(join(directory, "agent", "prompt-snippets-usage.jsonl"));
	});
});

test("does not create a usage record without selected snippets", async () => {
	await withUsageDirectory(async (directory) => {
		await recordSnippetUsage("session-1", []);
		await assert.rejects(readFile(join(directory, "agent", "prompt-snippets-usage.jsonl"), "utf8"), { code: "ENOENT" });
	});
});

test("swallows local write failures", async () => {
	await withUsageDirectory(async () => {
		process.env.PI_CODING_AGENT_DIR = "/dev/null";
		await assert.doesNotReject(recordSnippetUsage("session-1", ["orient-wait.md"]));
	});
});

test("does not follow a symbolic-link usage root", async () => {
	await withUsageDirectory(async (directory) => {
		const target = join(directory, "target");
		await mkdir(target);
		await symlink(target, join(directory, "agent"));

		await recordSnippetUsage("session-1", ["orient-wait.md"]);

		await assert.rejects(access(join(target, "prompt-snippets-usage.jsonl")), { code: "ENOENT" });
	});
});

test("does not follow a symbolic-link usage file", async () => {
	await withUsageDirectory(async (directory) => {
		const root = join(directory, "agent");
		const target = join(directory, "target.jsonl");
		await mkdir(root);
		await writeFile(target, "unchanged\n");
		await symlink(target, join(root, "prompt-snippets-usage.jsonl"));

		await recordSnippetUsage("session-1", ["orient-wait.md"]);

		assert.equal(await readFile(target, "utf8"), "unchanged\n");
	});
});

test("does not follow a symbolic-link ancestor", async () => {
	await withUsageDirectory(async (directory) => {
		const target = join(directory, "target");
		await mkdir(target);
		await symlink(target, join(directory, "linked"));
		process.env.PI_CODING_AGENT_DIR = join(directory, "linked", "agent");

		await recordSnippetUsage("session-1", ["orient-wait.md"]);

		await assert.rejects(access(join(target, "agent", "prompt-snippets-usage.jsonl")), { code: "ENOENT" });
	});
});
