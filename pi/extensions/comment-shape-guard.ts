import { createEditToolDefinition, createWriteToolDefinition, type ExtensionAPI } from "@earendil-works/pi-coding-agent";

export default function commentShapeGuard(pi: ExtensionAPI): void {
	pi.registerTool({
		...createEditToolDefinition(process.cwd()),
		async execute(id, params, signal, onUpdate, ctx) {
			const start = performance.now();
			let deadline = start + 20_000;
			const { createOperations } = await import("./comment-shape-guard/direct.ts");
			const operations = createOperations({ operation: "edit", get deadline() { return deadline; }, signal, validate: async (proposal) => {
				const { validateProposal } = await import("./comment-shape-guard/validate.ts");
				deadline = await validateProposal(proposal, start, deadline, signal);
			} });
			return createEditToolDefinition(ctx.cwd, { operations }).execute(id, params, undefined, onUpdate, ctx);
		},
	});
	pi.registerTool({
		...createWriteToolDefinition(process.cwd()),
		async execute(id, params, signal, onUpdate, ctx) {
			const start = performance.now();
			let deadline = start + 20_000;
			const { createOperations } = await import("./comment-shape-guard/direct.ts");
			const operations = createOperations({ operation: "write", get deadline() { return deadline; }, signal, validate: async (proposal) => {
				const { validateProposal } = await import("./comment-shape-guard/validate.ts");
				deadline = await validateProposal(proposal, start, deadline, signal);
			} });
			return createWriteToolDefinition(ctx.cwd, { operations }).execute(id, params, undefined, onUpdate, ctx);
		},
	});
}
