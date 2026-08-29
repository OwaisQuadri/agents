// KNOWN GAP, accepted (2026, user sign-off): a protected-path reference written to a
// file in one top-level group and executed in a later group —
// `echo "sed -i ... AGENTS.md" > /tmp/x.sh; bash /tmp/x.sh` — is not caught, because
// groupWritesProtectedPath judges each group independently and the execution group
// carries no path text of its own. This is the entry point to an unbounded class
// (base64-encoded payloads, more groups, a different write tool) that a segment-level
// heuristic cannot close without becoming a real shell interpreter. This guard's threat
// model is an agent's typical or mistaken bash usage, not a deliberate adversarial
// bypass of its own safety rail — closing this class needs a real shell parser or a
// dry-run trace, a materially bigger investment than this module makes. Narrowing this
// note needs a materially different threat model, never an imagined attacker.
const READ_ONLY_COMMANDS = new Set([
	"cat", "less", "more", "head", "tail", "grep", "egrep", "fgrep", "rg", "ls", "stat", "file", "wc",
	"diff", "cmp", "md5sum", "shasum", "sha1sum", "sha256sum", "realpath", "readlink", "dirname",
	"basename", "pwd", "which", "type", "jq", "tree", "du", "xxd", "hexdump", "bat", "nl", "column",
	"od", "strings", "echo", "printf", "true", "test", "[",
]);

// Read-only git subcommands, not read-anything-not-on-a-blocklist: `pull`, `merge`,
// `commit`, `cherry-pick`, `revert`, `submodule update`, and any future subcommand all
// default to write. Narrowing this list needs an observed false positive on a command
// proven not to mutate the working tree, never an imagined one.
const GIT_READ_SUBCOMMANDS = new Set([
	"status", "diff", "log", "show", "blame", "ls-files", "cat-file", "describe",
	"shortlog", "reflog", "name-rev", "rev-parse", "ls-remote", "ls-tree", "grep",
]);

// A pipeline stage whose leading command is an interpreter can execute text it never
// itself names a protected path in — `echo "sed -i ... AGENTS.md" | bash` carries the
// path only in the `echo` stage, not the `bash` stage. Any interpreter stage in a
// pipeline that references a protected path anywhere in the pipeline is a write.
const INTERPRETER_COMMANDS = new Set([
	"bash", "sh", "zsh", "dash", "ksh", "ash", "csh", "tcsh", "python", "python3", "perl",
	"ruby", "node", "xargs", "eval", "source", ".", "osascript",
]);

/**
 * Splits a bash command into the top-level groups a shell would run separately — on
 * `&&`, `||`, `;`, newlines, and a background-job `&` (but not the `&` inside `&>`/`&>>`
 * redirects). A group may still contain internal `|` pipe stages; those are split
 * separately by `splitPipeStages` so a pipeline can be judged as a whole. Deliberately
 * conservative, like logpath-guard/extract.ts: regex-based, no quote tracking. A rare
 * misparse falls through to the default-write verdict below, never to a false allow.
 */
function splitTopLevelGroups(command: string): string[] {
	return command.split(/&&|\|\||;|\n|(?<!\|)&(?!>)/);
}

// `|&` (bash/zsh shorthand for `2>&1 |`) is still a pipe, not a background job —
// splitTopLevelGroups' lookbehind already keeps its `&` out of the background-job split,
// so here it only needs folding into a plain `|` before the stage split, or the leading
// `&` would land on the next stage's command name.
function splitPipeStages(group: string): string[] {
	return group.replace(/\|&/g, "|").split("|");
}

/**
 * Extracts a segment's leading command name, past env-var assignments, `sudo`, `nice`,
 * `time`, `command`, and any directory prefix (`/usr/bin/cat` -> `cat`).
 *
 * @param segment One shell segment.
 * @returns The leading command name, or undefined when the segment has none.
 * @throws Never.
 */
function leadingCommand(segment: string): string | undefined {
	let rest = segment.trim();
	rest = rest.replace(/^(?:[A-Za-z_][\w]*=\S*\s+)+/, "");
	rest = rest.replace(/^(?:sudo|nice|time|command)\s+/, "");
	const match = /^(\S+)/.exec(rest);
	if (match === null) return undefined;
	return match[1].split("/").pop();
}

/**
 * Checks whether a segment redirects output (`>`, `>>`, `1>`, `2>`, `&>`, `&>>`) at a
 * target that names a protected agent-config path.
 *
 * @param segment One shell segment.
 * @param pathReferencePattern The protected-path pattern to test each redirect target against.
 * @returns True when a redirect target matches a protected path.
 * @throws Never.
 */
function hasOutputRedirectToProtectedPath(segment: string, pathReferencePattern: RegExp): boolean {
	const pattern = /(?:[12]?>>?|&>>?)\s*(['"]?)([^\s'"|;&]+)\1/g;
	let match: RegExpExecArray | null = pattern.exec(segment);
	while (match !== null) {
		if (pathReferencePattern.test(match[2])) return true;
		match = pattern.exec(segment);
	}
	return false;
}

const GIT_FLAGS_WITH_VALUE = new Set(["-C", "-c", "--git-dir", "--work-tree", "--namespace"]);

function gitSubcommandWrites(segment: string): boolean {
	const words = segment.trim().split(/\s+/);
	let index = 1;
	while (index < words.length && words[index].startsWith("-")) {
		if (GIT_FLAGS_WITH_VALUE.has(words[index])) index += 1;
		index += 1;
	}
	const subcommand = words[index];
	if (subcommand === undefined) return true;
	// `git reflog` alone (or `reflog show`) reads history; `reflog expire`/`reflog delete`
	// permanently discard entries — the bare subcommand name doesn't say which.
	if (subcommand === "reflog") {
		const action = words[index + 1];
		return action === "expire" || action === "delete";
	}
	return !GIT_READ_SUBCOMMANDS.has(subcommand);
}

/**
 * Judges whether one shell segment mutates a path it references. A known read-only
 * command (cat, grep, ls, ...) that never redirects output and never shells out through
 * command/process substitution is a read. Everything else — an unrecognized command, a
 * write command, `sed -i`, `git rm`, a `$(...)` substitution that could hide anything —
 * defaults to a write. Narrowing this list needs an observed false positive, never an
 * imagined one.
 *
 * @param segment One shell segment.
 * @param pathReferencePattern The protected-path pattern, already scoped to a home directory.
 * @returns True when the segment both references and plausibly mutates a protected path.
 * @throws Never.
 */
function segmentWritesProtectedPath(segment: string, pathReferencePattern: RegExp): boolean {
	if (!pathReferencePattern.test(segment)) return false;
	if (/\$\(|`|<\(|>\(/.test(segment)) return true;
	if (hasOutputRedirectToProtectedPath(segment, pathReferencePattern)) return true;
	const rawLeading = leadingCommand(segment);
	if (rawLeading === undefined) return true;
	// Command names are matched case-insensitively: a differently-cased spelling (`BASH`,
	// `Sed`) still resolves to the same binary via PATH lookup, so treating it as a
	// distinct, unrecognized command would default it to write anyway for `sed`/`awk`/
	// `find`/`git` (safe) but would wrongly skip the `INTERPRETER_COMMANDS` check in
	// groupWritesProtectedPath (unsafe) — lowercase once, use everywhere.
	const leading = rawLeading.toLowerCase();
	if (leading === "git") return gitSubcommandWrites(segment);
	if (leading === "sed") return /(?:^|\s)(?:-i\b|--in-place\b)/.test(segment);
	if (leading === "awk") return /(?:^|\s)-i\b/.test(segment);
	if (leading === "find") return /-delete\b|-exec\b|-execdir\b|-fprint\w*\b|-ok\b/.test(segment);
	return !READ_ONLY_COMMANDS.has(leading);
}

/**
 * Judges whether one top-level group (itself possibly a `|` pipeline) mutates a
 * protected path it references. A pipeline feeding a protected-path reference into an
 * interpreter stage (`echo "...AGENTS.md..." | bash`) is a write regardless of which
 * stage carries the path text, because the interpreter stage can act on it sight
 * unseen. Otherwise each pipe stage is judged independently, same as a lone segment.
 *
 * @param group One top-level command group, as split by `splitTopLevelGroups`.
 * @param pathReferencePattern The protected-path pattern, already scoped to a home directory.
 * @returns True when the group plausibly mutates a protected path.
 * @throws Never.
 */
function groupWritesProtectedPath(group: string, pathReferencePattern: RegExp): boolean {
	if (!pathReferencePattern.test(group)) return false;
	const stages = splitPipeStages(group);
	const hasInterpreterStage = stages.some((stage) => {
		const leading = leadingCommand(stage);
		return leading !== undefined && INTERPRETER_COMMANDS.has(leading.toLowerCase());
	});
	if (hasInterpreterStage) return true;
	return stages.some((stage) => segmentWritesProtectedPath(stage, pathReferencePattern));
}

/**
 * Judges whether a bash command mutates a protected agent-config path, as opposed to
 * merely reading one (`cat`, `grep`, `less`, a plain pipeline of those). Read-only access
 * to config is allowed; anything that plausibly writes, deletes, or shells out through it
 * stays blocked.
 *
 * @param command The bash tool call's command string.
 * @param pathReferencePattern The protected-path pattern, already scoped to a home directory.
 * @returns True when any group of the command plausibly mutates a protected path.
 * @throws Never.
 */
export function bashCommandWritesProtectedPath(command: string, pathReferencePattern: RegExp): boolean {
	return splitTopLevelGroups(command).some((group) => groupWritesProtectedPath(group, pathReferencePattern));
}
