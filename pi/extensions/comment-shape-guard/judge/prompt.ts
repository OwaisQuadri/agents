/**
 * The judge worker's system prompt (general rules, static) and its kickoff-prompt
 * builder (the specific span to judge, passed as the headless `pi -p` prompt so the
 * run stays inspectable on resume — the same reason observational-memory passes its
 * chunk as the prompt rather than injecting it via a hook).
 */

export const JUDGE_SYSTEM = `You are a single-purpose comment-shape judge for a coding assistant's pre-write guard.

You will be shown ONE code comment, the whitelist of shapes a comment is allowed to be, and — when available — the code that follows the comment in the file. Your only job is to decide which whitelist shape, if any, this comment fits, and call submit_verdict exactly once with your decision. You do not edit files, you do not have any tool but submit_verdict, and nothing you say outside that one tool call is read by anyone.

Judge strictly. The whitelist is closed and the default is "none" — a comment ships only if it clearly, unambiguously fits one shape. When in doubt, answer "none": a false negative (blocking a fine comment) costs the author a moment's annoyance; a false positive (approving a comment that should not exist) ships debt silently and is never caught again once it hits this cache.

For the "docstring on a public API declaration" shape specifically: it requires the comment to sit directly above a REAL public declaration (a function, type, or method other code outside this module would call), not a private one and not a bare statement. If the code shown after the comment does not look like a public declaration, or no following code is shown at all, do not approve this shape — the position claim is unverifiable, and an unverifiable claim is not a pass.

Call submit_verdict with the exact shape name as it appears in the whitelist below (character for character), or the literal string "none" if nothing fits. Keep your reason to one sentence.`;

export type KickoffPromptInput = {
	commentText: string;
	followingContext: string | undefined;
	whitelistDocText: string;
};

export function buildKickoffPrompt(input: KickoffPromptInput): string {
	const contextSection = input.followingContext
		? `The code immediately following this comment:\n\`\`\`\n${input.followingContext}\n\`\`\``
		: "No code follows this comment within what is visible to you. Treat any position-dependent shape claim (like a docstring) as unverifiable.";
	return [
		"The whitelist (verbatim from docs/comment-style.md):",
		"```",
		input.whitelistDocText.trim(),
		"```",
		"",
		"The comment to judge:",
		"```",
		input.commentText,
		"```",
		"",
		contextSection,
		"",
		"Call submit_verdict now with your decision.",
	].join("\n");
}
