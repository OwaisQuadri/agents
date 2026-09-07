import { stripVTControlCharacters } from "node:util";

export type PlannotatorPhase = "idle" | "planning" | "executing";

export type SessionEntry = {
	type: string;
	customType?: string;
	data?: unknown;
};

function isHorizontalBorder(line: string): boolean {
	return /^─{2,}(?: |$)/.test(line);
}

function spaceBorderRuns(line: string): string {
	let isDashNext = true;
	return line.replace(/─+/g, (run) => {
		let spacedRun = "";
		for (let index = 0; index < run.length; index += 1) {
			spacedRun += isDashNext ? "-" : " ";
			isDashNext = !isDashNext;
		}
		return spacedRun;
	});
}

function phaseFromData(data: unknown): PlannotatorPhase | undefined {
	if (typeof data !== "object" || data === null || !("phase" in data)) return undefined;
	const phase = data.phase;
	return phase === "idle" || phase === "planning" || phase === "executing" ? phase : undefined;
}

/**
 * Reads the latest authoritative Plannotator phase from session entries.
 * @param entries The active session branch entries in chronological order.
 * @returns The recorded phase, or idle when the latest record is invalid or absent.
 * @throws Never.
 */
export function getPlannotatorPhase(entries: readonly SessionEntry[]): PlannotatorPhase {
	for (let index = entries.length - 1; index >= 0; index -= 1) {
		const entry = entries[index];
		if (entry?.type !== "custom" || entry.customType !== "plannotator") continue;
		return phaseFromData(entry.data) ?? "idle";
	}
	return "idle";
}

/**
 * Replaces solid fill glyphs in the outer editor lines during planning.
 * @param lines The rendered editor lines.
 * @param phase The authoritative Plannotator phase.
 * @returns New rendered lines with spaced planning borders or unchanged content.
 * @throws Never.
 */
export function renderPlanningBorder(lines: string[], phase: PlannotatorPhase): string[] {
	if (phase !== "planning" || lines.length === 0) return lines;
	const rendered = [...lines];

	const topLine = stripVTControlCharacters(rendered[0] ?? "");
	if (isHorizontalBorder(topLine)) {
		rendered[0] = spaceBorderRuns(rendered[0] ?? "");
	}
	for (let index = rendered.length - 1; index > 0; index -= 1) {
		const visibleLine = stripVTControlCharacters(rendered[index] ?? "");
		if (!isHorizontalBorder(visibleLine)) continue;
		rendered[index] = spaceBorderRuns(rendered[index] ?? "");
		break;
	}
	return rendered;
}
