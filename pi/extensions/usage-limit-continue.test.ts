import assert from "node:assert/strict";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import {
	computeResetPlan,
	detectUsageLimitSignal,
	earliestAvailable,
	fallbackWindowResetMs,
	findTierFallback,
	handleMessageEnd,
	isLocalModel,
	parseResetFromHeaders,
	parseResetFromText,
	resolveHomeTier,
	scheduleResume,
} from "./usage-limit-continue.ts";

// The real map is compiled from config/model-tiers.json into pi settings; the last test
// in this file checks that file rather than restating it here.
const SYNTHETIC_FALLBACKS = {
	"provider-a/small": { model: "provider-b/small", thinking: "low" as const },
	"provider-a/medium": { model: "provider-b/medium", thinking: "medium" as const },
	"provider-a/large": { model: "provider-a/medium", thinking: "high" as const },
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
	assert.deepEqual(findTierFallback(SYNTHETIC_FALLBACKS, "provider-a", "large"), { model: "provider-a/medium", thinking: "high" });
	assert.deepEqual(findTierFallback(SYNTHETIC_FALLBACKS, "provider-a", "medium"), { model: "provider-b/medium", thinking: "medium" });
	assert.deepEqual(findTierFallback(SYNTHETIC_FALLBACKS, "provider-a", "small"), { model: "provider-b/small", thinking: "low" });
});

test("findTierFallback: a model outside the tier file has no fallback, so the resume path still owns it", () => {
	assert.equal(findTierFallback(SYNTHETIC_FALLBACKS, "provider-b", "medium"), null);
	assert.equal(findTierFallback(SYNTHETIC_FALLBACKS, "ollama", "llama-4"), null);
	assert.equal(findTierFallback({}, "provider-a", "large"), null);
});

const SYNTHETIC_PRIMARIES = {
	T1: { model: "provider-a/small", thinking: "low" as const },
	T2: { model: "provider-a/large", thinking: "medium" as const },
};

test("resolveHomeTier: a tier's own primary resolves to that tier", () => {
	assert.equal(resolveHomeTier(SYNTHETIC_PRIMARIES, "provider-a", "small"), "T1");
	assert.equal(resolveHomeTier(SYNTHETIC_PRIMARIES, "provider-a", "large"), "T2");
});

test("resolveHomeTier: a model that is no tier's primary (a mid-chain fallback, or untiered) resolves to null", () => {
	assert.equal(resolveHomeTier(SYNTHETIC_PRIMARIES, "provider-b", "small"), null);
	assert.equal(resolveHomeTier({}, "provider-a", "small"), null);
});

test("resolveHomeTier: two tiers naming the same primary resolves deterministically to the first tier in sorted order", () => {
	const dup = {
		T3: { model: "provider-a/small", thinking: "low" as const },
		T1: { model: "provider-a/small", thinking: "low" as const },
	};
	assert.equal(resolveHomeTier(dup, "provider-a", "small"), "T1");
});

test("earliestAvailable: the resume waits on the model that returns first, not the one that failed last", () => {
	const soonest = earliestAvailable({
		"anthropic/claude-fable-5": { resetAtMs: NOW + 20 * 60_000, thinking: "medium" },
		"anthropic/claude-opus-5": { resetAtMs: NOW + 2 * 3_600_000, thinking: "high" },
		"openai-codex/gpt-5.6-sol": { resetAtMs: NOW + 4 * 3_600_000, thinking: "high" },
	});
	assert.deepEqual(soonest, { model: "anthropic/claude-fable-5", resetAtMs: NOW + 20 * 60_000, thinking: "medium" });
});

test("earliestAvailable: nothing abandoned yields null, so the old single-reset path still owns it", () => {
	assert.equal(earliestAvailable({}), null);
	assert.deepEqual(earliestAvailable({ "anthropic/claude-opus-5": { resetAtMs: NOW, thinking: "high" } }), {
		model: "anthropic/claude-opus-5",
		resetAtMs: NOW,
		thinking: "high",
	});
});

test("earliestAvailable: expired reset times do not schedule a resume in the past", () => {
	assert.deepEqual(
		earliestAvailable(
			{
				"anthropic/claude-fable-5": { resetAtMs: NOW - 1, thinking: "medium" },
				"anthropic/claude-opus-5": { resetAtMs: NOW + 1, thinking: "high" },
			},
			NOW,
		),
		{ model: "anthropic/claude-opus-5", resetAtMs: NOW + 1, thinking: "high" },
	);
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
		const pastSessionFile = join(stateHome, "past-session.jsonl");
		const nowMs = Date.now();
		const immediate = await scheduleResume(pastSessionFile, nowMs - 1, nowMs);
		assert.ok(immediate && immediate.resetAtMs > nowMs);
		assert.equal(await scheduleResume(pastSessionFile, nowMs - 1, nowMs), null);
		assert.equal(await scheduleResume(pastSessionFile, nowMs - 1, nowMs + 2000), null);
		const first = await scheduleResume(sessionFile, Date.now() + FAR_FUTURE_MS, Date.now());
		assert.notEqual(first, null);

		const stateFile = join(stateHome, ".pi", "agent", "usage-limit-continue", "pending.json");
		const stored = JSON.parse(await readFile(stateFile, "utf8"));
		assert.equal(stored[sessionFile].sessionFile, sessionFile);

		const second = await scheduleResume(sessionFile, Date.now() + FAR_FUTURE_MS + 1000, Date.now());
		assert.equal(second, null);
		for (const pid of [immediate.pid, first?.pid]) {
			if (!pid || pid <= 0) {
				continue;
			}
			try {
				process.kill(pid);
			} catch (error) {
				if ((error as NodeJS.ErrnoException).code !== "ESRCH") {
					throw error;
				}
			}
		}
	} finally {
		process.env.HOME = originalHome;
		await rm(stateHome, { recursive: true, force: true });
	}
});

test("the real tier file compiles into a chain every hop of which resolves, one map per tier", async () => {
	const tiersPath = join(import.meta.dirname, "..", "..", "config", "model-tiers.json");
	type Entry = { model: string; thinking: string };
	const tiers = JSON.parse(await readFile(tiersPath, "utf8")) as {
		tiers: Record<string, { pi: Entry; fallbacks: Entry[]; climbOnExhaustion?: string }>;
		orchestrator: string;
		agents: Record<string, string>;
	};

	// What install.sh compiles into settings.modelTierFallbacks: one map PER TIER, so a model
	// shared by two tiers' chains (a real case now that thinking travels per model) never lets
	// one tier's mapping overwrite the other's.
	const compiled: Record<string, Record<string, string>> = {};
	const primaries: Record<string, string> = {};
	for (const [name, tier] of Object.entries(tiers.tiers)) {
		const chain = [tier.pi, ...tier.fallbacks];
		const hops: Record<string, string> = {};
		for (let i = 0; i < chain.length - 1; i++) {
			hops[chain[i].model] = chain[i + 1].model;
		}
		compiled[name] = hops;
		primaries[name] = tier.pi.model;
	}

	for (const [name, tier] of Object.entries(tiers.tiers)) {
		assert.ok(tier.fallbacks.length >= 1, `${name} has no fallback`);
		assert.ok(
			!tier.fallbacks.some((f) => f.model === tier.pi.model),
			`${name} falls back to itself`,
		);
	}

	// Every tier's own primary must be unique, or resolveHomeTier cannot tell which tier a
	// session starting on that model is meant to walk.
	const primaryOwners = new Map<string, string[]>();
	for (const [name, model] of Object.entries(primaries)) {
		primaryOwners.set(model, [...(primaryOwners.get(model) ?? []), name]);
	}
	for (const [model, owners] of primaryOwners) {
		assert.equal(owners.length, 1, `${model} is the primary of more than one tier: ${owners.join(", ")}`);
	}

	for (const [agent, tier] of Object.entries(tiers.agents)) {
		assert.ok(tiers.tiers[tier], `${agent} names tier ${tier}, which does not exist`);
	}

	for (const [name, tier] of Object.entries(tiers.tiers)) {
		if (tier.climbOnExhaustion) {
			assert.ok(tiers.tiers[tier.climbOnExhaustion], `${name} climbs to ${tier.climbOnExhaustion}, which does not exist`);
			assert.notEqual(tier.climbOnExhaustion, name, `${name} climbs to itself`);
		}
	}
	assert.ok(tiers.tiers[tiers.orchestrator], "orchestrator names a tier that does not exist");

	// A cycle here would strand the session in the fallback walk instead of letting it
	// reach the scheduled resume. Each tier's own chain is checked in isolation, matching how
	// applyTierFallback only ever walks within the one tier it resolved at session start.
	for (const [name, hops] of Object.entries(compiled)) {
		const seen = new Set<string>();
		let at: string | undefined = primaries[name];
		while (at && hops[at]) {
			assert.ok(!seen.has(at), `${name}'s fallback chain cycles at ${at}`);
			seen.add(at);
			at = hops[at];
		}
	}
});

test("handleMessageEnd: a print-mode worker retries the same turn on the fallback model instead of staying on the stale error", async () => {
	const stateHome = await mkdtemp(join(tmpdir(), "usage-limit-continue-"));
	const originalHome = process.env.HOME;
	process.env.HOME = stateHome;
	try {
		const settingsDir = join(stateHome, ".pi", "agent");
		await mkdir(settingsDir, { recursive: true });
		await writeFile(
			join(settingsDir, "settings.json"),
			JSON.stringify({
				modelTierFallbacks: { T1: { "provider-a/small": { model: "provider-b/small", thinking: "low" } } },
				tierPrimaries: { T1: { model: "provider-a/small", thinking: "low" } },
			}),
		);

		const fallbackModel = { provider: "provider-b", id: "small" };
		const sentMessages: { content: unknown; options: unknown }[] = [];
		const notifications: string[] = [];
		const thinkingLevels: string[] = [];
		const pi = {
			setModel: async () => true,
			setThinkingLevel: (level: string) => {
				thinkingLevels.push(level);
			},
			getThinkingLevel: () => "low",
			sendUserMessage: (content: unknown, options: unknown) => {
				sentMessages.push({ content, options });
			},
		};
		const ctx = {
			mode: "print",
			model: { provider: "provider-a", id: "small", cost: { input: 0, output: 0, cacheRead: 0 } },
			modelRegistry: { find: () => fallbackModel },
			ui: { notify: (message: string) => notifications.push(message) },
			sessionManager: { getSessionFile: () => null },
		};
		const message = { role: "assistant", stopReason: "error", errorMessage: "Codex error: The usage limit has been reached" };

		// eslint-disable-next-line @typescript-eslint/no-explicit-any
		await handleMessageEnd(message as any, ctx as any, pi as any);

		assert.equal(sentMessages.length, 1, "print mode must retry the turn once the fallback model is active");
		assert.deepEqual(sentMessages[0].options, { deliverAs: "followUp" });
		assert.ok(notifications.some((n) => n.includes("provider-b/small")), "notifies which fallback took over");
		assert.deepEqual(thinkingLevels, ["low"], "the hop also applies the fallback's own thinking level");
	} finally {
		process.env.HOME = originalHome;
		await rm(stateHome, { recursive: true, force: true });
	}
});

test("handleMessageEnd: an interactive session switches models but leaves the retry to the human", async () => {
	const stateHome = await mkdtemp(join(tmpdir(), "usage-limit-continue-"));
	const originalHome = process.env.HOME;
	process.env.HOME = stateHome;
	try {
		const settingsDir = join(stateHome, ".pi", "agent");
		await mkdir(settingsDir, { recursive: true });
		await writeFile(
			join(settingsDir, "settings.json"),
			JSON.stringify({
				modelTierFallbacks: { T1: { "provider-a/small": { model: "provider-b/small", thinking: "low" } } },
				tierPrimaries: { T1: { model: "provider-a/small", thinking: "low" } },
			}),
		);

		const fallbackModel = { provider: "provider-b", id: "small" };
		const sentMessages: unknown[] = [];
		const pi = {
			setModel: async () => true,
			setThinkingLevel: () => {},
			getThinkingLevel: () => "low",
			sendUserMessage: (content: unknown) => {
				sentMessages.push(content);
			},
		};
		const ctx = {
			mode: "tui",
			model: { provider: "provider-a", id: "small", cost: { input: 0, output: 0, cacheRead: 0 } },
			modelRegistry: { find: () => fallbackModel },
			ui: { notify: () => {} },
			sessionManager: { getSessionFile: () => null },
		};
		const message = { role: "assistant", stopReason: "error", errorMessage: "usage limit reached" };

		// eslint-disable-next-line @typescript-eslint/no-explicit-any
		await handleMessageEnd(message as any, ctx as any, pi as any);

		assert.equal(sentMessages.length, 0, "a tui session decides for itself whether to continue");
	} finally {
		process.env.HOME = originalHome;
		await rm(stateHome, { recursive: true, force: true });
	}
});

test("handleMessageEnd: a manual switch to another tier primary rehomes unless the fallback selected it", async () => {
	const stateHome = await mkdtemp(join(tmpdir(), "usage-limit-continue-"));
	const originalHome = process.env.HOME;
	process.env.HOME = stateHome;
	try {
		const settingsDir = join(stateHome, ".pi", "agent");
		await mkdir(settingsDir, { recursive: true });
		await writeFile(
			join(settingsDir, "settings.json"),
			JSON.stringify({
				modelTierFallbacks: {
					T1: {
						"provider-a/start": { model: "provider-b/middle", thinking: "low" },
						"provider-b/middle": { model: "provider-c/shared", thinking: "low" },
					},
					T2: {},
					T4: { "provider-c/shared": { model: "provider-d/right", thinking: "medium" } },
				},
				tierPrimaries: {
					T1: { model: "provider-a/start", thinking: "low" },
					T2: { model: "provider-e/lower", thinking: "low" },
					T4: { model: "provider-c/shared", thinking: "medium" },
				},
				tierClimb: { T1: "T2" },
			}),
		);

		const switches: string[] = [];
		const state = { model: "provider-a/start" };
		const sessionManager = { getSessionFile: () => undefined };
		const context = () => {
			const [provider, ...id] = state.model.split("/");
			return {
				mode: "tui",
				model: { provider, id: id.join("/"), cost: { input: 0, output: 0, cacheRead: 0 } },
				modelRegistry: {
					find: (nextProvider: string, nextId: string) => ({
						provider: nextProvider,
						id: nextId,
						cost: { input: 0, output: 0, cacheRead: 0 },
					}),
				},
				ui: { notify: () => {} },
				sessionManager,
			};
		};
		const pi = {
			setModel: async (model: { provider: string; id: string }) => {
				state.model = `${model.provider}/${model.id}`;
				switches.push(state.model);
				return true;
			},
			setThinkingLevel: () => {},
			getThinkingLevel: () => "low",
			sendUserMessage: () => {},
		};
		const error = { role: "assistant", stopReason: "error", errorMessage: "usage limit reached" };

		await handleMessageEnd(error, context() as never, pi as never);
		await handleMessageEnd({ role: "assistant", stopReason: "stop" }, context() as never, pi as never);
		state.model = "provider-c/shared";
		await handleMessageEnd(error, context() as never, pi as never);
		assert.deepEqual(switches, ["provider-b/middle", "provider-d/right"]);

		state.model = "provider-a/start";
		await handleMessageEnd({ role: "assistant", stopReason: "stop" }, context() as never, pi as never);
		await handleMessageEnd(error, context() as never, pi as never);
		await handleMessageEnd(error, context() as never, pi as never);
		await handleMessageEnd(error, context() as never, pi as never);
		assert.equal(switches.at(-1), "provider-e/lower");
	} finally {
		process.env.HOME = originalHome;
		await rm(stateHome, { recursive: true, force: true });
	}
});

test("handleMessageEnd: recreated contexts preserve live limits across a tier climb", async () => {
	const stateHome = await mkdtemp(join(tmpdir(), "usage-limit-continue-"));
	const originalHome = process.env.HOME;
	process.env.HOME = stateHome;
	try {
		const settingsDir = join(stateHome, ".pi", "agent");
		await mkdir(settingsDir, { recursive: true });
		await writeFile(
			join(settingsDir, "settings.json"),
			JSON.stringify({
				modelTierFallbacks: {
					T5: {
						"provider-a/new": { model: "provider-b/sol", thinking: "high" },
						"provider-b/sol": { model: "provider-a/old", thinking: "medium" },
					},
					T4: {
						"provider-a/opus": { model: "provider-b/sol", thinking: "medium" },
						"provider-b/sol": { model: "provider-a/opus", thinking: "medium" },
					},
				},
				tierPrimaries: {
					T5: { model: "provider-a/new", thinking: "medium" },
					T4: { model: "provider-a/opus", thinking: "medium" },
				},
				tierClimb: { T5: "T4" },
			}),
		);

		const switches: string[] = [];
		const ctx = {
			mode: "tui",
			model: { provider: "provider-a", id: "new", cost: { input: 0, output: 0, cacheRead: 0 } },
			modelRegistry: {
				find: (provider: string, id: string) => ({ provider, id, cost: { input: 0, output: 0, cacheRead: 0 } }),
			},
			ui: { notify: () => {} },
			sessionManager: { getSessionFile: () => null },
		};
		const pi = {
			setModel: async (model: { provider: string; id: string }) => {
				ctx.model = { ...model, cost: { input: 0, output: 0, cacheRead: 0 } };
				switches.push(`${model.provider}/${model.id}`);
				return true;
			},
			setThinkingLevel: () => {},
			getThinkingLevel: () => "medium",
			sendUserMessage: () => {},
		};
		const error = { role: "assistant", stopReason: "error", errorMessage: "usage limit reached; resets in 1s" };
		const eventContext = () => ({ ...ctx, sessionManager: ctx.sessionManager });

		for (let index = 0; index < 4; index += 1) {
			await handleMessageEnd(error, eventContext() as never, pi as never);
			await handleMessageEnd({ role: "user" }, eventContext() as never, pi as never);
			await handleMessageEnd({ role: "toolResult" }, eventContext() as never, pi as never);
			if (index === 0) {
				await handleMessageEnd({ role: "assistant", stopReason: "aborted" }, eventContext() as never, pi as never);
				await handleMessageEnd({ role: "assistant", stopReason: "toolUse" }, eventContext() as never, pi as never);
			}
			if (index === 1) {
				await handleMessageEnd({ role: "assistant", stopReason: "error" }, eventContext() as never, pi as never);
			}
		}

		assert.deepEqual(switches, ["provider-b/sol", "provider-a/old", "provider-a/opus"]);

		ctx.model = { provider: "provider-a", id: "new", cost: { input: 0, output: 0, cacheRead: 0 } };
		await handleMessageEnd(error, eventContext() as never, pi as never);
		assert.equal(switches.length, 3);

		await new Promise((resolve) => setTimeout(resolve, 1100));
		await handleMessageEnd(error, eventContext() as never, pi as never);
		assert.equal(switches.at(-1), "provider-b/sol");

		await handleMessageEnd({ role: "assistant", stopReason: "stop" }, eventContext() as never, pi as never);
		ctx.model = { provider: "provider-b", id: "sol", cost: { input: 0, output: 0, cacheRead: 0 } };
		await handleMessageEnd(error, eventContext() as never, pi as never);
		assert.equal(switches.at(-1), "provider-a/old");

		await handleMessageEnd({ role: "assistant", stopReason: "stop" }, eventContext() as never, pi as never);
		ctx.model = { provider: "provider-c", id: "manual", cost: { input: 0, output: 0, cacheRead: 0 } };
		const switchCount = switches.length;
		await handleMessageEnd(error, eventContext() as never, pi as never);
		assert.equal(switches.length, switchCount);
	} finally {
		process.env.HOME = originalHome;
		await rm(stateHome, { recursive: true, force: true });
	}
});

test("handleMessageEnd: a tier whose own chain runs out climbs to climbOnExhaustion's primary, T1-rises-to-T2 style", async () => {
	const stateHome = await mkdtemp(join(tmpdir(), "usage-limit-continue-"));
	const originalHome = process.env.HOME;
	process.env.HOME = stateHome;
	try {
		const settingsDir = join(stateHome, ".pi", "agent");
		await mkdir(settingsDir, { recursive: true });
		await writeFile(
			join(settingsDir, "settings.json"),
			JSON.stringify({
				// T1's own chain has no next hop for its primary (single-entry, already exhausted).
				modelTierFallbacks: { T1: {}, T2: { "provider-b/big": { model: "provider-c/big", thinking: "medium" } } },
				tierPrimaries: {
					T1: { model: "provider-a/small", thinking: "low" },
					T2: { model: "provider-b/big", thinking: "medium" },
				},
				tierClimb: { T1: "T2" },
			}),
		);

		const climbedModel = { provider: "provider-b", id: "big" };
		const notifications: string[] = [];
		const thinkingLevels: string[] = [];
		const pi = {
			setModel: async () => true,
			setThinkingLevel: (level: string) => {
				thinkingLevels.push(level);
			},
			getThinkingLevel: () => "low",
			sendUserMessage: () => {},
		};
		const ctx = {
			mode: "tui",
			model: { provider: "provider-a", id: "small", cost: { input: 0, output: 0, cacheRead: 0 } },
			modelRegistry: { find: () => climbedModel },
			ui: { notify: (message: string) => notifications.push(message) },
			sessionManager: { getSessionFile: () => null },
		};
		const message = { role: "assistant", stopReason: "error", errorMessage: "usage limit reached" };

		// eslint-disable-next-line @typescript-eslint/no-explicit-any
		await handleMessageEnd(message as any, ctx as any, pi as any);

		assert.ok(notifications.some((n) => n.includes("provider-b/big")), "climbs to T2's own primary, not a dead end");
		assert.deepEqual(thinkingLevels, ["medium"], "lands on T2's own thinking level, not T1's");
	} finally {
		process.env.HOME = originalHome;
		await rm(stateHome, { recursive: true, force: true });
	}
});
