import { readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join, resolve } from "node:path";

function asRecord(value: unknown): Record<string, unknown> | undefined {
	return typeof value === "object" && value !== null && !Array.isArray(value) ? (value as Record<string, unknown>) : undefined;
}

function asModelString(value: unknown): string | undefined {
	return typeof value === "string" && value.trim().length > 0 ? value : undefined;
}

export function piAgentRoot(env: Record<string, string | undefined> = process.env): string {
	const configured = env.PI_CODING_AGENT_DIR?.trim();
	return resolve(configured !== undefined && configured.length > 0 ? configured : join(homedir(), ".pi", "agent"));
}

export function readSubagentSettings(root: string = piAgentRoot()): unknown {
	try {
		return JSON.parse(readFileSync(join(root, "settings.json"), "utf8")) as unknown;
	} catch {
		return {};
	}
}

export function agentDefaultModelFrom(settings: unknown, agentType: string): string | undefined {
	const subagents = asRecord(asRecord(settings)?.subagents);
	if (subagents === undefined) return undefined;
	const override = asRecord(asRecord(subagents.agentOverrides)?.[agentType]);
	return asModelString(override?.model) ?? asModelString(subagents.defaultModel);
}
