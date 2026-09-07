import { createHash } from "node:crypto";
import type { JudgmentInput } from "./protocol.ts";

export type CacheEntry = Readonly<{ version: 1; key: string; decision: "pass" | "block"; reason: string }>;

const decisions = new Map<string, CacheEntry>();
const capacity = 256;

export function judgmentKey(input: JudgmentInput): string {
	return createHash("sha256").update(JSON.stringify([input.comment, input.code_context, input.language, input.rule_document, input.prompt_version, input.schema_version, input.judgment_configuration])).digest("hex");
}

export function readDecision(key: string): CacheEntry | undefined {
	return decisions.get(key);
}

export function writeDecision(entry: CacheEntry): void {
	decisions.delete(entry.key);
	decisions.set(entry.key, Object.freeze({ ...entry }));
	if (decisions.size > capacity) decisions.delete(decisions.keys().next().value!);
}
