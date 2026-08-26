import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

const observationalMemoryGate = "om.enabled";

function isObservationalMemoryEnabled(entries: Array<{ type?: string; customType?: string; data?: unknown }>): boolean {
	for (let index = entries.length - 1; index >= 0; index -= 1) {
		const entry = entries[index];
		if (entry?.type !== "custom" || entry.customType !== observationalMemoryGate) continue;
		return (entry.data as { enabled?: boolean } | undefined)?.enabled === true;
	}
	return false;
}

export default function observationalMemoryAlwaysOn(pi: ExtensionAPI): void {
	pi.on("session_start", (_event, ctx) => {
		if (isObservationalMemoryEnabled(ctx.sessionManager.getBranch())) return;
		pi.sendUserMessage("/om on", { expandPromptTemplates: true });
	});
}
