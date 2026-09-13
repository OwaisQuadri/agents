import { isToolCallEventType, type ExtensionAPI } from "@earendil-works/pi-coding-agent";

type Runner = (binary: string, args: string[]) => { status: number | null; stdout?: string; stderr?: string };
type AgentToolInput = { isolated?: boolean };

export default function agentMergeGuard(pi: ExtensionAPI, run?: Runner): void {
	pi.on("tool_call", async (event, ctx) => {
		if (isToolCallEventType<"Agent", AgentToolInput>("Agent", event) && event.input.isolated === true) {
			return {
				block: true,
				reason: "Run the child agent without isolated mode so agent-merge-guard remains active.",
			};
		}
		if (!isToolCallEventType("bash", event)) return;
		const [{ homedir }, { join }] = await Promise.all([import("node:os"), import("node:path")]);
		const binary = join(homedir(), ".local", "lib", "agents-tools", "agent-merge-guard");
		const args = ["--check", event.input.command, "--cwd", ctx.cwd];
		const result = run
			? run(binary, args)
			: await (async () => {
				const { spawnSync } = await import("node:child_process");
				const spawned = spawnSync(binary, args, { encoding: "utf8" });
				return { status: spawned.status, stdout: spawned.stdout, stderr: spawned.stderr };
			})();
		if (result.status === 0) return;
		return {
			block: true,
			reason: result.stdout || result.stderr || "Blocked merge: agent-merge-guard could not execute.",
		};
	});
}
