import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

import { loadExtensionModule } from "../../test-support.ts";

let dispose: (() => Promise<void>) | undefined;

afterEach(async () => {
	await dispose?.();
	dispose = undefined;
	delete process.env.CSG_RESULT_PATH;
});

test("throws when CSG_RESULT_PATH is not set", async () => {
	delete process.env.CSG_RESULT_PATH;
	const loaded = await loadExtensionModule("judge/agent/index.ts");
	dispose = loaded.dispose;
	const fakePi = { registerTool: () => {}, on: () => {} };
	assert.throws(() => loaded.module.default(fakePi), /CSG_RESULT_PATH/);
});

test("registers submit_verdict and a before_agent_start system prompt when CSG_RESULT_PATH is set", async () => {
	process.env.CSG_RESULT_PATH = "/tmp/does-not-matter.json";
	const loaded = await loadExtensionModule("judge/agent/index.ts");
	dispose = loaded.dispose;
	const registeredTools: string[] = [];
	const handlers: Record<string, unknown> = {};
	const fakePi = {
		registerTool: (def: { name: string }) => registeredTools.push(def.name),
		on: (event: string, handler: unknown) => (handlers[event] = handler),
	};
	loaded.module.default(fakePi);
	assert.deepEqual(registeredTools, ["submit_verdict"]);
	assert.ok(typeof handlers.before_agent_start === "function");
	assert.ok(typeof handlers.agent_end === "function");
	const result = await (handlers.before_agent_start as (e: unknown) => Promise<{ systemPrompt: string }>)(undefined);
	assert.ok(result.systemPrompt.includes("submit_verdict"));
});

test("agent_end shuts the worker down", async () => {
	process.env.CSG_RESULT_PATH = "/tmp/does-not-matter.json";
	const loaded = await loadExtensionModule("judge/agent/index.ts");
	dispose = loaded.dispose;
	const handlers: Record<string, unknown> = {};
	const fakePi = { registerTool: () => {}, on: (event: string, handler: unknown) => (handlers[event] = handler) };
	loaded.module.default(fakePi);
	let didShutdown = false;
	await (handlers.agent_end as (e: unknown, ctx: { shutdown: () => void }) => Promise<void>)(undefined, { shutdown: () => (didShutdown = true) });
	assert.equal(didShutdown, true);
});
