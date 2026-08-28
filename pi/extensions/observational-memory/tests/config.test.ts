import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { loadConfig } from "../src/config.js";

const ENV_AGENT_DIR = "PI_CODING_AGENT_DIR";

let globalDir: string;
let projectDir: string;
let previousEnvAgentDir: string | undefined;

function writeGlobalSettings(value: Record<string, unknown>): void {
	writeFileSync(join(globalDir, "settings.json"), JSON.stringify(value), "utf-8");
}

beforeEach(() => {
	globalDir = mkdtempSync(join(tmpdir(), "om-config-global-"));
	projectDir = mkdtempSync(join(tmpdir(), "om-config-project-"));
	previousEnvAgentDir = process.env[ENV_AGENT_DIR];
	process.env[ENV_AGENT_DIR] = globalDir;
});

afterEach(() => {
	rmSync(globalDir, { recursive: true, force: true });
	rmSync(projectDir, { recursive: true, force: true });
	if (previousEnvAgentDir === undefined) delete process.env[ENV_AGENT_DIR];
	else process.env[ENV_AGENT_DIR] = previousEnvAgentDir;
});

describe("loadConfig models", () => {
	it("resolves the tier-compiled model from subagents.agentOverrides when nothing else is set", () => {
		writeGlobalSettings({
			subagents: {
				agentOverrides: {
					"om-observer": { model: "openai-codex/gpt-5.6-luna", fallbackModels: ["anthropic/claude-haiku-4-5"] },
					"om-consolidator": { model: "openai-codex/gpt-5.3-codex-spark", fallbackModels: ["anthropic/claude-sonnet-5"] },
				},
			},
		});

		const config = loadConfig(projectDir);

		expect(config.models.observer).toEqual({ provider: "openai-codex", id: "gpt-5.6-luna", thinking: "low" });
		expect(config.models.consolidator).toEqual({ provider: "openai-codex", id: "gpt-5.3-codex-spark", thinking: "medium" });
	});

	it("prefers an explicit observational-memory.models override over the tier-compiled model", () => {
		writeGlobalSettings({
			subagents: {
				agentOverrides: {
					"om-observer": { model: "openai-codex/gpt-5.6-luna", fallbackModels: [] },
				},
			},
			"observational-memory": {
				models: {
					observer: { provider: "anthropic", id: "claude-haiku-4-5", thinking: "medium" },
				},
			},
		});

		const config = loadConfig(projectDir);

		expect(config.models.observer).toEqual({ provider: "anthropic", id: "claude-haiku-4-5", thinking: "medium" });
	});

	it("falls back to the free default when subagents.agentOverrides is absent (install.sh never run)", () => {
		writeGlobalSettings({});

		const config = loadConfig(projectDir);

		expect(config.models.observer).toEqual({ provider: "openrouter", id: "openrouter/free", thinking: "low" });
		expect(config.models.consolidator).toEqual({ provider: "openrouter", id: "openrouter/free", thinking: "medium" });
	});
});
