import { isToolCallEventType, type ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { spawnSync } from "node:child_process";
import { existsSync, realpathSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { blockedPreferredCliCommand, type Checker } from "./preferred-cli-guard/policy.ts";

// Wiring only: detection lives in tools/preferred-cli-guard/src/main.rs, run here as
// a subprocess rather than reimplemented in TypeScript.
export default function preferredCliGuard(pi: ExtensionAPI): void {
	const extensionPath = realpathSync(fileURLToPath(import.meta.url));
	const repositoryRoot = resolve(dirname(extensionPath), "..", "..");
	const binary = resolve(repositoryRoot, "tools/preferred-cli-guard/target/release/preferred-cli-guard");

	const check: Checker = (command) => {
		// Missing binary (fresh checkout) degrades to allow, same posture every checker
		// in this file's family takes — never a false block over a missing build.
		if (!existsSync(binary)) return { blocked: false };
		const run = spawnSync(binary, ["--check", command], { encoding: "utf8" });
		if (run.status === 0) return { blocked: false };
		return {
			blocked: true,
			reason: run.stdout.trim() || run.stderr.trim() || `preferred-cli-guard exited ${run.status}`,
		};
	};

	pi.on("tool_call", (event) => {
		if (!isToolCallEventType("bash", event)) return;
		const reason = blockedPreferredCliCommand(event.input, check);
		if (reason !== undefined) return { block: true, reason };
	});
}
