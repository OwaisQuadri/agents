/**
 * Pure extraction: which tool calls are in scope, what text to run comment-check's
 * `--list-json` over, and how to find the code that follows a comment span so a
 * docstring-position claim ("is this actually on a public declaration") can be judged.
 *
 * No subprocess calls live here — those belong to the orchestrator in
 * comment-shape-guard.ts, which already owns async/spawn concerns for the judge worker
 * itself. This file stays synchronous and fully testable without a filesystem or a
 * child process.
 */

export type EditFragment = { path: string; oldText: string; newText: string };
export type WriteFragment = { path: string; content: string };

/** `edits[].newText` is the only thing an edit call is about to write; `oldText` is not
 * — it is what is being replaced, and only useful for locating on-disk trailing context. */
export function editFragments(input: { path: string; edits: { oldText: string; newText: string }[] }): EditFragment[] {
	return input.edits.map((e) => ({ path: input.path, oldText: e.oldText, newText: e.newText }));
}

export function writeFragment(input: { path: string; content: string }): WriteFragment {
	return { path: input.path, content: input.content };
}

/** The extension comment-check's `--lang` flag expects, or undefined for an extensionless
 * or dotfile path (comment-check's shebang classification needs file content, which this
 * function does not have — the orchestrator falls back to that separately if needed). */
export function extensionOf(path: string): string | undefined {
	const base = path.split("/").at(-1) ?? path;
	const dot = base.lastIndexOf(".");
	if (dot <= 0) return undefined; // no extension, or a dotfile like ".gitignore"
	return base.slice(dot + 1);
}

export type CommentSpan = { startLine: number; endLine: number; kind: "doc" | "plain"; text: string };

/** Lines immediately after `span` within the SAME fragment text, up to `windowLines`.
 * Empty when the span is the fragment's last content (the common case where a
 * declaration was edited together with its comment covers this without ever needing
 * the on-disk fallback). */
export function followingContextWithinFragment(fragmentText: string, span: CommentSpan, windowLines = 3): string {
	const lines = fragmentText.split("\n");
	return lines
		.slice(span.endLine, span.endLine + windowLines)
		.filter((l) => l.trim().length > 0)
		.join("\n");
}

/** Fallback for when a span's own fragment has nothing after it: locate `oldText`
 * inside the file's CURRENT on-disk content (the edit has not applied yet at
 * `tool_call` time — this is genuinely the pre-edit file) and return the lines that
 * follow it there. Those lines are untouched by this edit, so they are exactly what
 * will follow `newText` once the edit lands. Returns undefined when `oldText` cannot
 * be located (should not happen for a real edit tool call, but a caller must not crash
 * on it) or when nothing follows it (end of file) — both cases mean the docstring
 * position claim is genuinely unverifiable, and the caller must judge conservatively
 * rather than guess. */
export function followingContextOnDisk(currentFileContent: string, oldText: string, windowLines = 3): string | undefined {
	const index = currentFileContent.indexOf(oldText);
	if (index === -1) return undefined;
	const after = currentFileContent.slice(index + oldText.length);
	const lines = after
		.split("\n")
		.filter((l) => l.trim().length > 0)
		.slice(0, windowLines);
	return lines.length > 0 ? lines.join("\n") : undefined;
}
