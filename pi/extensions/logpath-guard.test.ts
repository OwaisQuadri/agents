import assert from "node:assert/strict";
import { test } from "node:test";

import { extractLeadingCd, extractLogpathRedirectTargets, expandHome } from "./logpath-guard/extract.ts";
import { blockedLogpathWrite, type ValidateResult } from "./logpath-guard/policy.ts";

test("extractLogpathRedirectTargets finds >> and > redirects ending in logs/usage.jsonl", () => {
	assert.deepEqual(extractLogpathRedirectTargets('jq -cn \'{}\' >> skills/foo/logs/usage.jsonl'), [
		"skills/foo/logs/usage.jsonl",
	]);
	assert.deepEqual(extractLogpathRedirectTargets('printf x > "agents/skills/foo/logs/usage.jsonl"'), [
		"agents/skills/foo/logs/usage.jsonl",
	]);
	assert.deepEqual(extractLogpathRedirectTargets("git status"), []);
	assert.deepEqual(extractLogpathRedirectTargets("cat notes.txt >> other/file.txt"), []);
});

test("extractLeadingCd extracts a cd <dir> && prefix", () => {
	assert.equal(extractLeadingCd("cd ~/Documents/agents && mkdir -p skills/foo/logs"), "~/Documents/agents");
	assert.equal(extractLeadingCd('cd "/tmp/repo" && ls'), "/tmp/repo");
	assert.equal(extractLeadingCd("git status"), undefined);
});

test("expandHome replaces a leading ~, $HOME, or ${HOME}", () => {
	assert.equal(expandHome("~/Documents/agents", "/Users/x"), "/Users/x/Documents/agents");
	assert.equal(expandHome("$HOME/agents", "/Users/x"), "/Users/x/agents");
	assert.equal(expandHome("${HOME}/agents", "/Users/x"), "/Users/x/agents");
	assert.equal(expandHome("skills/foo", "/Users/x"), "skills/foo");
});

const ctx = { cwd: "/repo/agents", repoRoot: "/repo", home: "/Users/x" };

test("blockedLogpathWrite allows a command with no logpath redirect, without calling validate", () => {
	let called = false;
	const validate = (): ValidateResult => {
		called = true;
		return { ok: true };
	};
	const reason = blockedLogpathWrite({ command: "git status" }, ctx, validate);
	assert.equal(reason, undefined);
	assert.equal(called, false);
});

test("blockedLogpathWrite allows a redirect that validates ok", () => {
	const validate = (): ValidateResult => ({ ok: true });
	const reason = blockedLogpathWrite({ command: "jq -cn '{}' >> skills/foo/logs/usage.jsonl" }, ctx, validate);
	assert.equal(reason, undefined);
});

test("blockedLogpathWrite blocks a redirect that fails validation and names the resolved path", () => {
	const seen: string[] = [];
	const validate = (repoRoot: string, absoluteTarget: string): ValidateResult => {
		seen.push(absoluteTarget);
		return { ok: false, reason: "does not match <skills|agents|workflows>/<name>/logs/usage.jsonl" };
	};
	// cwd is /repo/agents, relative target resolves against it (the original incident's shape)
	const reason = blockedLogpathWrite({ command: "jq -cn '{}' >> skills/foo/logs/usage.jsonl" }, ctx, validate);
	assert.match(reason ?? "", /Blocked a write to \/repo\/agents\/skills\/foo\/logs\/usage\.jsonl/);
	assert.deepEqual(seen, ["/repo/agents/skills/foo/logs/usage.jsonl"]);
});

test("blockedLogpathWrite resolves a leading cd prefix before joining the relative target", () => {
	const seen: string[] = [];
	const validate = (repoRoot: string, absoluteTarget: string): ValidateResult => {
		seen.push(absoluteTarget);
		return { ok: true };
	};
	blockedLogpathWrite(
		{ command: "cd ~/Documents/agents && mkdir -p skills/foo/logs && jq -cn '{}' >> skills/foo/logs/usage.jsonl" },
		{ cwd: "/repo/agents", repoRoot: "/repo", home: "/Users/x" },
		validate,
	);
	assert.deepEqual(seen, ["/Users/x/Documents/agents/skills/foo/logs/usage.jsonl"]);
});

test("blockedLogpathWrite resolves an already-absolute redirect target as-is", () => {
	const seen: string[] = [];
	const validate = (repoRoot: string, absoluteTarget: string): ValidateResult => {
		seen.push(absoluteTarget);
		return { ok: true };
	};
	blockedLogpathWrite({ command: ">> /repo/skills/foo/logs/usage.jsonl" }, ctx, validate);
	assert.deepEqual(seen, ["/repo/skills/foo/logs/usage.jsonl"]);
});
