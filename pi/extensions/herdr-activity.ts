import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

import {
	settleHerdrAgent,
	startHerdrAgent,
	startHerdrSession,
	stopHerdrSession,
} from "./herdr-activity/state.ts";

type SubagentLifecycleEvent = {
	id?: unknown;
};

export default function herdrActivity(pi: ExtensionAPI): void {
	const activeSubagentIds = new Set<string>();

	function setSubagentActive(payload: unknown, isActive: boolean): void {
		if (payload === null || typeof payload !== "object") return;
		const { id } = payload as SubagentLifecycleEvent;
		if (typeof id !== "string" || id.trim().length === 0) return;

		const wasBusy = activeSubagentIds.size > 0;
		if (isActive) activeSubagentIds.add(id);
		else activeSubagentIds.delete(id);
		const isBusy = activeSubagentIds.size > 0;
		if (wasBusy !== isBusy) pi.events.emit("herdr:busy", { active: isBusy });
	}

	function clearSubagents(): void {
		if (activeSubagentIds.size === 0) return;
		activeSubagentIds.clear();
		pi.events.emit("herdr:busy", { active: false });
	}

	pi.on("session_start", (_event, ctx) => {
		clearSubagents();
		return startHerdrSession(pi, ctx);
	});
	pi.on("agent_start", (_event, ctx) => startHerdrAgent(pi, ctx));
	pi.on("agent_settled", (_event, ctx) => settleHerdrAgent(pi, ctx));
	pi.events.on("subagents:started", (payload) => setSubagentActive(payload, true));
	pi.events.on("subagents:completed", (payload) => setSubagentActive(payload, false));
	pi.events.on("subagents:failed", (payload) => setSubagentActive(payload, false));
	pi.on("session_shutdown", (_event, ctx) => {
		clearSubagents();
		return stopHerdrSession(pi, ctx);
	});
}
