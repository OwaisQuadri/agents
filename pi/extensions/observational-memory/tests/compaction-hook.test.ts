import { describe, expect, it } from "vitest";
import { registerCompactionHook } from "../src/hooks/compaction-hook.js";
import { Runtime } from "../src/runtime.js";

function deferred() {
	let resolve!: () => void;
	const promise = new Promise<void>((done) => {
		resolve = done;
	});
	return { promise, resolve };
}

describe("compaction observer wait", () => {
	it("stops waiting when compaction is cancelled", async () => {
		const handlers = new Map<string, (event: any, ctx: any) => Promise<unknown>>();
		const pi = {
			on(event: string, handler: (event: any, ctx: any) => Promise<unknown>) {
				handlers.set(event, handler);
			},
		};
		const runtime = new Runtime();
		runtime.enabled = true;
		runtime.configLoaded = true;
		runtime.config.passive = false;
		const observer = deferred();
		runtime.trackObserverTask(observer.promise);
		registerCompactionHook(pi as any, runtime);
		const controller = new AbortController();
		const handler = handlers.get("session_before_compact");
		if (!handler) throw new Error("Compaction hook was not registered");

		const waiting = handler({
			signal: controller.signal,
			preparation: { firstKeptEntryId: "first", tokensBefore: 100 },
			branchEntries: [],
		}, {
			hasUI: false,
			cwd: process.cwd(),
			sessionManager: { getBranch: () => [] },
		});
		controller.abort(new Error("Compaction cancelled"));

		await expect(waiting).resolves.toEqual({ cancel: true });
		expect(runtime.compactHookInFlight).toBe(false);
		observer.resolve();
	});

	it("cancels an already-aborted compaction before waiting", async () => {
		const handlers = new Map<string, (event: any, ctx: any) => Promise<unknown>>();
		const pi = {
			on(event: string, handler: (event: any, ctx: any) => Promise<unknown>) {
				handlers.set(event, handler);
			},
		};
		const runtime = new Runtime();
		runtime.enabled = true;
		runtime.configLoaded = true;
		runtime.config.passive = false;
		registerCompactionHook(pi as any, runtime);
		const controller = new AbortController();
		controller.abort(new Error("Compaction cancelled"));
		const handler = handlers.get("session_before_compact");
		if (!handler) throw new Error("Compaction hook was not registered");

		await expect(handler({
			signal: controller.signal,
			preparation: { firstKeptEntryId: "first", tokensBefore: 100 },
			branchEntries: [],
		}, {
			hasUI: false,
			cwd: process.cwd(),
			sessionManager: { getBranch: () => [] },
		})).resolves.toEqual({ cancel: true });
		expect(runtime.compactHookInFlight).toBe(false);
	});
});
