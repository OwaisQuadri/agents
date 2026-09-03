import assert from "node:assert/strict";
import { test } from "node:test";

import { buildKickoffPrompt, JUDGE_SYSTEM } from "./prompt.ts";

test("JUDGE_SYSTEM names the tool and the docstring-position rule", () => {
	assert.ok(JUDGE_SYSTEM.includes("submit_verdict"));
	assert.ok(JUDGE_SYSTEM.toLowerCase().includes("docstring"));
	assert.ok(JUDGE_SYSTEM.toLowerCase().includes("none"));
});

test("buildKickoffPrompt embeds the comment, the whitelist, and following context", () => {
	const prompt = buildKickoffPrompt({
		commentText: "// invariant: x never exceeds y",
		followingContext: "pub fn check(x: u32) -> bool {",
		whitelistDocText: "- inexpressible concept or architecture\n- TODO",
	});
	assert.ok(prompt.includes("// invariant: x never exceeds y"));
	assert.ok(prompt.includes("inexpressible concept or architecture"));
	assert.ok(prompt.includes("pub fn check(x: u32) -> bool {"));
});

test("buildKickoffPrompt flags an unverifiable position claim when no following context exists", () => {
	const prompt = buildKickoffPrompt({
		commentText: "/// docstring at end of file",
		followingContext: undefined,
		whitelistDocText: "- docstring on a public API declaration",
	});
	assert.ok(prompt.toLowerCase().includes("unverifiable"));
	assert.ok(!prompt.includes("```\nundefined\n```"));
});
