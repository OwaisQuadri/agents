import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { describe, expect, it } from "vitest";
import observationalMemory from "../src/index.js";
import { OM_ENABLED } from "../src/ledger/index.js";

type EventHandler = (event: unknown, context: unknown) => unknown;

describe("observational memory gate", () => {
	it("does not read model context while the extension is disabled", () => {
		const handlers = new Map<string, EventHandler>();
		const pi = {
			on(event: string, handler: EventHandler): void {
				handlers.set(event, handler);
			},
			registerCommand(): void {},
		} as unknown as ExtensionAPI;
		observationalMemory(pi);

		const staleContext = new Proxy(
			{},
			{
				get(): never {
					throw new Error("stale context accessed");
				},
			},
		);

		const modelSelect = handlers.get("model_select");
		expect(modelSelect).toBeTypeOf("function");
		if (!modelSelect) throw new Error("model_select handler was not registered");
		expect(() => modelSelect({}, staleContext)).not.toThrow();
	});

	it("does not compact while passive mode is active", () => {
		const previousPassive = process.env.PI_OM_PASSIVE;
		const projectDirectory = mkdtempSync(join(tmpdir(), "om-passive-"));
		process.env.PI_OM_PASSIVE = "1";
		try {
			const handlers = new Map<string, EventHandler>();
			const pi = {
				on(event: string, handler: EventHandler): void {
					handlers.set(event, handler);
				},
				registerCommand(): void {},
			} as unknown as ExtensionAPI;
			observationalMemory(pi);
			const context = {
				cwd: projectDirectory,
				mode: "json",
				hasUI: false,
				model: { contextWindow: 100_000 },
				sessionManager: {
					getBranch: () => [{ type: "custom", customType: OM_ENABLED, data: { enabled: true } }],
					getEntries: () => [],
					getSessionId: () => "passive-session",
					getHeader: () => undefined,
				},
				getContextUsage: () => ({ tokens: 100_000 }),
				compact: () => {
					throw new Error("passive mode compacted the session");
				},
			};
			const sessionStart = handlers.get("session_start");
			const modelSelect = handlers.get("model_select");
			if (!sessionStart || !modelSelect) throw new Error("required handler was not registered");

			sessionStart({}, context);
			expect(() => modelSelect({}, context)).not.toThrow();
		} finally {
			rmSync(projectDirectory, { recursive: true, force: true });
			if (previousPassive === undefined) delete process.env.PI_OM_PASSIVE;
			else process.env.PI_OM_PASSIVE = previousPassive;
		}
	});

	it("applies the current model context window when enabled", async () => {
		const projectDirectory = mkdtempSync(join(tmpdir(), "om-enable-"));
		try {
			const handlers = new Map<string, EventHandler>();
			let commandHandler: ((args: string, context: unknown) => Promise<void>) | undefined;
			const pi = {
				on(event: string, handler: EventHandler): void {
					handlers.set(event, handler);
				},
				registerCommand(name: string, command: { handler: (args: string, context: unknown) => Promise<void> }): void {
					if (name === "om") commandHandler = command.handler;
				},
				appendEntry(): void {},
				sendMessage(): void {},
			} as unknown as ExtensionAPI;
			observationalMemory(pi);
			let compactCalls = 0;
			const context = {
				cwd: projectDirectory,
				mode: "json",
				hasUI: false,
				model: { contextWindow: 1_000_000 },
				sessionManager: {
					getBranch: () => [],
					getEntries: () => [],
					getSessionId: () => "enabled-session",
					getHeader: () => undefined,
				},
				getContextUsage: () => ({ tokens: 40_000 }),
				compact: () => {
					compactCalls += 1;
				},
			};
			const sessionStart = handlers.get("session_start");
			const turnEnd = handlers.get("turn_end");
			if (!sessionStart || !turnEnd || !commandHandler) throw new Error("required handler was not registered");

			sessionStart({}, context);
			context.model.contextWindow = 100_000;
			await commandHandler("on", context);
			turnEnd({ toolResults: [] }, context);

			expect(compactCalls).toBe(1);
		} finally {
			rmSync(projectDirectory, { recursive: true, force: true });
		}
	});
});
