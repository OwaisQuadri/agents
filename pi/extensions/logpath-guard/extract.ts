/**
 * Extracts every `>` / `>>` redirect target in a bash command whose path ends in
 * `logs/usage.jsonl`. Deliberately conservative: only matches a directly-following shell
 * token (bare or quoted), never attempts to resolve `$(...)` command substitution or
 * other dynamic shell syntax inside the target — ambiguous input is skipped, not guessed
 * at. A guard that blocks on a guess is worse than one that misses an edge case
 * (ai-author's own rule: a checker deterministic about the wrong thing carries false
 * authority).
 *
 * @param command The bash tool call's command string.
 * @returns Every redirect target ending in `logs/usage.jsonl`, in order of appearance.
 * @throws Never.
 */
export function extractLogpathRedirectTargets(command: string): string[] {
	const targets: string[] = [];
	const pattern = />>?\s*(['"]?)([^\s'"|;&]*logs\/usage\.jsonl)\1/g;
	let match: RegExpExecArray | null = pattern.exec(command);
	while (match !== null) {
		targets.push(match[2]);
		match = pattern.exec(command);
	}
	return targets;
}

/**
 * Extracts a leading `cd <dir> &&` prefix's target directory, if the command starts with
 * one — the pattern every logged usage-line example in this repo actually uses
 * (`cd ~/Documents/agents && mkdir -p ... && ... >> ...`).
 *
 * @param command The bash tool call's command string.
 * @returns The `cd` target exactly as written (not yet expanded), or undefined.
 * @throws Never.
 */
export function extractLeadingCd(command: string): string | undefined {
	const match = /^\s*cd\s+(['"]?)([^\s'"]+)\1\s*&&/.exec(command);
	return match?.[2];
}

/**
 * Expands a leading `~`, `$HOME`, or `${HOME}` in a shell-written path.
 *
 * @param raw The path as written in the shell command.
 * @param home The home directory to substitute.
 * @returns The path with the leading token replaced by `home`, unchanged if none matched.
 * @throws Never.
 */
export function expandHome(raw: string, home: string): string {
	return raw.replace(/^(~|\$HOME|\$\{HOME\})(?=\/|$)/, home);
}
