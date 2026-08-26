import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

import {
	settleHerdrAgent,
	startHerdrAgent,
	startHerdrSession,
	stopHerdrSession,
} from "./herdr-activity/state.ts";

export default function herdrActivity(pi: ExtensionAPI): void {
	pi.on("session_start", (_event, ctx) => startHerdrSession(pi, ctx));
	pi.on("agent_start", (_event, ctx) => startHerdrAgent(pi, ctx));
	pi.on("agent_settled", (_event, ctx) => settleHerdrAgent(pi, ctx));
	pi.on("session_shutdown", (_event, ctx) => stopHerdrSession(pi, ctx));
}
