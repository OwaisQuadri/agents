import assert from "node:assert/strict";
import { test } from "node:test";

import { preferredCliGuardArguments } from "./preferred-cli-guard/arguments.ts";
import { blockedPreferredCliCommand, type CheckResult } from "./preferred-cli-guard/policy.ts";

test("blockedPreferredCliCommand allows a command the checker allows, without inventing a reason", () => {
	const check = (): CheckResult => ({ blocked: false });
	const reason = blockedPreferredCliCommand({ command: "git grep pattern" }, check);
	assert.equal(reason, undefined);
});

test("blockedPreferredCliCommand surfaces the checker's block reason verbatim", () => {
	const check = (): CheckResult => ({ blocked: true, reason: "Blocked `find` — use `fd` instead." });
	const reason = blockedPreferredCliCommand({ command: "find . -name '*.rs'" }, check);
	assert.equal(reason, "Blocked `find` — use `fd` instead.");
});

test("blockedPreferredCliCommand passes the exact command and optional timeout through to the checker", () => {
	const seen: Array<[string, number | undefined]> = [];
	const check = (command: string, timeout?: number): CheckResult => {
		seen.push([command, timeout]);
		return { blocked: false };
	};
	blockedPreferredCliCommand({ command: "git status" }, check);
	blockedPreferredCliCommand({ command: "./skills/tool-author/evals/run.sh", timeout: 7200 }, check);
	assert.deepEqual(seen, [
		["git status", undefined],
		["./skills/tool-author/evals/run.sh", 7200],
	]);
});

test("preferredCliGuardArguments keeps the repository root when timeout is absent", () => {
	assert.deepEqual(preferredCliGuardArguments("git status", undefined, "/repo"), [
		"--check",
		"git status",
		"--repository-root",
		"/repo",
	]);
	assert.deepEqual(preferredCliGuardArguments("echo ok", 30, "/repo"), [
		"--check",
		"echo ok",
		"--timeout",
		"30",
		"--repository-root",
		"/repo",
	]);
});

test("blockedPreferredCliCommand propagates a timeout block reason verbatim", () => {
	const reason = blockedPreferredCliCommand(
		{ command: "tools/skill-eval/run.sh", timeout: 7200 },
		() => ({ blocked: true, reason: "Run the full harness without an outer timeout." }),
	);
	assert.equal(reason, "Run the full harness without an outer timeout.");
});

test("blockedPreferredCliCommand still blocks on an empty reason string — callers must check !== undefined, not truthiness", () => {
	const check = (): CheckResult => ({ blocked: true, reason: "" });
	const reason = blockedPreferredCliCommand({ command: "find ." }, check);
	assert.equal(reason, "");
	assert.notEqual(reason, undefined);
});
