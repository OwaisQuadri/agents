import { test } from "node:test";
import assert from "node:assert/strict";

import { openInNvim } from "./nvim.ts";

const CWD = "/Users/tester/repo";

interface RecordedCall {
	command: string;
	args: string[];
}

function makeExec(): {
	exec: (
		command: string,
		args: string[],
	) => Promise<{ code: number; stdout: string; stderr: string }>;
	calls: RecordedCall[];
} {
	const calls: RecordedCall[] = [];
	const snapshot = {
		result: {
			snapshot: {
				workspaces: [
					{
						workspace_id: "w1",
						worktree: { checkout_path: CWD },
					},
				],
				tabs: [
					{ tab_id: "w1:t1", workspace_id: "w1", label: "agent" },
					{ tab_id: "w1:t2", workspace_id: "w1", label: "editor" },
				],
				panes: [
					{ pane_id: "w1:p1", tab_id: "w1:t1" },
					{ pane_id: "w1:p2", tab_id: "w1:t2" },
				],
			},
		},
	};
	const processInfo = {
		result: { process_info: { foreground_processes: [{ name: "nvim" }] } },
	};
	const exec = async (command: string, args: string[]) => {
		calls.push({ command, args });
		if (args[0] === "api" && args[1] === "snapshot") {
			return { code: 0, stdout: JSON.stringify(snapshot), stderr: "" };
		}
		if (args[0] === "pane" && args[1] === "process-info") {
			return { code: 0, stdout: JSON.stringify(processInfo), stderr: "" };
		}
		return { code: 0, stdout: "", stderr: "" };
	};
	return { exec, calls };
}

function sentCommand(calls: RecordedCall[]): string | null {
	const keys = calls
		.filter((call) => call.args[0] === "pane" && call.args[1] === "send-keys")
		.map((call) => call.args[3]);
	const textCall = calls.find(
		(call) => call.args[0] === "pane" && call.args[1] === "send-text",
	);
	if (textCall === undefined) {
		return null;
	}
	assert.deepEqual(
		keys,
		["esc", "colon", "e", "space", "enter"],
		"the command line must be typed as keys, with enter last",
	);
	assert.equal(
		calls.some((call) => call.args[1] === "run"),
		false,
		"pane run pastes into the buffer and must never be used to drive nvim",
	);
	return ":e " + textCall.args[3];
}

function sentText(calls: RecordedCall[]): string | null {
	return sentCommand(calls);
}

test("ordinary path opens and sends one escaped argv element", async () => {
	const { exec, calls } = makeExec();
	const isOpened = await openInNvim(exec, CWD, "src/api/fetchUsers.ts");
	assert.equal(isOpened, true);
	assert.equal(sentText(calls), ":e /Users/tester/repo/src/api/fetchUsers.ts");
});

test("vim metacharacters are escaped, never stripped", async () => {
	const { exec, calls } = makeExec();
	const isOpened = await openInNvim(exec, CWD, "two words/pct%and#hash|pipe.txt");
	assert.equal(isOpened, true);
	assert.equal(
		sentText(calls),
		":e /Users/tester/repo/two\\ words/pct\\%and\\#hash\\|pipe.txt",
	);
});

test("a path containing LF returns false and sends nothing", async () => {
	const { exec, calls } = makeExec();
	const isOpened = await openInNvim(exec, CWD, "evil\n:!echo PWNED\n.txt");
	assert.equal(isOpened, false);
	assert.equal(sentText(calls), null);
});

test("a path containing CR returns false and sends nothing", async () => {
	const { exec, calls } = makeExec();
	const isOpened = await openInNvim(exec, CWD, "carriage\rreturn.txt");
	assert.equal(isOpened, false);
	assert.equal(sentText(calls), null);
});

test("a path containing TAB returns false and sends nothing", async () => {
	const { exec, calls } = makeExec();
	const isOpened = await openInNvim(exec, CWD, "tab\there.txt");
	assert.equal(isOpened, false);
	assert.equal(sentText(calls), null);
});

test("a path escaping cwd via .. returns false and sends nothing", async () => {
	const { exec, calls } = makeExec();
	const isOpened = await openInNvim(exec, CWD, "../../../etc/passwd");
	assert.equal(isOpened, false);
	assert.equal(sentText(calls), null);
});

test("an absolute path outside cwd returns false and sends nothing", async () => {
	const { exec, calls } = makeExec();
	const isOpened = await openInNvim(exec, CWD, "/etc/passwd");
	assert.equal(isOpened, false);
	assert.equal(sentText(calls), null);
});

test("a .. segment that stays inside cwd still opens, normalized", async () => {
	const { exec, calls } = makeExec();
	const isOpened = await openInNvim(exec, CWD, "src/sub/../api.ts");
	assert.equal(isOpened, true);
	assert.equal(sentText(calls), ":e /Users/tester/repo/src/api.ts");
});

test("a basename starting with a dash still opens as a filename", async () => {
	const { exec, calls } = makeExec();
	const isOpened = await openInNvim(exec, CWD, "-c");
	assert.equal(isOpened, true);
	assert.equal(sentText(calls), ":e /Users/tester/repo/-c");
});

test("every recorded exec call passes arguments as an argv array", async () => {
	const { exec, calls } = makeExec();
	await openInNvim(exec, CWD, "src/api.ts");
	assert.ok(calls.length > 0);
	for (const call of calls) {
		assert.ok(Array.isArray(call.args));
		for (const arg of call.args) {
			assert.equal(typeof arg, "string");
		}
		assert.ok(!/[;&|]{1}/.test(call.command));
		assert.equal(call.args.join("").includes("\n"), false);
	}
});

test("a missing herdr link returns false and never throws", async () => {
	const failing = async () => ({ code: 127, stdout: "", stderr: "not found" });
	const isOpened = await openInNvim(failing, CWD, "src/api.ts");
	assert.equal(isOpened, false);
});

test("a throwing exec returns false and never throws", async () => {
	const throwing = async () => {
		throw new Error("spawn failure");
	};
	const isOpened = await openInNvim(throwing, CWD, "src/api.ts");
	assert.equal(isOpened, false);
});
