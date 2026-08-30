// KNOWN GAP, accepted: a path written to a file in one group and executed in a later
// group (`echo "..." > x.sh; bash x.sh`) is not caught — closing it, and every encoded
// variant, needs a real shell parser. This guard targets an agent's typical or mistaken
// bash usage, not a deliberate bypass of its own rail.
const READ_ONLY_COMMANDS = new Set([
	"cat", "less", "more", "head", "tail", "grep", "egrep", "fgrep", "rg", "ls", "stat", "file", "wc",
	"diff", "cmp", "md5sum", "shasum", "sha1sum", "sha256sum", "realpath", "readlink", "dirname",
	"basename", "pwd", "which", "type", "jq", "tree", "du", "xxd", "hexdump", "bat", "nl", "column",
	"od", "strings", "echo", "printf", "true", "test", "[",
]);

// Allowlist, not blocklist: an unlisted subcommand (pull, merge, commit, ...) defaults
// to write. Narrowing needs an observed false positive, never an imagined one.
const GIT_READ_SUBCOMMANDS = new Set([
	"status", "diff", "log", "show", "blame", "ls-files", "cat-file", "describe",
	"shortlog", "reflog", "name-rev", "rev-parse", "ls-remote", "ls-tree", "grep",
]);

// A stage running one of these can execute a protected-path reference carried by an
// earlier stage, e.g. `echo "sed -i ... AGENTS.md" | bash`.
const INTERPRETER_COMMANDS = new Set([
	"bash", "sh", "zsh", "dash", "ksh", "ash", "csh", "tcsh", "python", "python3", "perl",
	"ruby", "node", "xargs", "eval", "source", ".", "osascript",
]);

// Regex-based, no quote tracking — deliberate. A rare misparse falls through to the
// write default below, never to a false allow.
function splitTopLevelGroups(command: string): string[] {
	return command.split(/&&|\|\||;|\n|(?<!\|)&(?!>)/);
}

// `|&` is `2>&1 |`, still a pipe — fold it into `|` before splitting, or the leading
// `&` lands on the next stage's command name.
function splitPipeStages(group: string): string[] {
	return group.replace(/\|&/g, "|").split("|");
}

function leadingCommand(segment: string): string | undefined {
	let rest = segment.trim();
	rest = rest.replace(/^(?:[A-Za-z_][\w]*=\S*\s+)+/, "");
	rest = rest.replace(/^(?:sudo|nice|time|command)\s+/, "");
	const match = /^(\S+)/.exec(rest);
	if (match === null) return undefined;
	return match[1].split("/").pop();
}

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
	// `reflog` alone (or `reflog show`) reads; `expire`/`delete` discard history.
	if (subcommand === "reflog") {
		const action = words[index + 1];
		return action === "expire" || action === "delete";
	}
	return !GIT_READ_SUBCOMMANDS.has(subcommand);
}

function segmentWritesProtectedPath(segment: string, pathReferencePattern: RegExp): boolean {
	if (!pathReferencePattern.test(segment)) return false;
	if (/\$\(|`|<\(|>\(/.test(segment)) return true;
	if (hasOutputRedirectToProtectedPath(segment, pathReferencePattern)) return true;
	const rawLeading = leadingCommand(segment);
	if (rawLeading === undefined) return true;
	// Lowercase once: a differently-cased spelling (`BASH`) still resolves to the same
	// binary, and must still trip the INTERPRETER_COMMANDS check below.
	const leading = rawLeading.toLowerCase();
	if (leading === "git") return gitSubcommandWrites(segment);
	if (leading === "sed") return /(?:^|\s)(?:-i\b|--in-place\b)/.test(segment);
	if (leading === "awk") return /(?:^|\s)-i\b/.test(segment);
	if (leading === "find") return /-delete\b|-exec\b|-execdir\b|-fprint\w*\b|-ok\b/.test(segment);
	return !READ_ONLY_COMMANDS.has(leading);
}

// A protected-path reference feeding an interpreter stage is a write regardless of
// which stage carries the text — the interpreter can act on it sight unseen.
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
 * merely reading one.
 *
 * @param command The bash tool call's command string.
 * @param pathReferencePattern The protected-path pattern, already scoped to a home directory.
 * @returns True when the command plausibly mutates a protected path.
 * @throws Never.
 */
export function bashCommandWritesProtectedPath(command: string, pathReferencePattern: RegExp): boolean {
	return splitTopLevelGroups(command).some((group) => groupWritesProtectedPath(group, pathReferencePattern));
}
