import assert from "node:assert/strict";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { after, test } from "node:test";

import turboExtension from "./turbo.ts";
import {
	REVIEW_AGENT_TYPES,
	SESSION_MODEL_OVERRIDE_CHANNEL,
	highestTierPrimaryFrom,
	latestTurboState,
	parseModelReference,
	turboOverrideFor,
} from "./turbo/policy.ts";

const primary = { model: "openai-codex/gpt-6-astra", thinking: "medium" as const };
const parent = { model: "anthropic/claude-opus-5", thinking: "high" as const };
const originalAgentDirectory = process.env.PI_CODING_AGENT_DIR;

after(() => {
	if (originalAgentDirectory === undefined) delete process.env.PI_CODING_AGENT_DIR;
	else process.env.PI_CODING_AGENT_DIR = originalAgentDirectory;
});

function settingsRoot(body: unknown): string {
	const root = mkdtempSync(join(tmpdir(), "turbo-extension-"));
	writeFileSync(join(root, "settings.json"), JSON.stringify(body), "utf8");
	return root;
}

function runtime(entries: unknown[] = [], isInitiallyReady = true) {
	const commands = new Map<string, { handler: (args: string, ctx: unknown) => Promise<void> }>();
	const handlers = new Map<string, (event: unknown, ctx: unknown) => Promise<void>>();
	const busHandlers = new Map<string, Set<(payload: unknown) => void>>();
	const emitted: Array<{ event: string; payload: unknown }> = [];
	const appended: Array<{ type: string; data: unknown }> = [];
	const notifications: Array<{ message: string; level: string }> = [];
	const statuses: Array<{ key: string; value: string | undefined }> = [];
	const selectedModels: unknown[] = [];
	const thinkingLevels: string[] = [];
	const modelResults: boolean[] = [];
	const overrideErrors: Array<string | undefined> = [];
	let isModelAvailable = true;
	let isOverrideResponderAvailable = true;
	let overrideError: string | undefined;
	let thinkingLevel = parent.thinking;

	const models = new Map([
		[primary.model, { provider: "openai-codex", id: "gpt-6-astra" }],
		[parent.model, { provider: "anthropic", id: "claude-opus-5" }],
	]);
	const pi = {
		registerCommand(name: string, command: { handler: (args: string, ctx: unknown) => Promise<void> }) {
			commands.set(name, command);
		},
		on(name: string, handler: (event: unknown, ctx: unknown) => Promise<void>) {
			handlers.set(name, handler);
		},
		events: {
			on(name: string, handler: (payload: unknown) => void) {
				const handlersForName = busHandlers.get(name) ?? new Set();
				handlersForName.add(handler);
				busHandlers.set(name, handlersForName);
				return () => handlersForName.delete(handler);
			},
			emit(event: string, payload: unknown) {
				emitted.push({ event, payload });
				for (const handler of busHandlers.get(event) ?? []) handler(payload);
				if (!isOverrideResponderAvailable || event !== SESSION_MODEL_OVERRIDE_CHANNEL || typeof payload !== "object" || payload === null || !("requestId" in payload)) return;
				const requestId = String((payload as { requestId: unknown }).requestId);
				const error = overrideErrors.length > 0 ? overrideErrors.shift() : overrideError;
				queueMicrotask(() => {
					pi.events.emit(`${event}:reply:${requestId}`, error === undefined ? { success: true } : { success: false, error });
				});
			},
		},
		async setModel(value: unknown) {
			selectedModels.push(value);
			return modelResults.shift() ?? isModelAvailable;
		},
		getThinkingLevel() {
			return thinkingLevel;
		},
		setThinkingLevel(level: string) {
			thinkingLevel = level;
			thinkingLevels.push(level);
		},
		appendEntry(type: string, data: unknown) {
			appended.push({ type, data });
		},
	};
	const ctx = {
		model: models.get(parent.model),
		modelRegistry: {
			find(provider: string, id: string) {
				return models.get(`${provider}/${id}`);
			},
		},
		sessionManager: {
			getBranch() {
				return entries;
			},
		},
		ui: {
			theme: { fg: (_color: string, value: string) => value },
			notify(message: string, level: string) {
				notifications.push({ message, level });
			},
			setStatus(key: string, value: string | undefined) {
				statuses.push({ key, value });
			},
		},
	};

	turboExtension(pi as never);
	if (isInitiallyReady) pi.events.emit("subagents:ready", {});

	return {
		appended,
		commands,
		ctx,
		emitted,
		handlers,
		notifications,
		pi,
		selectedModels,
		setModelAvailable(value: boolean) {
			isModelAvailable = value;
		},
		setModelResults(...values: boolean[]) {
			modelResults.push(...values);
		},
		setOverrideError(value: string | undefined) {
			overrideError = value;
		},
		setOverrideErrors(...values: Array<string | undefined>) {
			overrideErrors.push(...values);
		},
		setOverrideResponderAvailable(value: boolean) {
			isOverrideResponderAvailable = value;
		},
		statuses,
		thinkingLevels,
	};
}

test("highestTierPrimaryFrom selects the highest numbered tier", () => {
	assert.deepEqual(highestTierPrimaryFrom({ tierPrimaries: { T5: parent, T6: primary } }), { tier: "T6", ...primary });
	assert.deepEqual(highestTierPrimaryFrom({ tierPrimaries: { T6: primary, T5: parent } }), { tier: "T6", ...primary });
	assert.deepEqual(highestTierPrimaryFrom({ tierPrimaries: { T10: primary, T9: parent } }), { tier: "T10", ...primary });
	assert.deepEqual(highestTierPrimaryFrom({ tierPrimaries: { T0: primary } }), { tier: "T0", ...primary });
	assert.equal(highestTierPrimaryFrom({ tierPrimaries: { T5: parent, T07: primary } }), undefined);
	assert.equal(highestTierPrimaryFrom({ tierPrimaries: { T5: parent, t7: primary } }), undefined);
	assert.equal(highestTierPrimaryFrom({ tierPrimaries: { T5: parent, " T7": primary } }), undefined);
	assert.equal(highestTierPrimaryFrom({ tierPrimaries: { T5: primary, custom: parent } }), undefined);
	assert.equal(highestTierPrimaryFrom({}), undefined);
	assert.equal(highestTierPrimaryFrom({ tierPrimaries: [] }), undefined);
	assert.equal(
		highestTierPrimaryFrom({ tierPrimaries: { T5: primary, T6: { model: "astra", thinking: "medium" } } }),
		undefined,
	);
	assert.equal(highestTierPrimaryFrom({ tierPrimaries: { T6: { model: primary.model, thinking: "extreme" } } }), undefined);
});

test("parseModelReference preserves the provider and full model id", () => {
	assert.deepEqual(parseModelReference("openai-codex/gpt-6-astra"), {
		provider: "openai-codex",
		id: "gpt-6-astra",
	});
	assert.equal(parseModelReference("gpt-6-astra"), undefined);
});

test("latestTurboState returns only the newest valid active entry", () => {
	const active = { isActive: true, tier: "T6", ...primary, parent };
	const entries = [
		{ type: "custom", customType: "turbo-state", data: active },
		{ type: "custom", customType: "turbo-state", data: { isActive: false } },
	];
	assert.deepEqual(latestTurboState(entries.slice(0, 1)), active);
	assert.deepEqual(
		latestTurboState([{ type: "custom", customType: "turbo-state", data: { ...active, tier: "T5" } }])?.tier,
		"T5",
	);
	assert.equal(latestTurboState([{ type: "custom", customType: "turbo-state", data: { ...active, tier: "T07" } }]), undefined);
	assert.equal(latestTurboState(entries), undefined);
});

test("turboOverrideFor excludes the four independent check agents", () => {
	assert.deepEqual(turboOverrideFor(primary), {
		model: primary.model,
		thinkingLevel: primary.thinking,
		excludedAgentTypes: REVIEW_AGENT_TYPES,
	});
	assert.deepEqual(REVIEW_AGENT_TYPES, ["anchor-verifier", "code-reviewer", "maestro-tester", "spec-tester"]);
});

async function settle(): Promise<void> {
	await new Promise((resolve) => setImmediate(resolve));
}

function overrideRequest(harness: ReturnType<typeof runtime>, index = 0): Record<string, unknown> {
	const requests = harness.emitted.filter((entry) => entry.event === SESSION_MODEL_OVERRIDE_CHANNEL);
	const payload = { ...(requests[index]?.payload as Record<string, unknown>) };
	delete payload.requestId;
	return payload;
}

test("/turbo selects the highest configured tier, captures the parent, persists state, and emits the override", async () => {
	process.env.PI_CODING_AGENT_DIR = settingsRoot({ tierPrimaries: { T5: parent, T6: primary } });
	const harness = runtime();

	await harness.commands.get("turbo")!.handler("", harness.ctx);

	assert.deepEqual(harness.selectedModels, [{ provider: "openai-codex", id: "gpt-6-astra" }]);
	assert.deepEqual(harness.thinkingLevels, ["medium"]);
	assert.deepEqual(harness.appended, [{ type: "turbo-state", data: { isActive: true, tier: "T6", ...primary, parent } }]);
	assert.deepEqual(overrideRequest(harness), turboOverrideFor(primary));
	assert.deepEqual(harness.statuses.at(-1), { key: "turbo", value: "turbo:T6" });
});

test("/turbo toggles off, restores the parent, clears the override and status, and persists inactivity", async () => {
	process.env.PI_CODING_AGENT_DIR = settingsRoot({ tierPrimaries: { T5: primary } });
	const harness = runtime();

	await harness.commands.get("turbo")!.handler("", harness.ctx);
	await harness.commands.get("turbo")!.handler("", harness.ctx);

	assert.deepEqual(harness.selectedModels, [
		{ provider: "openai-codex", id: "gpt-6-astra" },
		{ provider: "anthropic", id: "claude-opus-5" },
	]);
	assert.deepEqual(harness.thinkingLevels, ["medium", "high"]);
	assert.deepEqual(harness.appended.at(-1), { type: "turbo-state", data: { isActive: false } });
	assert.deepEqual(overrideRequest(harness, 1), { clear: true });
	assert.deepEqual(harness.statuses.at(-1), { key: "turbo", value: undefined });
});

test("/turbo repeats activation and deactivation deterministically", async () => {
	process.env.PI_CODING_AGENT_DIR = settingsRoot({ tierPrimaries: { T5: primary } });
	const harness = runtime();

	await harness.commands.get("turbo")!.handler("", harness.ctx);
	await harness.commands.get("turbo")!.handler("", harness.ctx);
	await harness.commands.get("turbo")!.handler("", harness.ctx);

	assert.deepEqual(harness.appended.map((entry) => entry.data), [
		{ isActive: true, tier: "T5", ...primary, parent },
		{ isActive: false },
		{ isActive: true, tier: "T5", ...primary, parent },
	]);
});

test("/turbo reports missing highest-tier and parent models separately", async () => {
	process.env.PI_CODING_AGENT_DIR = settingsRoot({
		tierPrimaries: { T5: primary, T6: { model: "invalid", thinking: "medium" } },
	});
	const missingTier = runtime();
	await missingTier.commands.get("turbo")!.handler("", missingTier.ctx);
	assert.match(missingTier.notifications.at(-1)?.message ?? "", /valid highest-tier model/i);
	assert.equal(missingTier.selectedModels.length, 0);
	assert.equal(missingTier.appended.length, 0);

	process.env.PI_CODING_AGENT_DIR = settingsRoot({ tierPrimaries: { T6: primary } });
	const missingParent = runtime();
	await missingParent.commands.get("turbo")!.handler("", { ...missingParent.ctx, model: undefined });
	assert.match(missingParent.notifications.at(-1)?.message ?? "", /parent session model/i);
	assert.equal(missingParent.selectedModels.length, 0);
	assert.equal(missingParent.appended.length, 0);
});

test("/turbo changes nothing when the configured model cannot authenticate", async () => {
	process.env.PI_CODING_AGENT_DIR = settingsRoot({ tierPrimaries: { T5: primary } });
	const harness = runtime();
	harness.setModelAvailable(false);

	await harness.commands.get("turbo")!.handler("", harness.ctx);

	assert.equal(harness.appended.length, 0);
	assert.equal(harness.emitted.filter((entry) => entry.event === SESSION_MODEL_OVERRIDE_CHANNEL).length, 0);
	assert.match(harness.notifications.at(-1)?.message ?? "", /could not authenticate/i);
});

test("session_start restores a valid Turbo state", async () => {
	const state = { isActive: true, tier: "T5", ...primary, parent };
	const harness = runtime([{ type: "custom", customType: "turbo-state", data: state }]);

	await harness.handlers.get("session_start")!({}, harness.ctx);
	await settle();

	assert.deepEqual(harness.selectedModels, [{ provider: "openai-codex", id: "gpt-6-astra" }]);
	assert.deepEqual(overrideRequest(harness), turboOverrideFor(primary));
	assert.equal(harness.appended.length, 0);
});

test("a persisted model authentication failure records inactivity", async () => {
	const state = { isActive: true, tier: "T5", ...primary, parent };
	const harness = runtime([{ type: "custom", customType: "turbo-state", data: state }]);
	harness.setModelAvailable(false);

	await harness.handlers.get("session_start")!({}, harness.ctx);
	await settle();

	assert.deepEqual(harness.appended, [{ type: "turbo-state", data: { isActive: false } }]);
	assert.equal(harness.emitted.filter((entry) => entry.event === SESSION_MODEL_OVERRIDE_CHANNEL).length, 0);
	assert.match(harness.notifications.at(-1)?.message ?? "", /could not authenticate/i);
});

test("an inactive persisted state keeps a resumed or new session inactive", async () => {
	const harness = runtime([{ type: "custom", customType: "turbo-state", data: { isActive: false } }]);

	await harness.handlers.get("session_start")!({}, harness.ctx);

	assert.equal(harness.selectedModels.length, 0);
	assert.equal(harness.emitted.filter((entry) => entry.event === SESSION_MODEL_OVERRIDE_CHANNEL).length, 0);
	assert.deepEqual(harness.statuses.at(-1), { key: "turbo", value: undefined });
});

test("an override failure restores the parent and does not activate Turbo", async () => {
	process.env.PI_CODING_AGENT_DIR = settingsRoot({ tierPrimaries: { T5: primary } });
	const harness = runtime();
	harness.setOverrideErrors("override unavailable", undefined);

	await harness.commands.get("turbo")!.handler("", harness.ctx);

	assert.deepEqual(harness.selectedModels, [
		{ provider: "openai-codex", id: "gpt-6-astra" },
		{ provider: "anthropic", id: "claude-opus-5" },
	]);
	assert.equal(harness.appended.length, 0);
	assert.match(harness.notifications.at(-1)?.message ?? "", /override unavailable/);
});

test("an override failure reports when the parent model rollback also fails", async () => {
	process.env.PI_CODING_AGENT_DIR = settingsRoot({ tierPrimaries: { T5: primary } });
	const harness = runtime();
	harness.setOverrideErrors("override unavailable", undefined);
	harness.setModelResults(true, false);

	await harness.commands.get("turbo")!.handler("", harness.ctx);

	assert.match(harness.notifications.at(-1)?.message ?? "", /could not restore the parent model/i);
	assert.deepEqual(overrideRequest(harness, 1), { clear: true });
	assert.equal(harness.appended.length, 0);
});

test("a failed persisted override records inactivity after restoring the parent", async () => {
	const state = { isActive: true, tier: "T5", ...primary, parent };
	const harness = runtime([{ type: "custom", customType: "turbo-state", data: state }]);
	harness.setOverrideErrors("override unavailable", undefined);

	await harness.handlers.get("session_start")!({}, harness.ctx);
	await settle();

	assert.deepEqual(harness.appended, [{ type: "turbo-state", data: { isActive: false } }]);
	assert.deepEqual(harness.selectedModels, [
		{ provider: "openai-codex", id: "gpt-6-astra" },
		{ provider: "anthropic", id: "claude-opus-5" },
	]);
});

test("a failed persisted rollback reports the parent model failure", async () => {
	const state = { isActive: true, tier: "T5", ...primary, parent };
	const harness = runtime([{ type: "custom", customType: "turbo-state", data: state }]);
	harness.setOverrideErrors("override unavailable", undefined);
	harness.setModelResults(true, false);

	await harness.handlers.get("session_start")!({}, harness.ctx);
	await settle();

	assert.match(harness.notifications.at(-1)?.message ?? "", /could not restore the parent model/i);
	assert.deepEqual(harness.appended, [{ type: "turbo-state", data: { isActive: false } }]);
});

test("an unconfirmed persisted clear remains pending instead of recording inactivity", async () => {
	const state = { isActive: true, tier: "T5", ...primary, parent };
	const harness = runtime([{ type: "custom", customType: "turbo-state", data: state }]);
	harness.setOverrideError("override unavailable");

	await harness.handlers.get("session_start")!({}, harness.ctx);
	await settle();

	assert.equal(harness.appended.length, 0);
	assert.deepEqual(harness.statuses.at(-1), { key: "turbo", value: "turbo:pending" });
	harness.setOverrideError(undefined);
	await harness.commands.get("turbo")!.handler("", harness.ctx);
	assert.deepEqual(harness.appended.at(-1), { type: "turbo-state", data: { isActive: false } });
});

test("a failed parent restore keeps Turbo active and does not clear the worker override", async () => {
	process.env.PI_CODING_AGENT_DIR = settingsRoot({ tierPrimaries: { T5: primary } });
	const harness = runtime();

	await harness.commands.get("turbo")!.handler("", harness.ctx);
	harness.setModelResults(false);
	await harness.commands.get("turbo")!.handler("", harness.ctx);

	assert.deepEqual(harness.appended.map((entry) => entry.data), [{ isActive: true, tier: "T5", ...primary, parent }]);
	assert.equal(harness.emitted.filter((entry) => entry.event === SESSION_MODEL_OVERRIDE_CHANNEL).length, 1);
	assert.deepEqual(harness.statuses.at(-1), { key: "turbo", value: "turbo:T5" });
});

test("a second session waits for a fresh worker-ready event", async () => {
	const state = { isActive: true, tier: "T5", ...primary, parent };
	const harness = runtime([{ type: "custom", customType: "turbo-state", data: state }]);

	await harness.handlers.get("session_start")!({}, harness.ctx);
	await settle();
	await harness.handlers.get("session_shutdown")!({}, harness.ctx);
	harness.setOverrideResponderAvailable(false);
	await harness.handlers.get("session_start")!({}, harness.ctx);
	assert.equal(harness.emitted.filter((entry) => entry.event === SESSION_MODEL_OVERRIDE_CHANNEL).length, 1);
	harness.setOverrideResponderAvailable(true);
	harness.pi.events.emit("subagents:ready", {});
	await settle();

	assert.equal(harness.emitted.filter((entry) => entry.event === SESSION_MODEL_OVERRIDE_CHANNEL).length, 2);
	assert.deepEqual(overrideRequest(harness, 1), turboOverrideFor(primary));
	assert.equal(harness.appended.length, 0);
	assert.deepEqual(harness.statuses.at(-1), { key: "turbo", value: "turbo:T5" });
});

test("a worker readiness timeout preserves the persisted active state for a later resume", async (context) => {
	context.mock.timers.enable({ apis: ["setTimeout"] });
	const state = { isActive: true, tier: "T5", ...primary, parent };
	const harness = runtime([{ type: "custom", customType: "turbo-state", data: state }], false);

	await harness.handlers.get("session_start")!({}, harness.ctx);
	context.mock.timers.tick(5_000);

	assert.equal(harness.appended.length, 0);
	assert.equal(harness.emitted.filter((entry) => entry.event === SESSION_MODEL_OVERRIDE_CHANNEL).length, 0);
	assert.deepEqual(harness.statuses.at(-1), { key: "turbo", value: "turbo:pending" });
	assert.match(harness.notifications.at(-1)?.message ?? "", /worker package is not ready/);

	harness.pi.events.emit("subagents:ready", {});
	await harness.commands.get("turbo")!.handler("", harness.ctx);

	assert.deepEqual(harness.selectedModels.at(-1), { provider: "anthropic", id: "claude-opus-5" });
	assert.deepEqual(harness.appended.at(-1), { type: "turbo-state", data: { isActive: false } });
});

test("a command cannot race a persisted restore before the worker package is ready", async () => {
	const state = { isActive: true, tier: "T5", ...primary, parent };
	const harness = runtime([{ type: "custom", customType: "turbo-state", data: state }], false);
	harness.setOverrideResponderAvailable(false);

	await harness.handlers.get("session_start")!({}, harness.ctx);
	await harness.commands.get("turbo")!.handler("", harness.ctx);
	harness.setOverrideResponderAvailable(true);
	harness.pi.events.emit("subagents:ready", {});
	await settle();

	assert.ok(harness.notifications.some((entry) => entry.message === "Turbo is already changing."));
	assert.deepEqual(overrideRequest(harness), turboOverrideFor(primary));
	assert.deepEqual(harness.statuses.at(-1), { key: "turbo", value: "turbo:T5" });
});

test("a missing override reply sends a compensating clear request", async () => {
	process.env.PI_CODING_AGENT_DIR = settingsRoot({ tierPrimaries: { T5: primary } });
	const harness = runtime();
	harness.setOverrideResponderAvailable(false);

	await harness.commands.get("turbo")!.handler("", harness.ctx);

	assert.deepEqual(overrideRequest(harness, 1), { clear: true });
	assert.deepEqual(harness.selectedModels.at(-1), { provider: "anthropic", id: "claude-opus-5" });
	assert.deepEqual(harness.appended.at(-1), { type: "turbo-state", data: { isActive: true, tier: "T5", ...primary, parent } });
	assert.deepEqual(harness.statuses.at(-1), { key: "turbo", value: "turbo:pending" });
});

test("an uncertain deactivation keeps Turbo active when every compensation fails", async () => {
	process.env.PI_CODING_AGENT_DIR = settingsRoot({ tierPrimaries: { T5: primary } });
	const harness = runtime();

	await harness.commands.get("turbo")!.handler("", harness.ctx);
	harness.setOverrideError("override unavailable");
	await harness.commands.get("turbo")!.handler("", harness.ctx);

	assert.equal(harness.emitted.filter((entry) => entry.event === SESSION_MODEL_OVERRIDE_CHANNEL).length, 4);
	assert.deepEqual(harness.appended.map((entry) => entry.data), [{ isActive: true, tier: "T5", ...primary, parent }]);
	assert.deepEqual(harness.statuses.at(-1), { key: "turbo", value: "turbo:T5" });
	assert.match(harness.notifications.at(-1)?.message ?? "", /could not confirm the worker override state/i);
});

test("a concurrent toggle is rejected while the first change is pending", async () => {
	process.env.PI_CODING_AGENT_DIR = settingsRoot({ tierPrimaries: { T5: primary } });
	const harness = runtime();
	harness.setOverrideResponderAvailable(false);

	const first = harness.commands.get("turbo")!.handler("", harness.ctx);
	await Promise.resolve();
	await harness.commands.get("turbo")!.handler("", harness.ctx);
	await first;

	assert.equal(harness.emitted.filter((entry) => entry.event === SESSION_MODEL_OVERRIDE_CHANNEL).length, 2);
	assert.deepEqual(overrideRequest(harness, 1), { clear: true });
	assert.ok(harness.notifications.some((entry) => entry.message === "Turbo is already changing."));
	assert.deepEqual(harness.appended.at(-1), { type: "turbo-state", data: { isActive: true, tier: "T5", ...primary, parent } });
	assert.deepEqual(harness.statuses.at(-1), { key: "turbo", value: "turbo:pending" });
});
