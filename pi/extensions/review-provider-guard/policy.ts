export const GUARDED_AGENT_TYPES = ["anchor-verifier", "code-reviewer", "maestro-tester", "spec-tester"] as const;

export type AgentToolInput = { subagent_type: string; model?: string };
export type ModelEntry = { id: string; name: string; provider: string };

export type DispatchContext = {
	sessionProvider: string | undefined;
	sessionModelId: string | undefined;
	availableModels: readonly ModelEntry[];
	agentDefaultModel: (agentType: string) => string | undefined;
};

const MAX_SUGGESTIONS = 2;

function normalize(reference: string): string {
	return reference.toLowerCase().replaceAll(".", "-");
}

function qualifiedName(model: ModelEntry): string {
	return `${model.provider}/${model.id}`;
}

export function providersForModelRef(reference: string, models: readonly ModelEntry[]): string[] {
	const query = normalize(reference.trim());
	if (query.length === 0) return [];

	const exact = models.find((model) => normalize(qualifiedName(model)) === query);
	if (exact !== undefined) return [exact.provider];

	const providers: string[] = [];
	for (const model of models) {
		const fields = [model.id, model.name, qualifiedName(model)].map(normalize);
		if (fields.some((field) => field === query || field.includes(query)) && !providers.includes(model.provider)) {
			providers.push(model.provider);
		}
	}
	return providers;
}

function otherProviderModels(context: DispatchContext): string[] {
	const others: string[] = [];
	for (const model of context.availableModels) {
		if (model.provider !== context.sessionProvider && others.length < MAX_SUGGESTIONS) {
			others.push(qualifiedName(model));
		}
	}
	return others;
}

export function blockedReviewDispatch(toolName: string, input: AgentToolInput, context: DispatchContext): string | undefined {
	if (toolName !== "Agent") return undefined;
	const agentType = input.subagent_type;
	if (!(GUARDED_AGENT_TYPES as readonly string[]).includes(agentType)) return undefined;
	if (context.sessionProvider === undefined) return undefined;

	const effective = (input.model ?? context.agentDefaultModel(agentType))?.trim();
	if (effective === undefined || effective.length === 0) return undefined;

	const candidates = providersForModelRef(effective, context.availableModels);
	if (candidates.length === 0) return undefined;
	if (!candidates.every((provider) => provider === context.sessionProvider)) return undefined;

	const builder = context.sessionModelId === undefined ? context.sessionProvider : `${context.sessionProvider}/${context.sessionModelId}`;
	const reason = `Blocked a ${agentType} dispatch resolving to "${effective}" on ${builder} — the same provider that built the change.`;
	const others = otherProviderModels(context);
	if (others.length === 0) return reason;
	return `${reason} Re-dispatch with an explicit \`model\` on a different provider, e.g. ${others.map((name) => `"${name}"`).join(", ")}.`;
}
