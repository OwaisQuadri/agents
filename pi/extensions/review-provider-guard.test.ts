import assert from "node:assert/strict";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

import { blockedReviewDispatch, providersForModelRef, type DispatchContext, type ModelEntry } from "./review-provider-guard/policy.ts";
import { agentDefaultModelFrom, piAgentRoot, readSubagentSettings } from "./review-provider-guard/settings.ts";

const models: ModelEntry[] = [
	{ id: "gpt-5.6-terra", name: "GPT-5.6 Terra", provider: "openai-codex" },
	{ id: "gpt-5.6-sol", name: "GPT-5.6 Sol", provider: "openai-codex" },
	{ id: "claude-opus-5", name: "Claude Opus 5", provider: "anthropic" },
	{ id: "claude-sonnet-5", name: "Claude Sonnet 5", provider: "anthropic" },
];

function context(overrides: Partial<DispatchContext> = {}): DispatchContext {
	return {
		sessionProvider: "openai-codex",
		sessionModelId: "gpt-5.6-terra",
		availableModels: models,
		agentDefaultModel: () => undefined,
		...overrides,
	};
}

test("blocks an explicit same-provider override and quotes both compared models", () => {
	const reason = blockedReviewDispatch("Agent", { subagent_type: "code-reviewer", model: "openai-codex/gpt-5.6-sol" }, context());
	assert.match(reason ?? "", /^Blocked a code-reviewer dispatch/);
	assert.match(reason ?? "", /openai-codex\/gpt-5\.6-sol/);
	assert.match(reason ?? "", /openai-codex\/gpt-5\.6-terra/);
});

test("allows an explicit cross-provider override", () => {
	assert.equal(blockedReviewDispatch("Agent", { subagent_type: "code-reviewer", model: "anthropic/claude-opus-5" }, context()), undefined);
});

test("allows a tier default that already routes to another provider", () => {
	const ctx = context({ agentDefaultModel: () => "anthropic/claude-opus-5" });
	assert.equal(blockedReviewDispatch("Agent", { subagent_type: "code-reviewer" }, ctx), undefined);
});

test("blocks a tier default that lands back on the session provider", () => {
	const ctx = context({ agentDefaultModel: () => "openai-codex/gpt-5.6-terra" });
	assert.match(blockedReviewDispatch("Agent", { subagent_type: "spec-tester" }, ctx) ?? "", /Blocked a spec-tester dispatch/);
});

test("guards exactly the four review and test roles", () => {
	const ctx = context({ agentDefaultModel: () => "openai-codex/gpt-5.6-terra" });
	for (const guarded of ["anchor-verifier", "code-reviewer", "maestro-tester", "spec-tester"]) {
		assert.match(blockedReviewDispatch("Agent", { subagent_type: guarded }, ctx) ?? "", /Blocked/, guarded);
	}
	for (const other of ["debugger", "log-summarizer", "web-research-summarizer", "Explore", "general-purpose"]) {
		assert.equal(blockedReviewDispatch("Agent", { subagent_type: other }, ctx), undefined, other);
	}
});

test("ignores every tool other than Agent", () => {
	const ctx = context({ agentDefaultModel: () => "openai-codex/gpt-5.6-terra" });
	for (const tool of ["bash", "SubagentWorkflow", "get_subagent_result", "write"]) {
		assert.equal(blockedReviewDispatch(tool, { subagent_type: "code-reviewer" }, ctx), undefined, tool);
	}
});

test("allows a reference no available model resolves", () => {
	assert.equal(blockedReviewDispatch("Agent", { subagent_type: "code-reviewer", model: "no-such-model" }, context()), undefined);
	assert.equal(blockedReviewDispatch("Agent", { subagent_type: "code-reviewer", model: "   " }, context()), undefined);
});

test("blocks a fuzzy reference that only resolves to the session provider", () => {
	const ctx = context({ sessionProvider: "anthropic", sessionModelId: "claude-opus-5" });
	assert.match(blockedReviewDispatch("Agent", { subagent_type: "maestro-tester", model: "opus" }, ctx) ?? "", /Blocked/);
});

test("allows a fuzzy reference that spans two providers", () => {
	const ambiguous: ModelEntry[] = [...models, { id: "sol-1", name: "Sol One", provider: "anthropic" }];
	const ctx = context({ availableModels: ambiguous });
	assert.equal(blockedReviewDispatch("Agent", { subagent_type: "code-reviewer", model: "sol" }, ctx), undefined);
});

test("allows every dispatch when the session model is unknown", () => {
	const ctx = context({ sessionProvider: undefined, sessionModelId: undefined, agentDefaultModel: () => "openai-codex/gpt-5.6-terra" });
	assert.equal(blockedReviewDispatch("Agent", { subagent_type: "code-reviewer" }, ctx), undefined);
});

test("ends the reason cleanly when no other provider is available", () => {
	const soleProvider = models.filter((model) => model.provider === "openai-codex");
	const ctx = context({ availableModels: soleProvider, agentDefaultModel: () => "openai-codex/gpt-5.6-terra" });
	const reason = blockedReviewDispatch("Agent", { subagent_type: "anchor-verifier" }, ctx) ?? "";
	assert.match(reason, /^Blocked a anchor-verifier dispatch/);
	assert.doesNotMatch(reason, /e\.g\./);
	assert.match(reason, /change\.$/);
});

test("resolves a model reference to the providers it could land on", () => {
	assert.deepEqual(providersForModelRef("anthropic/claude-opus-5", models), ["anthropic"]);
	assert.deepEqual(providersForModelRef("GPT-5.6-Terra", models), ["openai-codex"]);
	assert.deepEqual(providersForModelRef("no-such-model", models), []);
	assert.deepEqual(providersForModelRef("claude", models).sort(), ["anthropic"]);
	assert.deepEqual(providersForModelRef("5", models).sort(), ["anthropic", "openai-codex"]);
});

test("takes an exact qualified name over another provider's model that merely mentions it", () => {
	const mirrored: ModelEntry[] = [
		{ id: "claude-opus-5", name: "Claude Opus 5", provider: "anthropic" },
		{ id: "mirror-1", name: "anthropic/claude-opus-5 (mirror)", provider: "openrouter" },
	];
	assert.deepEqual(providersForModelRef("anthropic/claude-opus-5", mirrored), ["anthropic"]);

	const ctx = context({ sessionProvider: "anthropic", sessionModelId: "claude-opus-5", availableModels: mirrored });
	assert.match(blockedReviewDispatch("Agent", { subagent_type: "code-reviewer", model: "anthropic/claude-opus-5" }, ctx) ?? "", /Blocked a code-reviewer dispatch/);
});

test("reads the agent default model out of the persisted subagent settings", () => {
	const settings = {
		subagents: {
			defaultModel: "openai-codex/gpt-5.6-terra",
			agentOverrides: { "code-reviewer": { model: "anthropic/claude-opus-5", fallbackModels: ["openai-codex/gpt-5.6-sol"] } },
		},
	};
	assert.equal(agentDefaultModelFrom(settings, "code-reviewer"), "anthropic/claude-opus-5");
	assert.equal(agentDefaultModelFrom(settings, "spec-tester"), "openai-codex/gpt-5.6-terra");
	assert.equal(agentDefaultModelFrom({}, "code-reviewer"), undefined);
	assert.equal(agentDefaultModelFrom(null, "code-reviewer"), undefined);
	assert.equal(agentDefaultModelFrom("not-an-object", "code-reviewer"), undefined);
	assert.equal(agentDefaultModelFrom({ subagents: { agentOverrides: { "code-reviewer": {} } } }, "code-reviewer"), undefined);
});

test("degrades to empty settings on a missing or corrupt file", () => {
	const missingRoot = mkdtempSync(join(tmpdir(), "review-provider-guard-missing-"));
	assert.deepEqual(readSubagentSettings(missingRoot), {});

	const corruptRoot = mkdtempSync(join(tmpdir(), "review-provider-guard-corrupt-"));
	writeFileSync(join(corruptRoot, "settings.json"), "{ not json", "utf8");
	assert.deepEqual(readSubagentSettings(corruptRoot), {});

	const goodRoot = mkdtempSync(join(tmpdir(), "review-provider-guard-good-"));
	writeFileSync(join(goodRoot, "settings.json"), JSON.stringify({ subagents: { defaultModel: "anthropic/claude-opus-5" } }), "utf8");
	assert.equal(agentDefaultModelFrom(readSubagentSettings(goodRoot), "spec-tester"), "anthropic/claude-opus-5");
});

test("prefers the configured pi agent directory over the home default", () => {
	assert.equal(piAgentRoot({ PI_CODING_AGENT_DIR: "/tmp/review-provider-guard-root" }), "/tmp/review-provider-guard-root");
	assert.match(piAgentRoot({ PI_CODING_AGENT_DIR: "  " }), /\.pi\/agent$/);
	assert.match(piAgentRoot({}), /\.pi\/agent$/);
});
