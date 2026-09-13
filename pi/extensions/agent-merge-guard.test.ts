import assert from "node:assert/strict";
import { registerHooks } from "node:module";
import { homedir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent";

type Handler = (event: unknown, context: ExtensionContext) => unknown;
type RunnerResult = { status: number | null; stdout?: string; stderr?: string };
type Runner = (binary: string, args: string[]) => RunnerResult;

async function loadAgentMergeGuard(): Promise<typeof import("./agent-merge-guard.ts").default> {
	const hooks = registerHooks({
		resolve(specifier, context, nextResolve) {
			if (specifier === "@earendil-works/pi-coding-agent") {
				return { url: "agent-merge-guard:test-api", shortCircuit: true };
			}
			return nextResolve(specifier, context);
		},
		load(url, context, nextLoad) {
			if (url === "agent-merge-guard:test-api") {
				return {
					format: "module",
					source: "export const isToolCallEventType = (toolName, event) => event.type === 'tool_call' && event.toolName === toolName;",
					shortCircuit: true,
				};
			}
			return nextLoad(url, context);
		},
	});
	try {
		return (await import("./agent-merge-guard.ts")).default;
	} finally {
		hooks.deregister();
	}
}

function createHarness(): {
	api: ExtensionAPI;
	fire(event: unknown, cwd: string): Promise<unknown>;
} {
	let handler: Handler | undefined;
	const api = {
		on(eventName: string, registeredHandler: Handler) {
			assert.equal(eventName, "tool_call");
			handler = registeredHandler;
		},
	} as unknown as ExtensionAPI;
	return {
		api,
		async fire(event: unknown, cwd: string): Promise<unknown> {
			assert.ok(handler, "missing tool-call handler");
			return handler(event, { cwd } as ExtensionContext);
		},
	};
}

function bashEvent(command: string): unknown {
	return { type: "tool_call", toolName: "bash", input: { command } };
}

const expectedBinary = join(homedir(), ".local", "lib", "agents-tools", "agent-merge-guard");

test("allows checker status zero and passes the exact invocation", async () => {
	const agentMergeGuard = await loadAgentMergeGuard();
	const harness = createHarness();
	const calls: Array<{ binary: string; args: string[] }> = [];
	const run: Runner = (binary, args) => {
		calls.push({ binary, args });
		return { status: 0 };
	};
	agentMergeGuard(harness.api, run);
	assert.equal(await harness.fire(bashEvent("git status"), "/workspace"), undefined);
	assert.deepEqual(calls, [
		{
			binary: expectedBinary,
			args: ["--check", "git status", "--cwd", "/workspace"],
		},
	]);
});

test("blocks checker status one with stdout verbatim", async () => {
	const agentMergeGuard = await loadAgentMergeGuard();
	const harness = createHarness();
	agentMergeGuard(harness.api, () => ({ status: 1, stdout: " blocked output \n", stderr: "ignored" }));
	assert.deepEqual(await harness.fire(bashEvent("git merge topic"), "/workspace"), {
		block: true,
		reason: " blocked output \n",
	});
});

test("uses stderr when a failed checker has no stdout", async () => {
	const agentMergeGuard = await loadAgentMergeGuard();
	const harness = createHarness();
	agentMergeGuard(harness.api, () => ({ status: 1, stdout: "", stderr: "checker failed" }));
	assert.deepEqual(await harness.fire(bashEvent("git merge topic"), "/workspace"), {
		block: true,
		reason: "checker failed",
	});
});

test("blocks a checker with null status", async () => {
	const agentMergeGuard = await loadAgentMergeGuard();
	const harness = createHarness();
	agentMergeGuard(harness.api, () => ({ status: null }));
	assert.deepEqual(await harness.fire(bashEvent("git merge topic"), "/workspace"), {
		block: true,
		reason: "Blocked merge: agent-merge-guard could not execute.",
	});
});

test("blocks isolated child agents because they would disable the guard", async () => {
	const agentMergeGuard = await loadAgentMergeGuard();
	const harness = createHarness();
	agentMergeGuard(harness.api, () => ({ status: 0 }));
	assert.deepEqual(
		await harness.fire(
			{ type: "tool_call", toolName: "Agent", input: { subagent_type: "general-purpose", isolated: true } },
			"/workspace",
		),
		{
			block: true,
			reason: "Run the child agent without isolated mode so agent-merge-guard remains active.",
		},
	);
});

test("does not invoke the runner for other non-Bash tool calls", async () => {
	const agentMergeGuard = await loadAgentMergeGuard();
	const harness = createHarness();
	let callCount = 0;
	agentMergeGuard(harness.api, () => {
		callCount += 1;
		return { status: 0 };
	});
	assert.equal(
		await harness.fire({ type: "tool_call", toolName: "write", input: { path: "file" } }, "/workspace"),
		undefined,
	);
	assert.equal(
		await harness.fire(
			{ type: "tool_call", toolName: "Agent", input: { subagent_type: "general-purpose" } },
			"/workspace",
		),
		undefined,
	);
	assert.equal(callCount, 0);
});
