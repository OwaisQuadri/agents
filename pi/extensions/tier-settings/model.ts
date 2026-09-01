// Pure data shape and edit logic for the /tiers command. Navigation (list cursor, submenu
// stack, search) is owned by pi-tui's own SettingsList component (see tier-settings.ts) — the
// same component the built-in /settings command uses — so this file only keeps the part that
// is actually specific to config/model-tiers.json: the schema, validation, and the edit itself.

export type ThinkingLevel = "off" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max";
export const THINKING_LEVELS: ThinkingLevel[] = ["off", "minimal", "low", "medium", "high", "xhigh", "max"];

export type TierEntry = { model: string; thinking: ThinkingLevel };
export type Tier = { pi: TierEntry; fallbacks: TierEntry[]; climbOnExhaustion?: string };
export type TierFile = {
	tiers: Record<string, Tier>;
	orchestrator: string;
	agents: Record<string, string>;
	untiered?: Record<string, string>;
};

// Which entry within a tier an edit targets: the tier's own primary, or one fallback by its
// position in the ordered list.
export type Slot = { kind: "pi" } | { kind: "fallback"; index: number };

export function tierNames(file: TierFile): string[] {
	return Object.keys(file.tiers).sort();
}

export function slotsOf(tier: Tier): { slot: Slot; entry: TierEntry }[] {
	return [
		{ slot: { kind: "pi" } as const, entry: tier.pi },
		...tier.fallbacks.map((entry, index) => ({ slot: { kind: "fallback", index } as const, entry })),
	];
}

export function entryAt(tier: Tier, slot: Slot): TierEntry {
	return slot.kind === "pi" ? tier.pi : tier.fallbacks[slot.index];
}

function isTierEntry(value: unknown): value is TierEntry {
	return (
		typeof value === "object" &&
		value !== null &&
		typeof (value as { model?: unknown }).model === "string" &&
		isValidThinking((value as { thinking?: unknown }).thinking as string)
	);
}

/**
 * Validates the parsed JSON actually matches the per-entry `{model, thinking}` schema before
 * any code trusts `.model`/`.thinking` on a tier's `pi`/`fallbacks` entries. Without this,
 * `JSON.parse(raw) as TierFile` is just a compile-time assertion with no runtime effect: a repo
 * checkout still on the older flat-string schema (`"pi": "provider/model"`, tier-level
 * `"thinking"`) parses fine, `entryAt` returns that raw string, and `.model`/`.thinking` on a
 * string are `undefined` — which only surfaces later as a crash deep inside pi-tui's renderer
 * (`Cannot read properties of undefined (reading 'length')`), far from this cause. Catching the
 * mismatch here turns that into one clear, actionable message.
 */
export function parseTierFile(raw: string): TierFile {
	const parsed: unknown = JSON.parse(raw);
	if (typeof parsed !== "object" || parsed === null || !("tiers" in parsed)) {
		throw new Error("config/model-tiers.json: missing top-level \"tiers\" object");
	}
	const tiers = (parsed as { tiers: unknown }).tiers;
	if (typeof tiers !== "object" || tiers === null) {
		throw new Error('config/model-tiers.json: "tiers" is not an object');
	}
	for (const [name, tier] of Object.entries(tiers as Record<string, unknown>)) {
		if (typeof tier !== "object" || tier === null) {
			throw new Error(`config/model-tiers.json: tier "${name}" is not an object`);
		}
		const { pi, fallbacks } = tier as { pi?: unknown; fallbacks?: unknown };
		if (!isTierEntry(pi)) {
			throw new Error(
				`config/model-tiers.json: tier "${name}".pi is not a {model, thinking} object (this checkout may still be on the old flat-string schema — /tiers needs the per-model-thinking schema from GH-184)`,
			);
		}
		if (!Array.isArray(fallbacks) || !fallbacks.every(isTierEntry)) {
			throw new Error(`config/model-tiers.json: tier "${name}".fallbacks is not an array of {model, thinking} objects`);
		}
	}
	return parsed as TierFile;
}

/**
 * Model ids look like `provider/id` (and sometimes `provider/id/id`, e.g. the openrouter
 * catalog) — one or more non-empty segments after a first `/`. Rejects a bare word, a
 * leading/trailing slash, or an empty string, without needing the live model catalog.
 */
export function isValidModelId(candidate: string): boolean {
	const trimmed = candidate.trim();
	if (!trimmed.includes("/")) {
		return false;
	}
	const segments = trimmed.split("/");
	return segments.every((segment) => segment.length > 0);
}

export function isValidThinking(candidate: string): candidate is ThinkingLevel {
	return (THINKING_LEVELS as string[]).includes(candidate);
}

/**
 * Returns a NEW TierFile with one entry's model and thinking replaced, so the caller can
 * diff/serialize the result without mutating whatever was loaded from disk. Preserves key
 * order (JSON.stringify walks object keys in insertion order, and this only ever replaces
 * an existing entry's own fields, never re-inserts the object) and the fallback array's order.
 */
export function applyEdit(file: TierFile, tier: string, slot: Slot, model: string, thinking: ThinkingLevel): TierFile {
	const target = file.tiers[tier];
	if (!target) {
		throw new Error(`applyEdit: tier ${tier} does not exist`);
	}
	const nextEntry: TierEntry = { model, thinking };
	const nextTier: Tier =
		slot.kind === "pi"
			? { ...target, pi: nextEntry }
			: { ...target, fallbacks: target.fallbacks.map((entry, index) => (index === slot.index ? nextEntry : entry)) };
	return { ...file, tiers: { ...file.tiers, [tier]: nextTier } };
}
