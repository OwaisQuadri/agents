import type {
	DiffMode,
	DiffStats,
	Hunk,
	OverlayKey,
	OverlayModel,
	OverlayStep,
} from "./types.ts";

/**
 * Build the initial overlay model from both stat sets, request mode first,
 * request files ranked above overall-only files.
 *
 * @param requestStats current-request diff or null when no snapshot exists
 * @param overallStats overall worktree diff
 * @returns model with cursor 0, everything folded
 */
export function initialModel(
	requestStats: DiffStats | null,
	overallStats: DiffStats | null,
): OverlayModel {
	// TODO(AGNT-0015.T06): request-first ranking, mode pick, all folded, cursor 0.
	throw new Error("unimplemented");
}

/**
 * Pure keyboard transition for the overlay.
 *
 * @param model current model
 * @param key mapped key
 * @returns next model plus at most one effect; unknown transitions return
 *   the same model and null effect
 */
export function reduce(model: OverlayModel, key: OverlayKey): OverlayStep {
	// TODO(AGNT-0015.T06): pure transitions per interfaces.md; ≤1 effect per step.
	throw new Error("unimplemented");
}

/**
 * Fulfil a load-patch effect: attach fetched hunks to the row and unfold it.
 *
 * @param model current model
 * @param mode mode the patch was requested for
 * @param path row path
 * @param hunks fetched hunks
 * @returns next model; unchanged when the row no longer exists or mode moved on
 */
export function applyPatch(
	model: OverlayModel,
	mode: DiffMode,
	path: string,
	hunks: Hunk[],
): OverlayModel {
	// TODO(AGNT-0015.T06): attach+unfold iff mode current and row exists; else drop.
	throw new Error("unimplemented");
}

/**
 * Render the model to plain terminal lines for the overlay component.
 *
 * @param model current model
 * @param width available columns
 * @returns printable lines, one string per row
 */
export function renderLines(model: OverlayModel, width: number): string[] {
	// TODO(AGNT-0015.T07): folded/unfolded rows, hint line, truncation marker,
	// ANSI-safe names, width clip.
	throw new Error("unimplemented");
}

/**
 * Compact statusline badge text for both modes.
 *
 * @param requestStats current-request diff or null before the first request
 * @param overallStats overall worktree diff or null before the first refresh
 * @returns one-line badge such as "req +101 ~3 −8 · all +214 ~9 −31", or
 *   "diff clean" when both are empty
 */
export function badgeText(
	requestStats: DiffStats | null,
	overallStats: DiffStats | null,
): string {
	// TODO(AGNT-0015.T07): "req +a ~m −d · all +a ~m −d" / "diff clean".
	throw new Error("unimplemented");
}
