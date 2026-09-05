import assert from "node:assert/strict";
import { test } from "node:test";
import { isProtectedConfigPath, protectedConfigRoots } from "./config-write-guard/paths.ts";
import { blockedConfigToolCall } from "./config-write-guard/policy.ts";

const home = "/tmp/config-write-guard-home";
const user = "config-write-guard-user";

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
		"/tmp/config-write-guard-home/.config/simslim",
		"/tmp/config-write-guard-home/.pi/agent/agents",
		"/tmp/config-write-guard-home/.pi/agent/extensions",
		"/tmp/config-write-guard-home/.pi/agent/keybindings.json",
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
	assert.equal(isProtectedConfigPath(`${home}/.config/simslim/main.json`, home), true);
	assert.equal(isProtectedConfigPath(`${home}/.config/simslim/feature.json`, home), true);
	assert.equal(isProtectedConfigPath(`${home}/.config/herdr/session.json`, home), false);
	assert.equal(isProtectedConfigPath(`${home}/.pi/agent/sessions/session.jsonl`, home), false);
	assert.equal(isProtectedConfigPath(`${home}/.pi/agent/settings.json.backup`, home), false);
	assert.equal(isProtectedConfigPath(`${home}/.pi/agent/extensions-copy/file.ts`, home), false);
	assert.equal(isProtectedConfigPath(`${home}/.pi/agent/keybindings.json`, home), true);
	assert.equal(isProtectedConfigPath(`${home}/.pi/agent/keybindings.json.backup`, home), false);
});

test("blocks managed file writes and destination shell commands", () => {
	assert.match(blockedConfigToolCall("write", { path: `${home}/.pi/agent/extensions/custom-header.ts` }, home) ?? "", /Blocked/);
	assert.match(blockedConfigToolCall("edit", { path: `${home}/.claude/AGENTS.md` }, home) ?? "", /Blocked/);
	assert.equal(blockedConfigToolCall("write", { path: `${home}/.pi/agent/sessions/session.jsonl` }, home), undefined);
	assert.match(blockedConfigToolCall("bash", { command: "printf x > ~/.pi/agent/settings.json" }, home) ?? "", /Blocked/);
	assert.match(blockedConfigToolCall("bash", { command: "printf x > $HOME/.agents/skills/new/SKILL.md" }, home) ?? "", /Blocked/);
	assert.match(blockedConfigToolCall("bash", { command: `printf x > ${home}/.codex/AGENTS.md` }, home) ?? "", /Blocked/);
	assert.match(blockedConfigToolCall("bash", { command: `printf x > ${home}/.config/herdr/config.toml` }, home) ?? "", /Blocked/);
	assert.match(blockedConfigToolCall("bash", { command: `printf x > ${home}/.config/simslim/main.json` }, home) ?? "", /Blocked/);
	assert.match(blockedConfigToolCall("bash", { command: `printf x > ${home}/.config/simslim/main.json && echo done` }, home) ?? "", /Blocked/);
	assert.match(blockedConfigToolCall("bash", { command: `tee ${home}/.config/simslim/main.json < feature.json` }, home) ?? "", /Blocked/);
	assert.match(blockedConfigToolCall("bash", { command: `rm -rf ${home}/.config/simslim` }, home) ?? "", /Blocked/);
	assert.match(blockedConfigToolCall("bash", { command: `printf x > ${home}/.config/simslim//main.json` }, home) ?? "", /Blocked/);
	assert.match(blockedConfigToolCall("bash", { command: `simslim profile ${home}/.config/simslim/./main.json` }, home) ?? "", /Blocked/);
	assert.equal(blockedConfigToolCall("bash", { command: `simslim verify ABC --profile ${home}/.config/simslim/main.json` }, home), undefined);
	assert.equal(blockedConfigToolCall("bash", { command: `simslim on ABC --profile ${home}/.config/simslim/main.json` }, home), undefined);
	assert.equal(blockedConfigToolCall("bash", { command: `simslim --set testing on ABC --profile ${home}/.config/simslim/main.json` }, home), undefined);
	assert.equal(blockedConfigToolCall("bash", { command: `simslim --boot-timeout=15m verify ABC --profile ${home}/.config/simslim/main.json` }, home), undefined);
	assert.match(blockedConfigToolCall("bash", { command: `./simslim on ABC --profile ${home}/.config/simslim/main.json` }, home) ?? "", /Blocked/);
	assert.match(blockedConfigToolCall("bash", { command: `printf x > ${home}/.config/simslim/main.json.backup` }, home) ?? "", /Blocked/);
	assert.match(blockedConfigToolCall("bash", { command: `printf x > ${home}/.config/simslim/feature.json` }, home) ?? "", /Blocked/);
	assert.match(blockedConfigToolCall("bash", { command: `rm -rf ${home}/.config/simslim/` }, home) ?? "", /Blocked/);
	assert.match(blockedConfigToolCall("bash", { command: `rm -rf ${home}/.config/simslim/*` }, home) ?? "", /Blocked/);
	assert.equal(blockedConfigToolCall("bash", { command: `printf x > ${home}/.config/simslim-copy/main.json` }, home), undefined);
	assert.equal(blockedConfigToolCall("bash", { command: "git status" }, home), undefined);
});

test("allows read-only bash access to managed config", () => {
	assert.equal(blockedConfigToolCall("bash", { command: `cat ${home}/.pi/agent/settings.json` }, home), undefined);
	assert.equal(blockedConfigToolCall("bash", { command: `less ${home}/.claude/AGENTS.md` }, home), undefined);
	assert.equal(blockedConfigToolCall("bash", { command: `ls -la ${home}/.pi/agent/extensions` }, home), undefined);
	assert.equal(blockedConfigToolCall("bash", { command: `cat ${home}/.pi/agent/settings.json | less` }, home), undefined);
	assert.equal(blockedConfigToolCall("bash", { command: `git -C ${home}/.claude diff` }, home), undefined);
});

test("allows read-only navigation and search commands", () => {
	assert.equal(blockedConfigToolCall("bash", { command: `fd -t f . ${home}/.pi/agent/sessions` }, home), undefined);
	assert.equal(blockedConfigToolCall("bash", { command: `cd ${home}/.pi/agent/sessions && ls -la` }, home), undefined);
	assert.equal(blockedConfigToolCall("bash", { command: `rg foo ${home}/.agents/skills` }, home), undefined);
	assert.equal(blockedConfigToolCall("bash", { command: `rg foo ${home}/.pi/agent/extensions | head -1` }, home), undefined);
});

test("blocks grep against managed config after its allowlist removal", () => {
	assert.match(blockedConfigToolCall("bash", { command: `grep -r foo ${home}/.agents/skills` }, home) ?? "", /Blocked/);
	assert.match(blockedConfigToolCall("bash", { command: `grep foo ${home}/.claude/AGENTS.md | head -1` }, home) ?? "", /Blocked/);
});

test("allows egrep and fgrep, which the grep removal deliberately left in place", () => {
	assert.equal(blockedConfigToolCall("bash", { command: `egrep foo ${home}/.claude/AGENTS.md` }, home), undefined);
	assert.equal(blockedConfigToolCall("bash", { command: `fgrep foo ${home}/.claude/AGENTS.md` }, home), undefined);
});

test("leaves a pipe stage naming no protected path out of the judgment", () => {
	assert.equal(blockedConfigToolCall("bash", { command: `ls -la ${home}/.pi/agent/extensions | grep foo` }, home), undefined);
});

test("allows a write whose group carries no protected path, after cd joined the allowlist", () => {
	assert.equal(blockedConfigToolCall("bash", { command: `cd ${home}/.claude && rm AGENTS.md` }, home), undefined);
});

test("blocks direct write and delete commands touching managed config", () => {
	assert.match(blockedConfigToolCall("bash", { command: `rm ${home}/.pi/agent/settings.json` }, home) ?? "", /Blocked/);
	assert.match(blockedConfigToolCall("bash", { command: `sed -i s/a/b/ ${home}/.claude/AGENTS.md` }, home) ?? "", /Blocked/);
	assert.match(blockedConfigToolCall("bash", { command: `tee ${home}/.pi/agent/settings.json` }, home) ?? "", /Blocked/);
	assert.match(blockedConfigToolCall("bash", { command: `git -C ${home}/.claude checkout -- AGENTS.md` }, home) ?? "", /Blocked/);
	assert.match(blockedConfigToolCall("bash", { command: `echo $(rm ${home}/.pi/agent/settings.json)` }, home) ?? "", /Blocked/);
	assert.match(blockedConfigToolCall("bash", { command: `cat file.txt >> ${home}/.pi/agent/settings.json` }, home) ?? "", /Blocked/);
	assert.match(blockedConfigToolCall("bash", { command: `python3 -c "open('${home}/.pi/agent/settings.json','w')"` }, home) ?? "", /Blocked/);
});

test("blocks a background job from smuggling a write past the leading command", () => {
	assert.match(blockedConfigToolCall("bash", { command: `echo hi & sed -i s/a/b/ ${home}/.claude/AGENTS.md` }, home) ?? "", /Blocked/);
	assert.match(blockedConfigToolCall("bash", { command: `true & rm ${home}/.pi/agent/settings.json &` }, home) ?? "", /Blocked/);
});

test("blocks a git subcommand outside the read allowlist", () => {
	assert.match(blockedConfigToolCall("bash", { command: `git -C ${home}/.claude pull` }, home) ?? "", /Blocked/);
	assert.match(blockedConfigToolCall("bash", { command: `git -C ${home}/.claude merge feature` }, home) ?? "", /Blocked/);
	assert.match(blockedConfigToolCall("bash", { command: `git -C ${home}/.claude commit -am x` }, home) ?? "", /Blocked/);
});

test("blocks a redirect with no space before the operator", () => {
	assert.match(blockedConfigToolCall("bash", { command: `cat malicious.md>${home}/.claude/AGENTS.md` }, home) ?? "", /Blocked/);
	assert.match(blockedConfigToolCall("bash", { command: `echo pwned>${home}/.pi/agent/settings.json` }, home) ?? "", /Blocked/);
});

test("blocks a protected-path reference laundered through an interpreter pipe", () => {
	assert.match(blockedConfigToolCall("bash", { command: `echo "sed -i s/a/b/ ${home}/.claude/AGENTS.md" | bash` }, home) ?? "", /Blocked/);
	assert.match(blockedConfigToolCall("bash", { command: `printf '%s' "rm ${home}/.pi/agent/settings.json" | sh` }, home) ?? "", /Blocked/);
});

test("blocks git reflog's destructive actions but allows reflog show", () => {
	assert.match(blockedConfigToolCall("bash", { command: `git -C ${home}/.claude reflog expire --expire=now --all` }, home) ?? "", /Blocked/);
	assert.match(blockedConfigToolCall("bash", { command: `git -C ${home}/.claude reflog delete HEAD@{0}` }, home) ?? "", /Blocked/);
	assert.equal(blockedConfigToolCall("bash", { command: `git -C ${home}/.claude reflog show` }, home), undefined);
});

test("recognizes ~<username> as the protected home, not ~<other user>", () => {
	assert.match(
		blockedConfigToolCall("bash", { command: `sed -i s/a/b/ ~${user}/.claude/AGENTS.md` }, home, user) ?? "",
		/Blocked/,
	);
	assert.equal(blockedConfigToolCall("bash", { command: "sed -i s/a/b/ ~other/.claude/AGENTS.md" }, home, user), undefined);
});

test("recognizes a doubled slash as the same protected path", () => {
	assert.match(
		blockedConfigToolCall("bash", { command: `sed -i s/a/b/ ${home}//.claude/AGENTS.md` }, home, user) ?? "",
		/Blocked/,
	);
});

test("recognizes |& as a pipe into an interpreter, not a background job", () => {
	assert.match(
		blockedConfigToolCall("bash", { command: `echo "sed -i s/a/b/ ${home}/.claude/AGENTS.md" |& bash` }, home, user) ?? "",
		/Blocked/,
	);
});

test("recognizes a mixed-case interpreter name", () => {
	assert.match(
		blockedConfigToolCall("bash", { command: `echo "sed -i s/a/b/ ${home}/.claude/AGENTS.md" | BASH` }, home, user) ?? "",
		/Blocked/,
	);
});
