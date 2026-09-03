import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent";
import { Type } from "typebox";
import type { Static } from "typebox";

import { writeWorkerVerdict } from "../../spawn/runs.ts";

const SubmitVerdictSchema = Type.Object({
	shape: Type.String({
		description: 'The exact whitelist shape name this comment fits (character for character, as shown in the prompt), or the literal string "none" if nothing fits.',
	}),
	reason: Type.String({ description: "One sentence explaining the decision." }),
});

export type SubmitVerdictInput = Static<typeof SubmitVerdictSchema>;

/** Registers the worker's only tool. One call ends the run (agent_end shuts the
 * subprocess down right after) — unlike observational-memory's observer, which
 * accumulates across many calls, a judge decides exactly one span per run. */
export function registerVerdictTool(pi: ExtensionAPI, resultPath: string): void {
	pi.registerTool({
		name: "submit_verdict",
		label: "Submit verdict",
		description: "Submit the shape verdict for the one comment shown to you. Call this exactly once.",
		parameters: SubmitVerdictSchema,
		async execute(_id: string, params: SubmitVerdictInput, _signal: AbortSignal | undefined, _onUpdate: unknown, _ctx: ExtensionContext) {
			writeWorkerVerdict(resultPath, { shape: params.shape.trim(), reason: params.reason.trim() });
			return { content: [{ type: "text" as const, text: "verdict recorded" }] };
		},
	});
}
