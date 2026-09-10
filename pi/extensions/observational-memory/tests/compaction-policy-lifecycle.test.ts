import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { describe, expect, it, vi } from "vitest";
import observationalMemory from "../src/index.js";
import {
	MEMORY_COMPACTION_POLICY_EVENT,
	type MemoryCompactionPolicy,
} from "../src/hooks/compaction-trigger.js";

type Handler = (event: unknown, context: unknown) => unknown;

describe("compaction policy lifecycle", () => {
	it("republishes when memory is enabled, disabled, or the model changes", async () => {
		const handlers = new Map<string, Handler[]>();
		const policies: MemoryCompactionPolicy[] = [];
		let commandHandler: ((args: string, context: unknown) => Promise<void>) | undefined;
		const pi = {
			on(event: string, handler: Handler): void {
				const eventHandlers = handlers.get(event) ?? [];
				eventHandlers.push(handler);
				handlers.set(event, eventHandlers);
			},
			registerCommand(name: string, command: { handler: (args: string, context: unknown) => Promise<void> }): void {
				if (name === "om") commandHandler = command.handler;
			},
			appendEntry: vi.fn(),
			sendMessage: vi.fn(),
			events: {
				emit(channel: string, data: unknown): void {
					if (channel !== MEMORY_COMPACTION_POLICY_EVENT) return;
					const policy = data as MemoryCompactionPolicy;
					policy.isApplied = true;
					policies.push({ ...policy });
				},
			},
		} as unknown as ExtensionAPI;
		const compact = vi.fn();
		const context = {
			cwd: process.cwd(),
			mode: "json",
			hasUI: false,
			model: { contextWindow: 10_000 },
			sessionManager: {
				getBranch: () => [],
				getEntries: () => [],
				getSessionId: () => "policy-lifecycle",
				getHeader: () => undefined,
			},
			getContextUsage: () => ({ tokens: 9_000 }),
			compact,
		};
		observationalMemory(pi);

		for (const handler of handlers.get("session_start") ?? []) await handler({}, context);
		if (!commandHandler) throw new Error("om command was not registered");
		await commandHandler("on", context);
		for (const handler of handlers.get("model_select") ?? []) await handler({}, context);
		await commandHandler("off", context);

		expect(policies.map(policy => policy.isEnabled)).toEqual([false, true, true, false]);
		expect(compact).not.toHaveBeenCalled();
	});
});
