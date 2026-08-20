import * as assert from "node:assert/strict";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { test, type TestContext } from "node:test";

import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent";

import liveDiff from "./live-diff.ts";
import { makeFixtureRepo, type FixtureRepo } from "./live-diff/fixtures.ts";

type Handler = (event: unknown, ctx: ExtensionContext) => unknown;
type CommandHandler = (args: unknown, ctx: ExtensionContext) => Promise<void>;

interface Recorder {
	setStatusCalls: { key: string; text: string }[];
	customCalls: unknown[];
	notifyCalls: unknown[];
	isShutdown: boolean;
	postShutdownUiCalls: string[];
}

function createRecorder(): Recorder {
	return {
		setStatusCalls: [],
		customCalls: [],
		notifyCalls: [],
		isShutdown: false,
		postShutdownUiCalls: [],
	};
}

function createFakeCtx(recorder: Recorder, cwd: string): ExtensionContext {
	function guard(method: string): void {
		if (recorder.isShutdown) {
			recorder.postShutdownUiCalls.push(method);
			throw new Error(`stale ctx: ${method} after session_shutdown`);
		}
	}
	return {
		cwd,
		hasUI: true,
		mode: "tui",
		ui: {
			setStatus(key: string, text: string) {
				recorder.setStatusCalls.push({ key, text });
				guard("setStatus");
			},
			async custom(factory: unknown, options: unknown) {
				recorder.customCalls.push({ factory, options });
				guard("custom");
				return undefined;
			},
			notify(...args: unknown[]) {
				recorder.notifyCalls.push(args);
				guard("notify");
			},
		},
	} as unknown as ExtensionContext;
}

function createFakePi() {
	const handlers = new Map<string, Handler>();
	const commands = new Map<string, CommandHandler>();
	const api = {
		on(event: string, handler: Handler) {
			handlers.set(event, handler);
		},
		registerCommand(name: string, options: { handler: CommandHandler }) {
			commands.set(name, options.handler);
		},
	} as unknown as ExtensionAPI;
	return {
		api,
		fire(event: string, payload: unknown, ctx: ExtensionContext): unknown {
			const handler = handlers.get(event);
			assert.ok(handler, `missing handler for ${event}`);
			return handler(payload, ctx);
		},
		command(name: string): CommandHandler {
			const handler = commands.get(name);
			assert.ok(handler, `missing command ${name}`);
			return handler;
		},
		commandNames(): string[] {
			return [...commands.keys()];
		},
	};
}

function sleep(ms: number): Promise<void> {
	return new Promise((resolvePromise) => setTimeout(resolvePromise, ms));
}

async function waitFor(predicate: () => boolean, timeoutMs = 3000): Promise<void> {
	const startedAt = Date.now();
	while (!predicate()) {
		if (Date.now() - startedAt > timeoutMs) {
			throw new Error("waitFor timed out");
		}
		await sleep(25);
	}
}

interface Harness {
	pi: ReturnType<typeof createFakePi>;
	recorder: Recorder;
	ctx: ExtensionContext;
	repo: FixtureRepo;
}

function createHarness(t: TestContext): Harness {
	const pi = createFakePi();
	liveDiff(pi.api);
	const recorder = createRecorder();
	const repo = makeFixtureRepo(os.tmpdir());
	t.after(() => repo.cleanup());
	const ctx = createFakeCtx(recorder, repo.root);
	return { pi, recorder, ctx, repo };
}

test("registers the diff command", (t) => {
	const { pi } = createHarness(t);
	assert.ok(pi.commandNames().includes("diff"));
});

test("TC-04 badge after write tool, none for read tool, one more on settle", async (t) => {
	const { pi, recorder, ctx, repo } = createHarness(t);
	await pi.fire("agent_start", {}, ctx);

	fs.appendFileSync(path.join(repo.root, "alpha.txt"), "edit by tool\n");
	pi.fire("tool_execution_end", { toolName: "edit" }, ctx);
	await waitFor(() => recorder.setStatusCalls.length >= 1);
	const afterWrite = recorder.setStatusCalls.length;
	const lastWrite = recorder.setStatusCalls[afterWrite - 1];
	assert.equal(lastWrite.key, "live-diff");
	assert.match(lastWrite.text, /^req |^all |^diff/);

	pi.fire("tool_execution_end", { toolName: "read" }, ctx);
	await sleep(400);
	assert.equal(recorder.setStatusCalls.length, afterWrite);

	pi.fire("agent_settled", {}, ctx);
	await waitFor(() => recorder.setStatusCalls.length > afterWrite);
	assert.equal(
		recorder.setStatusCalls[recorder.setStatusCalls.length - 1].key,
		"live-diff",
	);
});

test("TC-12 external edit shows up in the settle badge", async (t) => {
	const { pi, recorder, ctx, repo } = createHarness(t);
	await pi.fire("agent_start", {}, ctx);
	pi.fire("agent_settled", {}, ctx);
	await waitFor(() => recorder.setStatusCalls.length >= 1);
	const cleanBadge = recorder.setStatusCalls[recorder.setStatusCalls.length - 1].text;
	assert.equal(cleanBadge, "diff clean");

	fs.writeFileSync(path.join(repo.root, "external.txt"), "external edit\n");
	const before = recorder.setStatusCalls.length;
	pi.fire("agent_settled", {}, ctx);
	await waitFor(() => recorder.setStatusCalls.length > before);
	const externalBadge = recorder.setStatusCalls[recorder.setStatusCalls.length - 1].text;
	assert.notEqual(externalBadge, cleanBadge);
	assert.match(externalBadge, /req/);
	assert.match(externalBadge, /all/);
});

test("TC-13 shutdown clears the trailing timer and no ctx use follows", async (t) => {
	const { pi, recorder, ctx, repo } = createHarness(t);
	await pi.fire("agent_start", {}, ctx);

	fs.appendFileSync(path.join(repo.root, "alpha.txt"), "first edit\n");
	pi.fire("tool_execution_end", { toolName: "edit" }, ctx);
	pi.fire("tool_execution_end", { toolName: "write" }, ctx);
	await waitFor(() => recorder.setStatusCalls.length >= 1);
	await sleep(150);

	const beforeShutdown = recorder.setStatusCalls.length;
	pi.fire("session_shutdown", {}, ctx);
	recorder.isShutdown = true;
	await sleep(450);
	assert.equal(recorder.setStatusCalls.length, beforeShutdown);
});

test("TC-19 shutdown during an in-flight refresh reports nothing afterwards", async (t) => {
	const { pi, recorder, ctx, repo } = createHarness(t);
	const rejections: unknown[] = [];
	const onRejection = (reason: unknown) => {
		rejections.push(reason);
	};
	process.on("unhandledRejection", onRejection);
	try {
		await pi.fire("agent_start", {}, ctx);

		fs.appendFileSync(path.join(repo.root, "alpha.txt"), "in-flight edit\n");
		for (let index = 0; index < 25; index += 1) {
			pi.fire("tool_execution_end", { toolName: "edit" }, ctx);
		}

		const beforeShutdown = recorder.setStatusCalls.length;
		pi.fire("session_shutdown", {}, ctx);
		recorder.isShutdown = true;

		await sleep(1200);

		assert.deepEqual(recorder.postShutdownUiCalls, []);
		assert.equal(recorder.setStatusCalls.length, beforeShutdown);
		assert.equal(rejections.length, 0);
	} finally {
		process.off("unhandledRejection", onRejection);
	}
});

test("TC-14 /diff during an in-flight refresh opens exactly one overlay", async (t) => {
	const { pi, recorder, ctx, repo } = createHarness(t);
	const rejections: unknown[] = [];
	const onRejection = (reason: unknown) => {
		rejections.push(reason);
	};
	process.on("unhandledRejection", onRejection);
	try {
		await pi.fire("agent_start", {}, ctx);
		fs.appendFileSync(path.join(repo.root, "beta.txt"), "mid-flight edit\n");
		pi.fire("tool_execution_end", { toolName: "edit" }, ctx);
		await pi.command("diff")("", ctx);
		assert.equal(recorder.customCalls.length, 1);
		await sleep(300);
		assert.equal(rejections.length, 0);
	} finally {
		process.off("unhandledRejection", onRejection);
	}
});
