import assert from "node:assert/strict";
import { test } from "node:test";

import { applyEdit, entryAt, isValidModelId, isValidThinking, parseTierFile, slotsOf, tierNames, type TierFile } from "./model.ts";

function sampleFile(): TierFile {
	return {
		tiers: {
			T1: {
				pi: { model: "openai-codex/gpt-5.6-luna", thinking: "low" },
				fallbacks: [
					{ model: "anthropic/claude-haiku-4-5", thinking: "low" },
					{ model: "openai-codex/gpt-5.6-terra", thinking: "minimal" },
				],
				climbOnExhaustion: "T2",
			},
			T2: {
				pi: { model: "anthropic/claude-haiku-4-5", thinking: "medium" },
				fallbacks: [{ model: "openai-codex/gpt-5.6-luna", thinking: "medium" }],
			},
		},
		orchestrator: "T2",
		agents: { "log-summarizer": "T2" },
	};
}

test("isValidModelId: accepts provider/id and rejects a bare word, empty segments, or an empty string", () => {
	assert.equal(isValidModelId("anthropic/claude-opus-5"), true);
	assert.equal(isValidModelId("openrouter/openrouter/free"), true);
	assert.equal(isValidModelId("claude-opus-5"), false);
	assert.equal(isValidModelId("anthropic/"), false);
	assert.equal(isValidModelId("/claude-opus-5"), false);
	assert.equal(isValidModelId(""), false);
	assert.equal(isValidModelId("   "), false);
});

test("isValidThinking: accepts exactly the seven known levels and rejects anything else", () => {
	for (const level of ["off", "minimal", "low", "medium", "high", "xhigh", "max"]) {
		assert.equal(isValidThinking(level), true);
	}
	assert.equal(isValidThinking("extreme"), false);
	assert.equal(isValidThinking(""), false);
	assert.equal(isValidThinking("Medium"), false);
});

test("tierNames: sorted, so T1..T5 always render in order regardless of file key order", () => {
	assert.deepEqual(tierNames(sampleFile()), ["T1", "T2"]);
});

test("slotsOf / entryAt: the primary comes first, then fallbacks in their own order", () => {
	const tier = sampleFile().tiers.T1;
	const slots = slotsOf(tier);
	assert.equal(slots.length, 3);
	assert.deepEqual(slots[0].slot, { kind: "pi" });
	assert.deepEqual(slots[1].slot, { kind: "fallback", index: 0 });
	assert.deepEqual(slots[2].slot, { kind: "fallback", index: 1 });
	assert.equal(entryAt(tier, { kind: "pi" }).model, "openai-codex/gpt-5.6-luna");
	assert.equal(entryAt(tier, { kind: "fallback", index: 1 }).model, "openai-codex/gpt-5.6-terra");
});

test("applyEdit: replaces the primary without touching fallbacks, tier order, or other tiers", () => {
	const file = sampleFile();
	const edited = applyEdit(file, "T1", { kind: "pi" }, "anthropic/claude-opus-5", "high");

	assert.deepEqual(edited.tiers.T1.pi, { model: "anthropic/claude-opus-5", thinking: "high" });
	assert.deepEqual(edited.tiers.T1.fallbacks, file.tiers.T1.fallbacks);
	assert.equal(edited.tiers.T1.climbOnExhaustion, "T2", "unrelated fields on the tier survive the edit");
	assert.deepEqual(edited.tiers.T2, file.tiers.T2, "other tiers are untouched");
	// The original is never mutated — callers can still compare before/after.
	assert.deepEqual(file.tiers.T1.pi, { model: "openai-codex/gpt-5.6-luna", thinking: "low" });
});

test("applyEdit: replaces one fallback by index, preserving every other fallback's position", () => {
	const edited = applyEdit(sampleFile(), "T1", { kind: "fallback", index: 0 }, "anthropic/claude-sonnet-5", "minimal");
	assert.deepEqual(edited.tiers.T1.fallbacks[0], { model: "anthropic/claude-sonnet-5", thinking: "minimal" });
	assert.equal(edited.tiers.T1.fallbacks[1].model, "openai-codex/gpt-5.6-terra", "the second fallback keeps its position");
});

test("applyEdit: an unknown tier throws rather than silently creating one", () => {
	assert.throws(() => applyEdit(sampleFile(), "T9", { kind: "pi" }, "anthropic/claude-opus-5", "high"));
});

test("parseTierFile: accepts a well-formed per-entry {model, thinking} file", () => {
	const parsed = parseTierFile(JSON.stringify(sampleFile()));
	assert.deepEqual(parsed, sampleFile());
});

test("parseTierFile: rejects the older flat-string schema (pi as a string, tier-level thinking) with a message naming the field and the migration", () => {
	const oldSchema = JSON.stringify({
		tiers: { T1: { pi: "openrouter/openrouter/free", fallbacks: ["openai-codex/gpt-5.6-luna"], thinking: "low" } },
		orchestrator: "T1",
		agents: {},
	});
	assert.throws(() => parseTierFile(oldSchema), /T1.*\.pi is not a \{model, thinking\} object/);
});

test("parseTierFile: rejects a fallback entry missing thinking, and a fallbacks field that isn't an array", () => {
	const missingThinking = JSON.stringify({
		tiers: { T1: { pi: { model: "a/b", thinking: "low" }, fallbacks: [{ model: "c/d" }] } },
		orchestrator: "T1",
		agents: {},
	});
	assert.throws(() => parseTierFile(missingThinking), /T1.*\.fallbacks/);

	const notAnArray = JSON.stringify({
		tiers: { T1: { pi: { model: "a/b", thinking: "low" }, fallbacks: { model: "c/d", thinking: "low" } } },
		orchestrator: "T1",
		agents: {},
	});
	assert.throws(() => parseTierFile(notAnArray), /T1.*\.fallbacks/);
});

test("parseTierFile: rejects missing/non-object top-level \"tiers\" and a non-object tier entry", () => {
	assert.throws(() => parseTierFile(JSON.stringify({ orchestrator: "T1", agents: {} })), /missing top-level "tiers"/);
	assert.throws(() => parseTierFile(JSON.stringify({ tiers: "nope", orchestrator: "T1", agents: {} })), /"tiers" is not an object/);
	assert.throws(
		() => parseTierFile(JSON.stringify({ tiers: { T1: "nope" }, orchestrator: "T1", agents: {} })),
		/tier "T1" is not an object/,
	);
});
