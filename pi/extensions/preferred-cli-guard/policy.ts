export type CheckResult = { blocked: boolean; reason?: string };
export type Checker = (command: string) => CheckResult;
export type BashToolInput = { command: string };

/**
 * Returns the block reason for a bash tool call the `preferred-cli-guard` Rust checker
 * rejects (a literal `find`/`grep` invocation), or `undefined` when the checker allows
 * it. All the decision logic (tokenizing, the rule table, the reason text) lives in
 * `tools/preferred-cli-guard/src/main.rs` — this only translates its verdict; `check` is
 * injected so a test can stand in for the compiled binary without building it.
 *
 * @param input The bash tool call's input.
 * @param check Runs the compiled checker (real callers pass a `spawnSync` wrapper;
 *   tests pass a fixture).
 * @returns The block reason, or undefined.
 * @throws Never.
 */
export function blockedPreferredCliCommand(input: BashToolInput, check: Checker): string | undefined {
	const result = check(input.command);
	return result.blocked ? result.reason : undefined;
}
