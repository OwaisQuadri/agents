import { GUARDED_AGENT_TYPES } from "../review-provider-guard/policy.ts";

export const REVIEW_AGENT_TYPES = GUARDED_AGENT_TYPES;
export const SESSION_MODEL_OVERRIDE_CHANNEL = "subagents:rpc:model_override";

export type ThinkingLevel = "off" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max";
export type TierPrimary = { model: string; thinking: ThinkingLevel };
export type ActiveTurboState = {
	isActive: true;
	tier: "T5";
	model: string;
	thinking: ThinkingLevel;
	parent: TierPrimary;
};
export type TurboState = ActiveTurboState | { isActive: false };

const THINKING_LEVELS: readonly ThinkingLevel[] = ["off", "minimal", "low", "medium", "high", "xhigh", "max"];

function record(value: unknown): Record<string, unknown> | undefined {
	return typeof value === "object" && value !== null && !Array.isArray(value) ? (value as Record<string, unknown>) : undefined;
}

function primaryFrom(value: unknown): TierPrimary | undefined {
	const entry = record(value);
	if (entry === undefined || typeof entry.model !== "string" || !isThinkingLevel(entry.thinking) || parseModelReference(entry.model) === undefined) {
		return undefined;
	}
	return { model: entry.model, thinking: entry.thinking };
}

function isThinkingLevel(value: unknown): value is ThinkingLevel {
	return typeof value === "string" && THINKING_LEVELS.includes(value as ThinkingLevel);
}

function turboStateFrom(value: unknown): TurboState | undefined {
	const state = record(value);
	if (state?.isActive === false) return { isActive: false };
	if (state?.isActive !== true || state.tier !== "T5") return undefined;
	const primary = primaryFrom(state);
	const parent = primaryFrom(state.parent);
	if (primary === undefined || parent === undefined) return undefined;
	return { isActive: true, tier: "T5", ...primary, parent };
}

export function parseModelReference(model: string): { provider: string; id: string } | undefined {
	const separator = model.indexOf("/");
	if (separator <= 0 || separator === model.length - 1) return undefined;
	return { provider: model.slice(0, separator), id: model.slice(separator + 1) };
}

export function tierFivePrimaryFrom(settings: unknown): TierPrimary | undefined {
	return primaryFrom(record(record(settings)?.tierPrimaries)?.T5);
}

export function latestTurboState(entries: readonly unknown[]): ActiveTurboState | undefined {
	for (let index = entries.length - 1; index >= 0; index--) {
		const entry = record(entries[index]);
		if (entry?.type !== "custom" || entry.customType !== "turbo-state") continue;
		const state = turboStateFrom(entry.data);
		if (state === undefined) continue;
		if (!state.isActive) return undefined;
		return state;
	}
	return undefined;
}

export function turboOverrideFor(primary: TierPrimary): {
	model: string;
	thinkingLevel: ThinkingLevel;
	excludedAgentTypes: typeof REVIEW_AGENT_TYPES;
} {
	return { model: primary.model, thinkingLevel: primary.thinking, excludedAgentTypes: REVIEW_AGENT_TYPES };
}
