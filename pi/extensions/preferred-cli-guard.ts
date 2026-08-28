import { isToolCallEventType, type ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { spawnSync } from "node:child_process";
import { existsSync, realpathSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { blockedPreferredCliCommand, type Checker } from "./preferred-cli-guard/policy.ts";

// Wiring only — see pi/extensions/preferred-cli-guard/policy.ts for the translation
// and tools/preferred-cli-guard/src/main.rs for the actual tokenizing + rule-table
// decision. The check itself stays in Rust (real computation: command-position-aware
// tokenization) and runs here as a subprocess, never reimplemented in TypeScript —
// tool-author's own rule against a second implementation of the same rule.
export default function preferredCliGuard(pi: ExtensionAPI): void {
	const extensionPath = realpathSync(fileURLToPath(import.meta.url));
	const repositoryRoot = resolve(dirname(extensionPath), "..", "..");
	const binary = resolve(repositoryRoot, "tools/preferred-cli-guard/target/release/preferred-cli-guard");

	const check: Checker = (command) => {
		// The checker binary may not be built yet (fresh checkout, cargo not run).
		// Degrade to allow rather than block every bash call on missing infrastructure —
		// the same choice pi/extensions/logpath-guard.ts and hooks/rag-recall's own
		// docstring make: a broken guard must degrade to silence, never to a dead turn.
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
