import { isToolCallEventType, type ExtensionAPI, type ExtensionContext } from "@earendil-works/pi-coding-agent";

import { blockedReviewDispatch, type AgentToolInput, type DispatchContext, type ModelEntry } from "./review-provider-guard/policy.ts";
import { agentDefaultModelFrom, readSubagentSettings } from "./review-provider-guard/settings.ts";

function reachableModels(ctx: ExtensionContext): ModelEntry[] {
	return ctx.modelRegistry.getAvailable().map((model) => ({ id: model.id, name: model.name, provider: model.provider }));
}

export default function reviewProviderGuard(pi: ExtensionAPI): void {
	let cachedSettings: unknown;

	const agentDefaultModel = (agentType: string): string | undefined => {
		cachedSettings ??= readSubagentSettings();
		return agentDefaultModelFrom(cachedSettings, agentType);
	};

	pi.on("tool_call", (event, ctx) => {
		if (!isToolCallEventType<"Agent", AgentToolInput>("Agent", event)) return;
		const context: DispatchContext = {
			sessionProvider: ctx.model?.provider,
			sessionModelId: ctx.model?.id,
			availableModels: reachableModels(ctx),
			agentDefaultModel,
		};
		const reason = blockedReviewDispatch(event.toolName, event.input, context);
		if (reason !== undefined) return { block: true, reason };
	});
}
