/**
 * The judge worker's own extension (L4), loaded into the headless subprocess via `-e`.
 * Single role, unlike observational-memory's observer/consolidator branch on OM_WORKER
 * — a judge run only ever does one thing: decide one comment span's shape and exit.
 * Builtin tools are disabled (`--no-builtin-tools`); this registers exactly the one
 * tool it needs.
 */
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

import { JUDGE_SYSTEM } from "../prompt.ts";
import { registerVerdictTool } from "./tool.ts";

export default function commentShapeJudge(pi: ExtensionAPI): void {
	const resultPath = process.env.CSG_RESULT_PATH;
	if (!resultPath) throw new Error("CSG_RESULT_PATH not set for comment-shape-guard judge worker");

	registerVerdictTool(pi, resultPath);

	pi.on("before_agent_start", async () => {
		return { systemPrompt: JUDGE_SYSTEM };
	});

	// Headless `pi -p` exits when the agent loop ends; shutdown is a belt-and-suspenders,
	// matching observational-memory's own worker.
	pi.on("agent_end", async (_event: unknown, ctx: { shutdown: () => void }) => {
		ctx.shutdown();
	});
}
