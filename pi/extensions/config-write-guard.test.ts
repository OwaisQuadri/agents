import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, readFileSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, relative } from "node:path";
import { test } from "node:test";
import { classifyCheckoutCommand } from "./config-write-guard/bash-intent.ts";
import { isPathInsideRoot, isProtectedConfigPath, protectedConfigRoots } from "./config-write-guard/paths.ts";
import { blockedConfigToolCall, type GuardContext } from "./config-write-guard/policy.ts";

const home = "/tmp/config-write-guard-home";
const user = "config-write-guard-user";
const repositoryRoot = "/tmp/agents-main";
const worktreeRoot = "/tmp/agents-worktree";

function guard(cwd = repositoryRoot, isRepositoryClean = true, worktreeRoots = [repositoryRoot, worktreeRoot]): GuardContext {
	return { cwd, repositoryRoot, isRepositoryClean: () => isRepositoryClean, worktreeRoots: () => worktreeRoots };
}

for (const isWrapped of [false, true]) {
	for (const form of ["direct", "sh", "pwd-sh"]) {
		test(`allows the pinned installed selector: ${form}, wrapped=${isWrapped}`, () => {
			withSelectorFixture(({ script, worktree, home: linkedHome, context }) => {
				const payload = form === "pwd-sh" ? `pwd && sh ${script}`
					: `cd ${worktree} && ${form === "sh" ? "sh " : ""}${script}`;
				const command = isWrapped ? `/bin/zsh -lc '${payload}'` : payload;
				for (const currentContext of [undefined, context, { ...context, cwd: context.repositoryRoot }]) {
					assert.equal(blockedConfigToolCall("bash", { command }, linkedHome, user, currentContext), undefined, command);
				}
			});
		});
	}
}

function withSelectorFixture(run: (fixture: {
	script: string; source: string; worktree: string; home: string; context: GuardContext;
}) => void): void {
	const fixture = mkdtempSync(join(tmpdir(), `config-write-guard-selector-${process.pid}-`));
	const root = join(fixture, "primary");
	const worktree = join(fixture, "other-worktree");
	const linkedHome = join(fixture, "home");
	const source = join(root, "skills/task-graph/scripts/next-issue.sh");
	const script = join(linkedHome, ".agents/skills/task-graph/scripts/next-issue.sh");
	try {
		mkdirSync(join(root, "skills/task-graph/scripts"), { recursive: true });
		mkdirSync(join(linkedHome, ".agents/skills"), { recursive: true });
		mkdirSync(worktree);
		writeFileSync(source, readFileSync(new URL("../../skills/task-graph/scripts/next-issue.sh", import.meta.url)), { mode: 0o755 });
		symlinkSync(join(root, "skills/task-graph"), join(linkedHome, ".agents/skills/task-graph"));
		run({ script, source, worktree, home: linkedHome, context: {
			cwd: worktree, repositoryRoot: root, isRepositoryClean: () => true, worktreeRoots: () => [root],
		} });
	} finally {
		rmSync(fixture, { recursive: true, force: true });
	}
}

for (const isWrapped of [false, true]) {
	test(`allows the installed selector without leaving the primary checkout, wrapped=${isWrapped}`, () => {
		withSelectorFixture(({ script, home: linkedHome, context }) => {
			for (const payload of [script, `sh ${script}`, `/bin/sh ${script}`]) {
				const command = isWrapped ? `/bin/zsh -lc '${payload}'` : payload;
				assert.equal(blockedConfigToolCall("bash", { command }, linkedHome, user, { ...context, cwd: context.repositoryRoot }), undefined, command);
			}
		});
	});

	test(`denies selector exceptions for external aliases and primary source paths, wrapped=${isWrapped}`, () => {
		withSelectorFixture(({ script, source, worktree, home: linkedHome, context }) => {
			const alias = join(worktree, "next-issue.sh");
			symlinkSync(script, alias);
			for (const path of [alias, source, script.replace("/scripts/", "/scripts/./")]) {
				for (const payload of [path, `sh ${path}`, `/bin/sh ${path}`]) {
					const command = isWrapped ? `/bin/zsh -lc '${payload}'` : payload;
					assert.match(blockedConfigToolCall("bash", { command }, linkedHome, user, { ...context, cwd: context.repositoryRoot }) ?? "", /Blocked/, command);
				}
			}
		});
	});

	test(`selector exceptions preserve write and execution guards, wrapped=${isWrapped}`, () => {
		withSelectorFixture(({ script, source, home: linkedHome, context }) => {
			const neighbor = script.replace("next-issue.sh", "unknown.sh");
			const copy = join(linkedHome, ".agents/skills/next-issue.sh");
			writeFileSync(copy, readFileSync(source));
			for (const payload of [
				`${copy}`, `sh ${copy}`, `${neighbor}`, `sh ${neighbor}`,
				`${script} --help`, `sh ${script} extra`, `/bin/sh -e ${script}`,
				`${script} && sh ${neighbor}`, `sh ${script} && ${neighbor}`,
				`${script} > ${linkedHome}/.pi/agent/settings.json`,
				`sh ${script} > ${context.repositoryRoot}/output`,
				`${script} && touch ${context.repositoryRoot}/output`,
				`sh ${script} && touch ${linkedHome}/.pi/agent/settings.json`,
				`cd ${context.repositoryRoot} && sh ${script} && touch output`,
				`eval ${script}`, `command ${script}`, `. ${script}`,
				`echo $(${script})`, `echo \`sh ${script}\``,
				`${script} | sh`, `sh ${script} | sh`, `cat ${script} | sh`,
				`sh < ${script}`, `X=1 sh ${script}`, `sh ${script} $EXTRA`,
			]) {
				const command = isWrapped ? `/bin/zsh -lc '${payload}'` : payload;
				assert.match(blockedConfigToolCall("bash", { command }, linkedHome, user, context) ?? "", /Blocked/, command);
			}
			writeFileSync(source, `${readFileSync(source, "utf8")}\nprintf changed\n`);
			for (const payload of [script, `sh ${script}`]) {
				const command = isWrapped ? `/bin/zsh -lc '${payload}'` : payload;
				assert.match(blockedConfigToolCall("bash", { command }, linkedHome, user, context) ?? "", /Blocked/, command);
			}
		});
	});
}

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

test("allows a static Z shell wrapper reading managed config", () => {
	const command = `/bin/zsh -lc 'cat ${home}/.pi/agent/settings.json'`;
	assert.equal(blockedConfigToolCall("bash", { command }, home), undefined);
});

test("allows a static Z shell wrapper changing to managed config and listing files", () => {
	const command = `/bin/zsh -lc 'cd ${home}/.claude && ls'`;
	assert.equal(blockedConfigToolCall("bash", { command }, home), undefined);
});

for (const [name, command] of [
	["relative deletion after changing directory", `/bin/zsh -lc 'cd ${home}/.claude && rm -rf x'`],
	["relative redirect after changing directory", `/bin/zsh -lc 'cd ${home}/.claude; printf x > settings.json'`],
	["relative deletion in a pipeline", `/bin/zsh -lc 'cd ${home}/.claude | rm x'`],
	["interpreter after a managed-config read", `/bin/zsh -lc 'cat ${home}/.claude/settings.json && bash'`],
	["missing closing quote", `/bin/zsh -lc 'cat ${home}/.pi/agent/settings.json`],
	["dynamic double-quoted payload", `/bin/zsh -lc "cat ${home}/.pi/agent/settings.json $EXTRA"`],
	["output redirect", `/bin/zsh -lc 'printf x > ${home}/.pi/agent/settings.json'`],
	["adjacent output redirect", `/bin/zsh -lc 'printf x>${home}/.pi/agent/settings.json'`],
	["append redirect", `/bin/zsh -lc 'printf x>>${home}/.pi/agent/settings.json'`],
	["command substitution", `/bin/zsh -lc 'cat $(touch ${home}/.pi/agent/settings.json)'`],
	["backtick substitution", `/bin/zsh -lc 'cat \`touch ${home}/.pi/agent/settings.json\`'`],
	["interpreter pipeline", `/bin/zsh -lc 'cat ${home}/.pi/agent/settings.json | sh'`],
	["command execution", `/bin/zsh -lc 'command touch ${home}/.pi/agent/settings.json'`],
	["unsupported inventory loop", `/bin/zsh -lc 'for file in ${home}/.pi/agent/extensions/*(N); do printf "%s\\n" "\${file:t}"; done'`],
]) {
	test(`blocks a managed-config Z shell wrapper with ${name}`, () => {
		assert.match(blockedConfigToolCall("bash", { command }, home) ?? "", /Blocked/, command);
	});
}

test("allows command-name lookup of a managed config path without executing it", () => {
	assert.equal(blockedConfigToolCall("bash", { command: `command -v ${home}/.pi/agent/settings.json` }, home), undefined);
});

test("blocks command-name lookup output redirected to managed config", () => {
	assert.match(blockedConfigToolCall("bash", { command: `command -v tmux > ${home}/.pi/agent/settings.json` }, home) ?? "", /Blocked/);
});

test("blocks command-name lookup with substitution referencing managed config", () => {
	assert.match(blockedConfigToolCall("bash", { command: `command -v $(touch ${home}/.pi/agent/settings.json)` }, home) ?? "", /Blocked/);
});

test("blocks command touch targeting managed config", () => {
	assert.match(blockedConfigToolCall("bash", { command: `command touch ${home}/.pi/agent/settings.json` }, home) ?? "", /Blocked/);
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

test("blocks file tools in the primary checkout and allows a worktree", () => {
	assert.match(blockedConfigToolCall("edit", { path: `${repositoryRoot}/skills/x.md` }, home, user, guard()) ?? "", /worktree/);
	assert.match(blockedConfigToolCall("write", { path: "skills/x.md" }, home, user, guard()) ?? "", /worktree/);
	assert.equal(blockedConfigToolCall("write", { path: `${worktreeRoot}/skills/x.md` }, home, user, guard(worktreeRoot)), undefined);
});

test("canonical containment follows existing and partial symbolic-link paths", () => {
	const root = mkdtempSync(join(tmpdir(), "main-checkout-root-"));
	const outside = mkdtempSync(join(tmpdir(), "main-checkout-outside-"));
	try {
		mkdirSync(join(root, "skills"));
		writeFileSync(join(root, "skills", "existing.md"), "x\n");
		symlinkSync(join(root, "skills"), join(outside, "skills-link"));
		assert.equal(isPathInsideRoot(join(outside, "skills-link", "existing.md"), root), true);
		assert.equal(isPathInsideRoot(join(outside, "skills-link", "new", "file.md"), root), true);
		assert.equal(isPathInsideRoot(join(outside, "ordinary.md"), root), false);
	} finally {
		rmSync(root, { recursive: true, force: true });
		rmSync(outside, { recursive: true, force: true });
	}
});

test("a symbolic-link alias into managed config stays protected", () => {
	const linkedHome = mkdtempSync(join(tmpdir(), "config-write-guard-alias-home-"));
	const aliasRoot = mkdtempSync(join(tmpdir(), "config-write-guard-alias-root-"));
	try {
		mkdirSync(join(linkedHome, ".claude", "skills"), { recursive: true });
		symlinkSync(join(linkedHome, ".claude", "skills"), join(aliasRoot, "skills-link"));
		assert.equal(isProtectedConfigPath(join(aliasRoot, "skills-link", "new.md"), linkedHome), true);
	} finally {
		rmSync(linkedHome, { recursive: true, force: true });
		rmSync(aliasRoot, { recursive: true, force: true });
	}
});

test("managed config stays protected when its root is a symbolic link", () => {
	const linkedHome = mkdtempSync(join(tmpdir(), "config-write-guard-linked-home-"));
	const source = mkdtempSync(join(tmpdir(), "config-write-guard-linked-source-"));
	try {
		mkdirSync(join(linkedHome, ".agents"));
		mkdirSync(join(source, "agent-author"));
		writeFileSync(join(source, "agent-author", "SKILL.md"), "x\n");
		symlinkSync(source, join(linkedHome, ".agents", "skills"));
		assert.equal(isProtectedConfigPath(join(linkedHome, ".agents", "skills", "agent-author", "SKILL.md"), linkedHome), true);
	} finally {
		rmSync(linkedHome, { recursive: true, force: true });
		rmSync(source, { recursive: true, force: true });
	}
});

test("classifies checkout shell reads and static Z shell wrappers", () => {
	assert.equal(classifyCheckoutCommand("git status --short"), "read");
	assert.equal(classifyCheckoutCommand("rg guard pi/extensions | head -20"), "read");
	assert.equal(classifyCheckoutCommand("/bin/zsh -lc 'git status --short'"), "read");
	assert.equal(classifyCheckoutCommand("/bin/zsh -lc 'rg guard pi/extensions | head -20'"), "read");
	assert.equal(classifyCheckoutCommand("/bin/zsh -lc 'git status 2>/dev/null'"), "read");
});

test("classifies tmux command-name lookup as a checkout read", () => {
	assert.equal(classifyCheckoutCommand("command -v tmux"), "read");
});

test("classifies sandbox-exec command-name lookup as a checkout read", () => {
	assert.equal(classifyCheckoutCommand("command -v sandbox-exec"), "read");
});

test("classifies combined command-name lookups in a static Z shell wrapper as a checkout read", () => {
	const target = "/tmp/config-write-guard-probe/home/.pi/agent/extensions/live-diff";
	const command = `/bin/zsh -lc 'command -v sandbox-exec; command -v tmux; /usr/bin/readlink ${target}; /usr/bin/test -f ${target}/engine.ts && printf "footer dependency resolves\\n"'`;
	assert.equal(classifyCheckoutCommand(command), "read");
});

test("blocks checkout output redirection from command-name lookup", () => {
	for (const command of [
		"command -v tmux > leaked.md",
		"/bin/zsh -lc 'command -v sandbox-exec >> leaked.md'",
	]) {
		assert.equal(classifyCheckoutCommand(command), "write-or-unknown", command);
	}
});

test("blocks substitution in command-name lookup", () => {
	for (const command of [
		"command -v $(touch leaked.md)",
		"/bin/zsh -lc 'command -v `touch leaked.md`'",
	]) {
		assert.equal(classifyCheckoutCommand(command), "write-or-unknown", command);
	}
});

test("blocks checkout writes after command-name lookup", () => {
	for (const command of [
		"command -v tmux; touch leaked.md",
		"/bin/zsh -lc 'command -v sandbox-exec && touch leaked.md'",
	]) {
		assert.equal(classifyCheckoutCommand(command), "write-or-unknown", command);
	}
});

test("does not allow unrestricted command execution as a checkout read", () => {
	assert.equal(classifyCheckoutCommand("command touch leaked.md"), "write-or-unknown");
	assert.equal(classifyCheckoutCommand("/bin/zsh -lc 'command touch leaked.md'"), "write-or-unknown");
});

test("allows lookup and reads through a primary-checkout symbolic link but blocks writes", () => {
	const root = mkdtempSync(join(tmpdir(), "config-write-guard-lookup-main-"));
	const worktree = mkdtempSync(join(tmpdir(), "config-write-guard-lookup-worktree-"));
	try {
		writeFileSync(join(root, "engine.ts"), "x\n");
		const target = join(worktree, "source-link");
		symlinkSync(root, target);
		const context: GuardContext = {
			cwd: worktree,
			repositoryRoot: root,
			isRepositoryClean: () => true,
			worktreeRoots: () => [root, worktree],
		};
		assert.match(blockedConfigToolCall("bash", { command: `command touch ${target}/leaked.md` }, home, user, context) ?? "", /worktree/);
		const command = `/bin/zsh -lc 'command -v sandbox-exec; command -v tmux; /usr/bin/readlink ${target}; /usr/bin/test -f ${target}/engine.ts && printf "footer dependency resolves\\n"'`;
		assert.equal(blockedConfigToolCall("bash", { command }, home, user, context), undefined);
	} finally {
		rmSync(root, { recursive: true, force: true });
		rmSync(worktree, { recursive: true, force: true });
	}
});

for (const [operation, separator] of [
	["cmp", " "],
	["shasum -a 256", " "],
	["cmp", " > "],
	["shasum -a 256", " >> "],
	["cmp", ">"],
	["shasum -a 256", ">>"],
]) {
	for (const isWrapped of [false, true]) {
		const isRedirect = separator.includes(">");
		test(`${isRedirect ? "blocks" : "allows"} ${isWrapped ? "wrapped" : "bare"} ${operation} with separator ${JSON.stringify(separator)} and worktree plus primary-alias operands`, () => {
			const fixture = mkdtempSync(join(tmpdir(), `config-write-guard-comparison-${process.pid}-`));
			const root = join(fixture, "primary");
			const worktree = join(fixture, "worktree");
			const alias = join(fixture, "primary-link");
			try {
				mkdirSync(root);
				mkdirSync(worktree);
				writeFileSync(join(root, "engine.ts"), "x\n");
				writeFileSync(join(worktree, "engine.ts"), "x\n");
				symlinkSync(root, alias);
				const context: GuardContext = {
					cwd: worktree,
					repositoryRoot: root,
					isRepositoryClean: () => true,
					worktreeRoots: () => [root, worktree],
				};
				const payload = `${operation} ${worktree}/engine.ts${separator}${alias}/engine.ts`;
				const command = isWrapped ? `/bin/zsh -lc '${payload}'` : payload;
				if (isRedirect) {
					assert.match(blockedConfigToolCall("bash", { command }, home, user, context) ?? "", /worktree/, command);
				} else {
					assert.equal(blockedConfigToolCall("bash", { command }, home, user, context), undefined, command);
				}
			} finally {
				rmSync(fixture, { recursive: true, force: true });
			}
		});
	}
}

test("classifies checkout shell writes and uncertain wrappers", () => {
	for (const command of [
		"printf x > leaked.md",
		"git status && touch leaked.md",
		"python3 -c 'open(\"leaked.md\", \"w\")'",
		"cargo test",
		"rg --pre 'touch leaked.md' guard .",
		"git diff --output=leaked.patch",
		"git -c diff.external=./write.sh diff HEAD",
		"git --git-dir=/tmp/other status",
		"GIT_EXTERNAL_DIFF=./write.sh git diff HEAD",
		"sed 'w leaked.md' README.md",
		"find . -fls leaked.md",
		"/bin/zsh -lc 'touch leaked.md'",
		"/bin/zsh -lc \"git status $EXTRA\"",
		"/bin/zsh -lc 'git status",
	]) {
		assert.equal(classifyCheckoutCommand(command), "write-or-unknown", command);
	}
});

test("allows only the exact fast-forward pull form", () => {
	assert.equal(classifyCheckoutCommand("git pull --ff-only"), "clean-fast-forward-pull");
	assert.equal(classifyCheckoutCommand("/bin/zsh -lc 'git pull --ff-only'"), "clean-fast-forward-pull");
	for (const command of [
		"git pull",
		"git pull --rebase",
		"git pull --ff-only origin main",
		"git status && git pull --ff-only",
		"git pull --ff-only && git status",
	]) {
		assert.equal(classifyCheckoutCommand(command), "write-or-unknown", command);
	}
});

test("allows an exact pull only on a clean primary checkout", () => {
	assert.equal(blockedConfigToolCall("bash", { command: "git pull --ff-only" }, home, user, guard()), undefined);
	assert.match(blockedConfigToolCall("bash", { command: "git pull --ff-only" }, home, user, guard(repositoryRoot, false)) ?? "", /clean/);
	assert.match(blockedConfigToolCall("bash", { command: "git pull" }, home, user, guard()) ?? "", /worktree/);
});

test("blocks checkout shell writes and keeps reads usable", () => {
	assert.equal(blockedConfigToolCall("bash", { command: "git status --short" }, home, user, guard()), undefined);
	assert.match(blockedConfigToolCall("bash", { command: "touch leaked.md" }, home, user, guard()) ?? "", /worktree/);
	assert.match(blockedConfigToolCall("bash", { command: `printf x > ${repositoryRoot}/leaked.md` }, home, user, guard(worktreeRoot)) ?? "", /worktree/);
});

test("blocks absolute, home-variable, tilde, and changed-directory references from a worktree", () => {
	const root = `${home}/Documents/agents`;
	const context: GuardContext = {
		cwd: worktreeRoot,
		repositoryRoot: root,
		isRepositoryClean: () => true,
		worktreeRoots: () => [root, worktreeRoot],
	};
	const relativeRoot = relative(worktreeRoot, root);
	for (const command of [
		`printf x > ${root}/leaked.md`,
		`printf x > ${home}/Documents/./agents/leaked.md`,
		"printf x > $HOME/Documents/agents/leaked.md",
		"printf x > ${HOME}/Documents/agents/leaked.md",
		"printf x > \"$HOME\"/Documents/agents/leaked.md",
		"printf x > \"${HOME}\"/Documents/agents/leaked.md",
		"printf x > $HOME/Documents//agents/leaked.md",
		"printf x > ~/Documents/agents/leaked.md",
		`printf x > ~${user}/Documents/agents/leaked.md`,
		`cd ${root} && rm -rf skills`,
		`cd ${home}/Documents && touch agents/leaked.md`,
		`cd ${home}/Documents && cd agents && touch leaked.md`,
		"cd ~ && cd Documents/agents && touch leaked.md",
		`printf x > ${home}/Documents/x/../agents/leaked.md`,
		"cd ~/Documents/agents && touch leaked.md",
		`printf x > ${relativeRoot}/leaked.md`,
		`cd ${relativeRoot} && touch leaked.md`,
	]) {
		assert.match(blockedConfigToolCall("bash", { command }, home, user, context) ?? "", /worktree/, command);
	}
});

test("keeps a nested feature worktree writable", () => {
	const nestedWorktree = `${repositoryRoot}/.claude/worktrees/feature`;
	const context = guard(nestedWorktree, true, [repositoryRoot, nestedWorktree]);
	assert.equal(blockedConfigToolCall("write", { path: `${nestedWorktree}/skills/x.md` }, home, user, context), undefined);
	assert.equal(blockedConfigToolCall("bash", { command: "touch note.md" }, home, user, context), undefined);
	assert.equal(blockedConfigToolCall("Agent", { subagent_type: "implementer" }, home, user, context), undefined);
});

test("allows shell writes aimed at nested and sibling worktrees", () => {
	const root = `${home}/Documents/agents`;
	const nestedWorktree = `${root}/.claude/worktrees/feature`;
	const siblingWorktree = `${root}-worktrees/feature`;
	const context: GuardContext = {
		cwd: worktreeRoot,
		repositoryRoot: root,
		isRepositoryClean: () => true,
		worktreeRoots: () => [root, nestedWorktree, siblingWorktree, worktreeRoot],
	};
	assert.equal(blockedConfigToolCall("bash", { command: `touch ${nestedWorktree}/x.md` }, home, user, context), undefined);
	assert.equal(blockedConfigToolCall("bash", { command: `touch ${siblingWorktree}/x.md` }, home, user, context), undefined);
	assert.match(blockedConfigToolCall("bash", { command: `touch ${nestedWorktree}/../../../leaked.md` }, home, user, context) ?? "", /worktree/);
});

test("requires worktree isolation for write-capable child agents", () => {
	assert.equal(blockedConfigToolCall("Agent", { subagent_type: "Explore" }, home, user, guard()), undefined);
	assert.equal(blockedConfigToolCall("Agent", { subagent_type: "code-reviewer" }, home, user, guard()), undefined);
	assert.match(blockedConfigToolCall("Agent", { subagent_type: "implementer" }, home, user, guard()) ?? "", /worktree/);
	assert.match(blockedConfigToolCall("Agent", { subagent_type: "general-purpose", isolation: "off" }, home, user, guard()) ?? "", /worktree/);
	assert.equal(blockedConfigToolCall("Agent", { subagent_type: "implementer", isolation: "worktree" }, home, user, guard()), undefined);
	assert.equal(blockedConfigToolCall("Agent", { subagent_type: "implementer" }, home, user, guard(worktreeRoot)), undefined);
});
