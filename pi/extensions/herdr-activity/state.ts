import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent";

type HerdrStatus = "blocked" | "idle" | "working";
type HerdrContext = Pick<ExtensionContext, "mode" | "sessionManager">;

interface HerdrActivityState {
	activeAgentCount: number;
	isBlocked: boolean;
	sequence: number;
}

const STATE_KEY = "__owaisHerdrActivityState";

function state(): HerdrActivityState {
	const globals = globalThis as typeof globalThis & { __owaisHerdrActivityState?: HerdrActivityState };
	if (globals.__owaisHerdrActivityState === undefined) {
		globals.__owaisHerdrActivityState = {
			activeAgentCount: 0,
			isBlocked: false,
			sequence: Date.now() * 1000,
		};
	}
	return globals.__owaisHerdrActivityState;
}

function status(current: HerdrActivityState): HerdrStatus {
	if (current.isBlocked) return "blocked";
	return current.activeAgentCount > 0 ? "working" : "idle";
}

function message(current: HerdrActivityState, isSettled: boolean): string {
	if (current.isBlocked) return "Needs attention";
	if (current.activeAgentCount > 0) return "Working";
	return isSettled ? "Done" : "Idle";
}

async function report(pi: ExtensionAPI, ctx: HerdrContext, isSettled = false): Promise<void> {
	const paneId = process.env.HERDR_PANE_ID;
	if (ctx.mode !== "tui" || process.env.HERDR_ENV !== "1" || !process.env.HERDR_SOCKET_PATH || !paneId) return;

	const current = state();
	const args = [
		"pane",
		"report-agent",
		paneId,
		"--source",
		"pi",
		"--agent",
		"pi",
		"--state",
		status(current),
		"--message",
		message(current, isSettled),
		"--seq",
		String(++current.sequence),
	];
	const sessionId = ctx.sessionManager.getSessionId();
	if (sessionId) args.push("--agent-session-id", sessionId);

	try {
		await pi.exec(process.env.HERDR_BIN_PATH || "herdr", args);
	} catch {}
}

export function resetHerdrActivityState(): void {
	delete (globalThis as typeof globalThis & { __owaisHerdrActivityState?: HerdrActivityState }).__owaisHerdrActivityState;
}

export async function startHerdrSession(pi: ExtensionAPI, ctx: HerdrContext): Promise<void> {
	const current = state();
	current.activeAgentCount = 0;
	current.isBlocked = false;
	await report(pi, ctx);
}

export async function startHerdrAgent(pi: ExtensionAPI, ctx: HerdrContext): Promise<void> {
	state().activeAgentCount += 1;
	await report(pi, ctx);
}

export async function settleHerdrAgent(pi: ExtensionAPI, ctx: HerdrContext): Promise<void> {
	// agent_start can fire more than once per agent_settled (once per retry or queued
	// continuation within one run), so settling zeroes the counter instead of
	// decrementing it — a decrement can strand it above zero and pin "working" forever.
	state().activeAgentCount = 0;
	await report(pi, ctx, true);
}

export async function setHerdrBlocked(pi: ExtensionAPI, ctx: HerdrContext, isBlocked: boolean): Promise<void> {
	state().isBlocked = isBlocked;
	await report(pi, ctx);
}

export async function stopHerdrSession(pi: ExtensionAPI, ctx: HerdrContext): Promise<void> {
	const current = state();
	current.activeAgentCount = 0;
	current.isBlocked = false;
	await report(pi, ctx);
}
