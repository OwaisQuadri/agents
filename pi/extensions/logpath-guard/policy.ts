import { isAbsolute, resolve } from "node:path";

import { expandHome, extractLeadingCd, extractLogpathRedirectTargets } from "./extract.ts";

export type BashToolInput = { command: string };
export type ValidateResult = { ok: boolean; reason?: string };
/** Runs the actual structural check against one resolved absolute path. */
export type Validator = (repoRoot: string, absoluteTarget: string) => ValidateResult;

/**
 * Returns the block reason for a bash tool call that would append a usage.jsonl log line
 * to a path that structurally cannot be the artifact's real log, or undefined when the
 * call has no such redirect, or every redirect resolves correctly.
 *
 * @param input The bash tool call's input.
 * @param ctx The session's cwd, the repo root, and the home directory to expand `~` against.
 * @param validate Runs the structural check on one resolved absolute path (real callers
 *   pass the compiled `logpath-check` binary; tests pass a fixture).
 * @returns A block reason naming the bad path and the check's own reason, or undefined.
 * @throws Never.
 */
export function blockedLogpathWrite(
	input: BashToolInput,
	ctx: { cwd: string; repoRoot: string; home: string },
	validate: Validator,
): string | undefined {
	const targets = extractLogpathRedirectTargets(input.command);
	if (targets.length === 0) return undefined;

	const leadingCd = extractLeadingCd(input.command);
	const base = leadingCd ? resolve(ctx.cwd, expandHome(leadingCd, ctx.home)) : ctx.cwd;

	for (const rawTarget of targets) {
		const expanded = expandHome(rawTarget, ctx.home);
		const absolute = isAbsolute(expanded) ? expanded : resolve(base, expanded);
		const result = validate(ctx.repoRoot, absolute);
		if (!result.ok) {
			return `Blocked a write to ${absolute} — ${result.reason ?? "failed the logpath-check structural check"}. Use the anchored form from the artifact's own "## logging" section instead (<repo-root>/<skills|agents|workflows>/<name>/logs/usage.jsonl, <repo-root> from \`git rev-parse --show-toplevel\`).`;
		}
	}
	return undefined;
}
