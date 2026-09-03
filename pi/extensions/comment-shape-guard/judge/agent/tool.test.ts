import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, test } from "node:test";

import { loadExtensionModule } from "../../test-support.ts";

let dir: string;
let tool: any;
let dispose: () => Promise<void>;
let resultPath: string;

beforeEach(async () => {
	dir = mkdtempSync(join(tmpdir(), "comment-shape-guard-judge-tool-"));
	resultPath = join(dir, "r.result.json");
	const loaded = await loadExtensionModule("judge/agent/tool.ts");
	dispose = loaded.dispose;
	const fakePi = { registerTool: (def: any) => (tool = def) };
	loaded.module.registerVerdictTool(fakePi, resultPath);
});

afterEach(async () => {
	rmSync(dir, { recursive: true, force: true });
	await dispose();
});

function readResult(): { shape: string; reason: string } {
	return JSON.parse(readFileSync(resultPath, "utf-8"));
}

test("registers exactly one tool named submit_verdict", () => {
	assert.equal(tool.name, "submit_verdict");
});

test("execute writes the trimmed verdict to the result file", async () => {
	await tool.execute("1", { shape: "  TODO  ", reason: "  explicit deferred follow-up  " });
	assert.deepEqual(readResult(), { shape: "TODO", reason: "explicit deferred follow-up" });
});

test("execute writes a none verdict the same way as an approved shape", async () => {
	await tool.execute("1", { shape: "none", reason: "no shape fits" });
	assert.deepEqual(readResult(), { shape: "none", reason: "no shape fits" });
});
