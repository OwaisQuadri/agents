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

test("allows read-only shell access to managed config, blocks anything that could mutate it", () => {
	assert.equal(blockedConfigToolCall("bash", { command: `cat ${home}/.pi/agent/settings.json` }, home), undefined);
	assert.equal(blockedConfigToolCall("bash", { command: `less ${home}/.claude/AGENTS.md` }, home), undefined);
	assert.equal(blockedConfigToolCall("bash", { command: `grep -r foo ${home}/.agents/skills` }, home), undefined);
	assert.equal(blockedConfigToolCall("bash", { command: `ls -la ${home}/.pi/agent/extensions` }, home), undefined);
	assert.equal(blockedConfigToolCall("bash", { command: `cat ${home}/.pi/agent/settings.json | less` }, home), undefined);
	assert.equal(blockedConfigToolCall("bash", { command: `git -C ${home}/.claude diff` }, home), undefined);

	assert.match(blockedConfigToolCall("bash", { command: `rm ${home}/.pi/agent/settings.json` }, home) ?? "", /Blocked/);
	assert.match(blockedConfigToolCall("bash", { command: `sed -i s/a/b/ ${home}/.claude/AGENTS.md` }, home) ?? "", /Blocked/);
	assert.match(blockedConfigToolCall("bash", { command: `tee ${home}/.pi/agent/settings.json` }, home) ?? "", /Blocked/);
	assert.match(blockedConfigToolCall("bash", { command: `git -C ${home}/.claude checkout -- AGENTS.md` }, home) ?? "", /Blocked/);
	assert.match(blockedConfigToolCall("bash", { command: `echo $(rm ${home}/.pi/agent/settings.json)` }, home) ?? "", /Blocked/);
	assert.match(blockedConfigToolCall("bash", { command: `cat file.txt >> ${home}/.pi/agent/settings.json` }, home) ?? "", /Blocked/);
	assert.match(blockedConfigToolCall("bash", { command: `python3 -c "open('${home}/.pi/agent/settings.json','w')"` }, home) ?? "", /Blocked/);

	// A background job (`&`) must not smuggle a write past the leading segment's verdict.
	assert.match(blockedConfigToolCall("bash", { command: `echo hi & sed -i s/a/b/ ${home}/.claude/AGENTS.md` }, home) ?? "", /Blocked/);
	assert.match(blockedConfigToolCall("bash", { command: `true & rm ${home}/.pi/agent/settings.json &` }, home) ?? "", /Blocked/);

	// git subcommands outside the read allowlist default to write, including ones that
	// mutate the working tree without appearing destructive by name.
	assert.match(blockedConfigToolCall("bash", { command: `git -C ${home}/.claude pull` }, home) ?? "", /Blocked/);
	assert.match(blockedConfigToolCall("bash", { command: `git -C ${home}/.claude merge feature` }, home) ?? "", /Blocked/);
	assert.match(blockedConfigToolCall("bash", { command: `git -C ${home}/.claude commit -am x` }, home) ?? "", /Blocked/);

	// A redirect with no space before the operator must still be caught.
	assert.match(blockedConfigToolCall("bash", { command: `cat malicious.md>${home}/.claude/AGENTS.md` }, home) ?? "", /Blocked/);
	assert.match(blockedConfigToolCall("bash", { command: `echo pwned>${home}/.pi/agent/settings.json` }, home) ?? "", /Blocked/);

	// A protected-path reference must not be laundered through a read-only leading command
	// piped into an interpreter that can act on it sight unseen.
	assert.match(blockedConfigToolCall("bash", { command: `echo "sed -i s/a/b/ ${home}/.claude/AGENTS.md" | bash` }, home) ?? "", /Blocked/);
	assert.match(blockedConfigToolCall("bash", { command: `printf '%s' "rm ${home}/.pi/agent/settings.json" | sh` }, home) ?? "", /Blocked/);

	// `git reflog expire`/`reflog delete` discard history; the bare `reflog` subcommand
	// (allowlisted for `reflog`/`reflog show`) must not cover its destructive actions.
	assert.match(blockedConfigToolCall("bash", { command: `git -C ${home}/.claude reflog expire --expire=now --all` }, home) ?? "", /Blocked/);
	assert.match(blockedConfigToolCall("bash", { command: `git -C ${home}/.claude reflog delete HEAD@{0}` }, home) ?? "", /Blocked/);
	assert.equal(blockedConfigToolCall("bash", { command: `git -C ${home}/.claude reflog show` }, home), undefined);
});

test("catches tilde-username paths, doubled slashes, |& pipes, and mixed-case interpreter names", () => {
	const user = "config-write-guard-user";
	const homeWithUser = `${home}`;

	// `~<currentUsername>/...` expands to the same directory as `~/...`.
	assert.match(
		blockedConfigToolCall("bash", { command: `sed -i s/a/b/ ~${user}/.claude/AGENTS.md` }, homeWithUser, user) ?? "",
		/Blocked/,
	);
	// A different username's `~other/...` is not this machine's protected home.
	assert.equal(blockedConfigToolCall("bash", { command: "sed -i s/a/b/ ~other/.claude/AGENTS.md" }, homeWithUser, user), undefined);

	// A doubled `/` between the home token and the dot-directory is still the same path.
	assert.match(
		blockedConfigToolCall("bash", { command: `sed -i s/a/b/ ${home}//.claude/AGENTS.md` }, home, user) ?? "",
		/Blocked/,
	);

	// `|&` (2>&1 |) still pipes a protected-path reference into an interpreter.
	assert.match(
		blockedConfigToolCall("bash", { command: `echo "sed -i s/a/b/ ${home}/.claude/AGENTS.md" |& bash` }, home, user) ?? "",
		/Blocked/,
	);

	// A mixed-case interpreter name resolves to the same binary via PATH lookup.
	assert.match(
		blockedConfigToolCall("bash", { command: `echo "sed -i s/a/b/ ${home}/.claude/AGENTS.md" | BASH` }, home, user) ?? "",
		/Blocked/,
	);
});
