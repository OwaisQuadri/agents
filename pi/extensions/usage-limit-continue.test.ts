import assert from "node:assert/strict";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import {
	computeResetPlan,
	detectUsageLimitSignal,
	earliestAvailable,
	fallbackWindowResetMs,
	findTierFallback,
	isLocalModel,
	parseResetFromHeaders,
	parseResetFromText,
	scheduleResume,
} from "./usage-limit-continue.ts";

// SYNTHETIC FIXTURE, not configuration. These provider and model names are invented so a
// reader never mistakes this block for the routing table. The real one is compiled from
// config/model-tiers.json into pi settings, and the last test in this file checks that the
// real file resolves rather than restating it here.
const SYNTHETIC_FALLBACKS = {
	"provider-a/small": "provider-b/small",
	"provider-a/medium": "provider-b/medium",
	"provider-a/large": "provider-a/medium",
};

const NOW = new Date("2026-08-19T12:00:00.000-04:00").getTime();

function anthropicModel() {
	return { provider: "anthropic", baseUrl: "https://api.anthropic.com", cost: { input: 3, output: 15, cacheRead: 0.3 } };
}

function localModel(overrides: Partial<{ provider: string; baseUrl: string }> = {}) {
	return { provider: "ollama", baseUrl: "http://localhost:11434", cost: { input: 0, output: 0, cacheRead: 0 }, ...overrides };
}

test("isLocalModel: recognizes known local providers and hosts, and leaves paid providers alone", () => {
	assert.equal(isLocalModel(localModel()), true);
	assert.equal(isLocalModel({ provider: "custom-lmstudio", baseUrl: "http://127.0.0.1:1234", cost: { input: 0, output: 0, cacheRead: 0 } }), true);
	assert.equal(isLocalModel(anthropicModel()), false);
	assert.equal(isLocalModel(undefined), false);
});

test("detectUsageLimitSignal: matches known usage-limit phrasing, ignores unrelated errors", () => {
	assert.equal(detectUsageLimitSignal("You've hit your usage limit. It resets at 3:00pm."), true);
	assert.equal(detectUsageLimitSignal("Weekly limit reached, resets in 2 days."), true);
	assert.equal(detectUsageLimitSignal("429 Too Many Requests"), true);
	assert.equal(detectUsageLimitSignal("connection refused"), false);
});

test("parseResetFromHeaders: retry-after in seconds", () => {
	const resetMs = parseResetFromHeaders({ "retry-after": "120" }, NOW);
	assert.equal(resetMs, NOW + 120_000);
});

test("parseResetFromHeaders: retry-after as an HTTP date", () => {
	const httpDate = new Date(NOW + 60_000).toUTCString();
	const resetMs = parseResetFromHeaders({ "retry-after": httpDate }, NOW);
	assert.equal(resetMs, Date.parse(httpDate));
});

test("parseResetFromHeaders: anthropic-style *-reset headers as ISO timestamps, takes the latest", () => {
	const earlier = new Date(NOW + 60_000).toISOString();
	const later = new Date(NOW + 3_600_000).toISOString();
	const resetMs = parseResetFromHeaders(
		{
			"anthropic-ratelimit-requests-reset": earlier,
			"anthropic-ratelimit-tokens-reset": later,
		},
		NOW,
	);
	assert.equal(resetMs, Date.parse(later));
});

test("parseResetFromHeaders: openai-style duration reset headers ('6m0s')", () => {
	const resetMs = parseResetFromHeaders({ "x-ratelimit-reset-requests": "6m0s" }, NOW);
	assert.equal(resetMs, NOW + 6 * 60_000);
});

test("parseResetFromHeaders: no recognizable header returns null", () => {
	assert.equal(parseResetFromHeaders({ "content-type": "application/json" }, NOW), null);
});

test("parseResetFromText: 'resets in Xh Ym' duration", () => {
	const resetMs = parseResetFromText("Weekly limit reached. Resets in 2h 30m.", NOW);
	assert.equal(resetMs, NOW + (2 * 60 + 30) * 60_000);
});

test("parseResetFromText: 'try again in N seconds' duration", () => {
	const resetMs = parseResetFromText("Rate limited. Try again in 45 seconds.", NOW);
	assert.equal(resetMs, NOW + 45_000);
});

test("parseResetFromText: 'resets at H:MMam/pm' clock time, same day when still ahead", () => {
	const resetMs = parseResetFromText("5-hour limit reached, resets at 3:00pm.", NOW);
	const expected = new Date(NOW);
	expected.setHours(15, 0, 0, 0);
	assert.equal(resetMs, expected.getTime());
});

test("parseResetFromText: 'resets at H:MMam/pm' rolls to tomorrow when the clock time has passed", () => {
	const resetMs = parseResetFromText("resets at 9:00am", NOW);
	const expected = new Date(NOW);
	expected.setDate(expected.getDate() + 1);
	expected.setHours(9, 0, 0, 0);
	assert.equal(resetMs, expected.getTime());
});

test("parseResetFromText: an embedded ISO timestamp wins over phrasing", () => {
	const iso = new Date(NOW + 7_200_000).toISOString();
	const resetMs = parseResetFromText(`Limit resets at ${iso}`, NOW);
	assert.equal(resetMs, Date.parse(iso));
});

test("parseResetFromText: no parseable time returns null", () => {
	assert.equal(parseResetFromText("You've hit your usage limit.", NOW), null);
});

test("fallbackWindowResetMs: defaults to the 5-hour session window when no instant is stated", () => {
	assert.equal(fallbackWindowResetMs("5-hour limit reached.", NOW), NOW + 5 * 60 * 60 * 1000);
});

test("fallbackWindowResetMs: defaults to the 7-day weekly window when no instant is stated", () => {
	assert.equal(fallbackWindowResetMs("You've hit your weekly limit.", NOW), NOW + 7 * 24 * 60 * 60 * 1000);
});

test("fallbackWindowResetMs: neither window named returns null", () => {
	assert.equal(fallbackWindowResetMs("usage limit reached", NOW), null);
});

test("computeResetPlan: an unparseable instant falls back to the named 5-hour or 7-day window", () => {
	const plan = computeResetPlan({ status: 429, headers: {}, text: "5-hour limit reached", model: anthropicModel(), nowMs: NOW });
	assert.deepEqual(plan, { isDetected: true, resetAtMs: NOW + 5 * 60 * 60 * 1000, matchedText: "5-hour limit reached" });
});

test("computeResetPlan: skips local models entirely, even on a 429", () => {
	const plan = computeResetPlan({ status: 429, headers: { "retry-after": "60" }, text: "rate limited", model: localModel(), nowMs: NOW });
	assert.equal(plan, null);
});

test("computeResetPlan: a bare 429 with no usage-limit text still schedules off headers", () => {
	const plan = computeResetPlan({ status: 429, headers: { "retry-after": "30" }, text: "", model: anthropicModel(), nowMs: NOW });
	assert.deepEqual(plan, { isDetected: true, resetAtMs: NOW + 30_000, matchedText: "" });
});

test("computeResetPlan: usage-limit text with no 429 status still detects, from message_end alone", () => {
	const plan = computeResetPlan({
		status: 0,
		headers: {},
		text: "5-hour limit reached, resets at 3:00pm.",
		model: anthropicModel(),
		nowMs: NOW,
	});
	assert.equal(plan?.isDetected, true);
	assert.notEqual(plan?.resetAtMs, null);
});

test("computeResetPlan: neither a 429 nor usage-limit text returns null", () => {
	const plan = computeResetPlan({ status: 500, headers: {}, text: "internal server error", model: anthropicModel(), nowMs: NOW });
	assert.equal(plan, null);
});

test("computeResetPlan: detected but unparseable time reports isDetected with a null resetAtMs", () => {
	const plan = computeResetPlan({ status: 429, headers: {}, text: "usage limit reached", model: anthropicModel(), nowMs: NOW });
	assert.deepEqual(plan, { isDetected: true, resetAtMs: null, matchedText: "usage limit reached" });
});

test("findTierFallback: fable falls back to opus, and every other tier resolves its own backup", () => {
	assert.equal(findTierFallback(SYNTHETIC_FALLBACKS, "provider-a", "large"), "provider-a/medium");
	assert.equal(findTierFallback(SYNTHETIC_FALLBACKS, "provider-a", "medium"), "provider-b/medium");
	assert.equal(findTierFallback(SYNTHETIC_FALLBACKS, "provider-a", "small"), "provider-b/small");
});

test("findTierFallback: a model outside the tier file has no fallback, so the resume path still owns it", () => {
	assert.equal(findTierFallback(SYNTHETIC_FALLBACKS, "provider-b", "medium"), null);
	assert.equal(findTierFallback(SYNTHETIC_FALLBACKS, "ollama", "llama-4"), null);
	assert.equal(findTierFallback({}, "provider-a", "large"), null);
});

test("earliestAvailable: the resume waits on the model that returns first, not the one that failed last", () => {
	const soonest = earliestAvailable({
		"anthropic/claude-fable-5": NOW + 20 * 60_000,
		"anthropic/claude-opus-5": NOW + 2 * 3_600_000,
		"openai-codex/gpt-5.6-sol": NOW + 4 * 3_600_000,
	});
	assert.deepEqual(soonest, { model: "anthropic/claude-fable-5", resetAtMs: NOW + 20 * 60_000 });
});

test("earliestAvailable: nothing abandoned yields null, so the old single-reset path still owns it", () => {
	assert.equal(earliestAvailable({}), null);
	assert.deepEqual(earliestAvailable({ "anthropic/claude-opus-5": NOW }), {
		model: "anthropic/claude-opus-5",
		resetAtMs: NOW,
	});
});

test("scheduleResume: writes a pending-job record and refuses a duplicate schedule for the same session", async () => {
	// A resetAt an hour out so the detached `sleep && pi --session ...` job this spawns
	// never fires while this test (or its tmp dir cleanup) is running.
	const FAR_FUTURE_MS = 3_600_000;
	const stateHome = await mkdtemp(join(tmpdir(), "usage-limit-continue-"));
	const originalHome = process.env.HOME;
	process.env.HOME = stateHome;
	try {
		const sessionFile = join(stateHome, "session.jsonl");
		const first = await scheduleResume(sessionFile, Date.now() + FAR_FUTURE_MS, Date.now());
		assert.notEqual(first, null);

		const stateFile = join(stateHome, ".pi", "agent", "usage-limit-continue", "pending.json");
		const stored = JSON.parse(await readFile(stateFile, "utf8"));
		assert.equal(stored[sessionFile].sessionFile, sessionFile);

		const second = await scheduleResume(sessionFile, Date.now() + FAR_FUTURE_MS + 1000, Date.now());
		assert.equal(second, null);
	} finally {
		process.env.HOME = originalHome;
		await rm(stateHome, { recursive: true, force: true });
	}
});

test("the real tier file compiles into a chain every hop of which resolves", async () => {
	const tiersPath = join(import.meta.dirname, "..", "..", "config", "model-tiers.json");
	const tiers = JSON.parse(await readFile(tiersPath, "utf8")) as {
		tiers: Record<string, { pi: string; fallbacks: string[] }>;
		orchestrator: string;
		agents: Record<string, string>;
	};

	// What install.sh compiles into settings.modelTierFallbacks: each tier's first backup.
	const compiled: Record<string, string> = {};
	for (const tier of Object.values(tiers.tiers)) {
		compiled[tier.pi] = tier.fallbacks[0];
	}

	// Every tier names at least one backup, and never itself.
	for (const [name, tier] of Object.entries(tiers.tiers)) {
		assert.ok(tier.fallbacks.length >= 1, `${name} has no fallback`);
		assert.ok(!tier.fallbacks.includes(tier.pi), `${name} falls back to itself`);
	}

	// Every agent's tier exists, so no assignment routes into a hole.
	for (const [agent, tier] of Object.entries(tiers.agents)) {
		assert.ok(tiers.tiers[tier], `${agent} names tier ${tier}, which does not exist`);
	}
	assert.ok(tiers.tiers[tiers.orchestrator], "orchestrator names a tier that does not exist");

	// The chain terminates: walking first-backups from any tier primary must stop rather
	// than cycle, which is what lets the session fall through to a scheduled resume.
	for (const start of Object.keys(compiled)) {
		const seen = new Set<string>();
		let at: string | undefined = start;
		while (at && compiled[at]) {
			assert.ok(!seen.has(at), `fallback chain cycles at ${at}`);
			seen.add(at);
			at = compiled[at];
		}
	}
});
