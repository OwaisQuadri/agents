import { isToolCallEventType, type ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { spawnSync } from "node:child_process";
import { existsSync, realpathSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { blockedLogpathWrite, type Validator } from "./logpath-guard/policy.ts";

// Wiring only — see pi/extensions/logpath-guard/policy.ts for the decision logic and
// tools/logpath-check/src/main.rs for the actual structural check. The check itself
// stays in Rust (real computation: path resolution, artifact-directory existence) and
// runs here as a subprocess via `--validate-path`, never reimplemented in TypeScript —
// tool-author's own rule against a second implementation of the same rule.
export default function logpathGuard(pi: ExtensionAPI): void {
	const extensionPath = realpathSync(fileURLToPath(import.meta.url));
	const repositoryRoot = resolve(dirname(extensionPath), "..", "..");
	const binary = resolve(repositoryRoot, "tools/logpath-check/target/release/logpath-check");

	const validate: Validator = (repoRoot, absoluteTarget) => {
		// The checker binary may not be built yet (fresh checkout, cargo not run). Degrade
		// to allow rather than block every bash call on missing infrastructure — the same
		// choice hooks/rag-recall's own docstring states: a broken check must degrade to
		// silence, never to a dead turn.
		if (!existsSync(binary)) return { ok: true };
		const run = spawnSync(binary, [repoRoot, "--validate-path", absoluteTarget], { encoding: "utf8" });
		if (run.status === 0) return { ok: true };
		return { ok: false, reason: run.stdout.trim() || run.stderr.trim() || `logpath-check exited ${run.status}` };
	};

	pi.on("tool_call", (event, ctx) => {
		if (!isToolCallEventType("bash", event)) return;
		const reason = blockedLogpathWrite(event.input, { cwd: ctx.cwd, repoRoot: repositoryRoot, home: homedir() }, validate);
		if (reason !== undefined) return { block: true, reason };
	});
}
