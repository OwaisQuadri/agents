import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent";
import { randomUUID } from "node:crypto";
import { readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join, resolve } from "node:path";

import {
	SESSION_MODEL_OVERRIDE_CHANNEL,
	latestTurboState,
	parseModelReference,
	tierFivePrimaryFrom,
	turboOverrideFor,
	type ActiveTurboState,
	type TierPrimary,
} from "./turbo/policy.ts";

function settingsFile(): string {
	const configured = process.env.PI_CODING_AGENT_DIR?.trim();
	return join(resolve(configured && configured.length > 0 ? configured : join(homedir(), ".pi", "agent")), "settings.json");
}

function tierFivePrimary(): TierPrimary | undefined {
	try {
		return tierFivePrimaryFrom(JSON.parse(readFileSync(settingsFile(), "utf8")) as unknown);
	} catch {
		return undefined;
	}
}

async function selectModel(pi: ExtensionAPI, ctx: ExtensionContext, primary: TierPrimary): Promise<boolean> {
	const reference = parseModelReference(primary.model);
	const model = reference === undefined ? undefined : ctx.modelRegistry.find(reference.provider, reference.id);
	if (model === undefined || !(await pi.setModel(model))) return false;
	pi.setThinkingLevel(primary.thinking);
	return true;
}

function parentModel(pi: ExtensionAPI, ctx: ExtensionContext): TierPrimary | undefined {
	if (ctx.model === undefined) return undefined;
	return { model: `${ctx.model.provider}/${ctx.model.id}`, thinking: pi.getThinkingLevel() };
}

const RPC_TIMEOUT_MS = 500;
const READINESS_TIMEOUT_MS = 5_000;

async function requestSessionModelOverride(pi: ExtensionAPI, request: object): Promise<string | undefined> {
	const requestId = randomUUID();
	const replyChannel = `${SESSION_MODEL_OVERRIDE_CHANNEL}:reply:${requestId}`;
	return new Promise((resolveReply) => {
		const unsubscribe = pi.events.on(replyChannel, (reply) => {
			clearTimeout(timeout);
			unsubscribe();
			if (typeof reply !== "object" || reply === null || !("success" in reply)) {
				resolveReply("The worker override returned an invalid reply.");
				return;
			}
			const result = reply as { success: boolean; error?: unknown };
			resolveReply(result.success ? undefined : String(result.error ?? "The worker override failed."));
		});
		const timeout = setTimeout(() => {
			unsubscribe();
			resolveReply("The installed worker package does not support Turbo.");
		}, RPC_TIMEOUT_MS);
		pi.events.emit(SESSION_MODEL_OVERRIDE_CHANNEL, { requestId, ...request });
	});
}

async function rollbackActivation(
	pi: ExtensionAPI,
	ctx: ExtensionContext,
	parent: TierPrimary,
	error: string,
): Promise<{ isCleared: boolean; message: string }> {
	const clearError = await requestSessionModelOverride(pi, { clear: true });
	const didRestore = await selectModel(pi, ctx, parent);
	const clearMessage = clearError === undefined ? "" : " Pi could not confirm that the worker override was cleared.";
	const restoreMessage = didRestore ? "" : " Pi could not restore the parent model.";
	return { isCleared: clearError === undefined, message: `${error}${clearMessage}${restoreMessage}` };
}

export default function turboExtension(pi: ExtensionAPI): void {
	let active: ActiveTurboState | undefined;
	let isChanging = false;
	let isSubagentsReady = false;
	let pendingRestore: { state: ActiveTurboState; ctx: ExtensionContext } | undefined;
	let readinessTimeout: ReturnType<typeof setTimeout> | undefined;

	const setActive = (state: ActiveTurboState, ctx: ExtensionContext): void => {
		active = state;
		ctx.ui.setStatus("turbo", "turbo:T5");
	};

	const restorePersisted = async (state: ActiveTurboState, ctx: ExtensionContext): Promise<void> => {
		try {
			if (!(await selectModel(pi, ctx, state))) {
				active = undefined;
				pi.appendEntry("turbo-state", { isActive: false });
				ctx.ui.notify("Turbo could not authenticate the persisted T5 model.", "warning");
				return;
			}
			const error = await requestSessionModelOverride(pi, turboOverrideFor(state));
			if (error !== undefined) {
				const rollback = await rollbackActivation(pi, ctx, state.parent, error);
				if (rollback.isCleared) {
					active = undefined;
					pi.appendEntry("turbo-state", { isActive: false });
				} else {
					active = state;
					ctx.ui.setStatus("turbo", "turbo:pending");
				}
				ctx.ui.notify(rollback.message, "warning");
				return;
			}
			setActive(state, ctx);
		} finally {
			isChanging = false;
		}
	};

	const startPendingRestore = (): Promise<void> | undefined => {
		if (pendingRestore === undefined) return undefined;
		if (readinessTimeout !== undefined) clearTimeout(readinessTimeout);
		const pending = pendingRestore;
		pendingRestore = undefined;
		return restorePersisted(pending.state, pending.ctx);
	};

	pi.events.on("subagents:ready", () => {
		isSubagentsReady = true;
		void startPendingRestore();
	});

	pi.on("session_start", (_event, ctx) => {
		if (readinessTimeout !== undefined) clearTimeout(readinessTimeout);
		active = latestTurboState(ctx.sessionManager.getBranch());
		ctx.ui.setStatus("turbo", undefined);
		if (active === undefined) return;
		isChanging = true;
		pendingRestore = { state: active, ctx };
		if (isSubagentsReady) {
			void startPendingRestore();
			return;
		}
		readinessTimeout = setTimeout(() => {
			if (pendingRestore === undefined) return;
			pendingRestore = undefined;
			isChanging = false;
			ctx.ui.setStatus("turbo", "turbo:pending");
			ctx.ui.notify("Turbo could not restore because the worker package is not ready.", "warning");
		}, READINESS_TIMEOUT_MS);
	});

	pi.on("session_shutdown", () => {
		if (readinessTimeout !== undefined) clearTimeout(readinessTimeout);
		readinessTimeout = undefined;
		pendingRestore = undefined;
		active = undefined;
		isChanging = false;
		isSubagentsReady = false;
	});

	pi.registerCommand("turbo", {
		description: "Toggle the current session and new workers on Tier 5",
		handler: async (_args, ctx) => {
			if (isChanging) {
				ctx.ui.notify("Turbo is already changing.", "warning");
				return;
			}
			isChanging = true;
			try {
				if (active !== undefined) {
					const state = active;
					if (!(await selectModel(pi, ctx, state.parent))) {
						ctx.ui.notify("Turbo could not restore the parent model.", "warning");
						return;
					}
					const error = await requestSessionModelOverride(pi, { clear: true });
					if (error !== undefined) {
						const overrideError = await requestSessionModelOverride(pi, turboOverrideFor(state));
						if (overrideError === undefined) {
							const didRestore = await selectModel(pi, ctx, state);
							ctx.ui.notify(didRestore ? error : `${error} Pi could not restore the T5 model.`, "warning");
							return;
						}
						const finalClearError = await requestSessionModelOverride(pi, { clear: true });
						if (finalClearError !== undefined) {
							const didRestore = await selectModel(pi, ctx, state);
							const restoreMessage = didRestore ? "" : " Pi could not restore the T5 model.";
							ctx.ui.notify(`${error} Pi could not confirm the worker override state.${restoreMessage}`, "warning");
							return;
						}
						active = undefined;
						pi.appendEntry("turbo-state", { isActive: false });
						ctx.ui.setStatus("turbo", undefined);
						ctx.ui.notify("Turbo is off after the worker override retry failed.", "warning");
						return;
					}
					active = undefined;
					pi.appendEntry("turbo-state", { isActive: false });
					ctx.ui.setStatus("turbo", undefined);
					return;
				}

				const primary = tierFivePrimary();
				const parent = parentModel(pi, ctx);
				if (primary === undefined || parent === undefined) {
					ctx.ui.notify("Turbo could not find a valid T5 model and parent session model.", "warning");
					return;
				}
				if (!(await selectModel(pi, ctx, primary))) {
					ctx.ui.notify("Turbo could not authenticate the configured T5 model.", "warning");
					return;
				}
				const state: ActiveTurboState = { isActive: true, tier: "T5", ...primary, parent };
				const error = await requestSessionModelOverride(pi, turboOverrideFor(primary));
				if (error !== undefined) {
					const rollback = await rollbackActivation(pi, ctx, parent, error);
					if (!rollback.isCleared) {
						active = state;
						pi.appendEntry("turbo-state", state);
						ctx.ui.setStatus("turbo", "turbo:pending");
					}
					ctx.ui.notify(rollback.message, "warning");
					return;
				}
				pi.appendEntry("turbo-state", state);
				setActive(state, ctx);
			} finally {
				isChanging = false;
			}
		},
	});
}
