import assert from "node:assert/strict";
import { test } from "node:test";
import { isProtectedConfigPath, protectedConfigRoots } from "./config-write-guard/paths.ts";
import { blockedConfigToolCall } from "./config-write-guard/policy.ts";

const home = "/tmp/config-write-guard-home";

test("protects only managed agent destinations", () => {
	assert.deepEqual(protectedConfigRoots(home), [
		"/tmp/config-write-guard-home/.agents/skills",
		"/tmp/config-write-guard-home/.claude/AGENTS.md",
		"/tmp/config-write-guard-home/.claude/agents",
		"/tmp/config-write-guard-home/.claude/rules",
		"/tmp/config-write-guard-home/.claude/skills",
		"/tmp/config-write-guard-home/.codex/AGENTS.md",
		"/tmp/config-write-guard-home/.codex/skills",
		"/tmp/config-write-guard-home/.config/herdr/config.toml",
		"/tmp/config-write-guard-home/.pi/agent/agents",
		"/tmp/config-write-guard-home/.pi/agent/extensions",
		"/tmp/config-write-guard-home/.pi/agent/settings.json",
	]);
});

test("blocks managed files and descendants without blocking siblings", () => {
	assert.equal(isProtectedConfigPath(`${home}/.pi/agent/extensions/custom-header.ts`, home), true);
	assert.equal(isProtectedConfigPath(`${home}/.pi/agent/extensions/../extensions/custom-header.ts`, home), true);
	assert.equal(isProtectedConfigPath(`${home}/.agents/skills/session-stats/SKILL.md`, home), true);
	assert.equal(isProtectedConfigPath(`${home}/.claude/AGENTS.md`, home), true);
	assert.equal(isProtectedConfigPath(`${home}/.codex/AGENTS.md`, home), true);
	assert.equal(isProtectedConfigPath(`${home}/.config/herdr/config.toml`, home), true);
	assert.equal(isProtectedConfigPath(`${home}/.config/herdr/session.json`, home), false);
	assert.equal(isProtectedConfigPath(`${home}/.pi/agent/sessions/session.jsonl`, home), false);
	assert.equal(isProtectedConfigPath(`${home}/.pi/agent/settings.json.backup`, home), false);
	assert.equal(isProtectedConfigPath(`${home}/.pi/agent/extensions-copy/file.ts`, home), false);
});

test("blocks managed file writes and destination shell commands", () => {
	assert.match(blockedConfigToolCall("write", { path: `${home}/.pi/agent/extensions/custom-header.ts` }, home) ?? "", /Blocked/);
	assert.match(blockedConfigToolCall("edit", { path: `${home}/.claude/AGENTS.md` }, home) ?? "", /Blocked/);
	assert.equal(blockedConfigToolCall("write", { path: `${home}/.pi/agent/sessions/session.jsonl` }, home), undefined);
	assert.match(blockedConfigToolCall("bash", { command: "printf x > ~/.pi/agent/settings.json" }, home) ?? "", /Blocked/);
	assert.match(blockedConfigToolCall("bash", { command: "printf x > $HOME/.agents/skills/new/SKILL.md" }, home) ?? "", /Blocked/);
	assert.match(blockedConfigToolCall("bash", { command: `printf x > ${home}/.codex/AGENTS.md` }, home) ?? "", /Blocked/);
	assert.match(blockedConfigToolCall("bash", { command: `printf x > ${home}/.config/herdr/config.toml` }, home) ?? "", /Blocked/);
	assert.equal(blockedConfigToolCall("bash", { command: "git status" }, home), undefined);
});
