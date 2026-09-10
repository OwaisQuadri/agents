import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { describe, expect, it, vi } from "vitest";
import {
	MEMORY_COMPACTION_POLICY_EVENT,
	publishCompactionPolicy,
	registerCompactionTrigger,
	type MemoryCompactionPolicy,
} from "../src/hooks/compaction-trigger.js";
import { OM_RESUME } from "../src/ledger/index.js";
import { Runtime } from "../src/runtime.js";

type Handler = (event: unknown, context: unknown) => unknown;

function enabledRuntime(): Runtime {
	const runtime = new Runtime();
	runtime.enabled = true;
	runtime.configLoaded = true;
	runtime.config.passive = false;
	runtime.config.compactAtContextTokens = 5_000;
	return runtime;
}

function triggerContext() {
	return {
		hasUI: false,
		model: { contextWindow: 10_000 },
		sessionManager: { getBranch: () => [] },
		getContextUsage: () => ({ tokens: 9_809 }),
		compact: vi.fn(),
	};
}

function triggerHandler(handlers: Map<string, Handler>): Handler {
	const handler = handlers.get("turn_end");
	if (!handler) throw new Error("Compaction trigger was not registered");
	return handler;
}

describe("compaction policy publishing", () => {
	it("uses native ownership instead of manual compaction or a resume message", () => {
		const handlers = new Map<string, Handler>();
		const sendMessage = vi.fn();
		const pi = {
			on(event: string, handler: Handler): void {
				handlers.set(event, handler);
			},
			sendMessage,
			events: {
				emit(channel: string, data: unknown): void {
				if (channel === MEMORY_COMPACTION_POLICY_EVENT) {
					(data as MemoryCompactionPolicy).isApplied = true;
				}
			},
		},
		} as unknown as ExtensionAPI;
		const runtime = enabledRuntime();
		const context = triggerContext();
		registerCompactionTrigger(pi, runtime);

		triggerHandler(handlers)({
			message: { role: "assistant", stopReason: "toolUse", usage: { totalTokens: 809 } },
			toolResults: [{}],
		}, context);

		expect(context.compact).not.toHaveBeenCalled();
		expect(sendMessage).not.toHaveBeenCalled();
	});

	it("keeps the legacy compact and resume path without a policy receiver", () => {
		const handlers = new Map<string, Handler>();
		const sendMessage = vi.fn();
		const pi = {
			on(event: string, handler: Handler): void {
				handlers.set(event, handler);
			},
			sendMessage,
		} as unknown as ExtensionAPI;
		const runtime = enabledRuntime();
		const context = triggerContext();
		registerCompactionTrigger(pi, runtime);

		triggerHandler(handlers)({
			message: { role: "assistant", stopReason: "toolUse", usage: { totalTokens: 809 } },
			toolResults: [{}],
		}, context);

		expect(context.compact).toHaveBeenCalledTimes(1);
		const options = context.compact.mock.calls[0][0] as { onComplete: () => void };
		options.onComplete();
		expect(sendMessage).toHaveBeenCalledWith(
			expect.objectContaining({ customType: OM_RESUME }),
		{ triggerTurn: true },
		);
	});

	it("carries the trailing tool-result allowance into an acknowledged policy", () => {
		const handlers = new Map<string, Handler>();
		let policy: MemoryCompactionPolicy | undefined;
		const pi = {
			on(event: string, handler: Handler): void {
				handlers.set(event, handler);
			},
			sendMessage(): void {},
			events: {
				emit(channel: string, data: unknown): void {
				if (channel !== MEMORY_COMPACTION_POLICY_EVENT) return;
				policy = data as MemoryCompactionPolicy;
				policy.isApplied = true;
			},
		},
		} as unknown as ExtensionAPI;
		const runtime = enabledRuntime();
		const context = triggerContext();
		registerCompactionTrigger(pi, runtime);

		triggerHandler(handlers)({
			message: { role: "assistant", stopReason: "toolUse", usage: { totalTokens: 809 } },
			toolResults: [{}],
		}, context);

		expect(policy).toMatchObject({ keepRecentTokens: 9_001, isApplied: true });
	});

	it("records a receiver rejection before falling back", () => {
		const runtime = enabledRuntime();
		const pi = {
			events: {
				emit(channel: string, data: unknown): void {
				if (channel === MEMORY_COMPACTION_POLICY_EVENT) {
					(data as MemoryCompactionPolicy).error = "Pi 0.84.2 does not support between-turn automatic compaction";
				}
			},
		},
		} as unknown as ExtensionAPI;

		expect(publishCompactionPolicy(pi, runtime, { model: { contextWindow: 10_000 } })).toBe(false);
		expect(runtime.lastWorkerError).toContain("does not support");
	});
});
