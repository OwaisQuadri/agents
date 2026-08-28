import assert from "node:assert/strict";
import { test } from "node:test";

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

test("blockedPreferredCliCommand passes the exact command string through to the checker", () => {
	const seen: string[] = [];
	const check = (command: string): CheckResult => {
		seen.push(command);
		return { blocked: false };
	};
	blockedPreferredCliCommand({ command: "ps aux | grep foo" }, check);
	assert.deepEqual(seen, ["ps aux | grep foo"]);
});

test("blockedPreferredCliCommand still blocks on an empty reason string — callers must check !== undefined, not truthiness", () => {
	const check = (): CheckResult => ({ blocked: true, reason: "" });
	const reason = blockedPreferredCliCommand({ command: "find ." }, check);
	assert.equal(reason, "");
	assert.notEqual(reason, undefined);
});
