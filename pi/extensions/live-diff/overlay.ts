import type {
	DiffMode,
	DiffStats,
	FileChange,
	Hunk,
	HunkLine,
	OverlayKey,
	OverlayModel,
	OverlayRow,
	OverlayStep,
	RenderRow,
	RenderSpan,
	RowTone,
	ViewerState,
} from "./types.ts";

/** Lines scrolled by page-up and page-down when the reducer has no display height. */
const VIEWER_PAGE_SIZE = 20;

function rowsForMode(
	mode: DiffMode,
	requestStats: DiffStats | null,
	overallStats: DiffStats | null,
): OverlayRow[] {
	const requestFiles = requestStats?.files ?? [];
	const overallFiles = overallStats?.files ?? [];
	const source =
		mode === "request"
			? requestFiles
			: (() => {
					const requestPaths = new Set(requestFiles.map((f) => f.path));
					return [
						...overallFiles.filter((f) => requestPaths.has(f.path)),
						...overallFiles.filter((f) => !requestPaths.has(f.path)),
					];
				})();
	const seenPaths = new Set<string>();
	const unique = source.filter((change) => {
		if (seenPaths.has(change.path)) {
			return false;
		}
		seenPaths.add(change.path);
		return true;
	});
	return unique.map((change) => ({ change, isFolded: true, hunks: null }));
}

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
	const mode: DiffMode =
		requestStats && requestStats.files.length > 0 ? "request" : "overall";
	return {
		mode,
		rows: rowsForMode(mode, requestStats, overallStats),
		cursor: 0,
		isLoadingPatch: false,
		viewer: null,
	};
}

/**
 * Rebuild rows after a mode flip, using fresh stats from the shell.
 *
 * @param model model whose mode is already flipped
 * @param requestStats current-request diff or null
 * @param overallStats overall worktree diff or null
 * @returns model with rows re-ranked for model.mode, fold state and fetched
 *   hunks preserved by path, cursor clamped
 */
export function rebuildRows(
	model: OverlayModel,
	requestStats: DiffStats | null,
	overallStats: DiffStats | null,
): OverlayModel {
	const previousByPath = new Map(
		model.rows.map((row) => [row.change.path, row]),
	);
	const rows = rowsForMode(model.mode, requestStats, overallStats).map(
		(row) => {
			const previous = previousByPath.get(row.change.path);
			return previous
				? { ...row, isFolded: previous.isFolded, hunks: previous.hunks }
				: row;
		},
	);
	const cursor = Math.min(model.cursor, Math.max(rows.length - 1, 0));
	return { ...model, rows, cursor };
}

/**
 * Total display lines a loaded diff occupies: one header plus one line per
 * hunk line, across every hunk. Used to clamp the viewer's scroll offset.
 *
 * @param hunks parsed hunks, or null while the patch is still loading
 * @returns total scrollable lines; 0 while loading
 */
function viewerLineCount(hunks: Hunk[] | null): number {
	if (hunks === null) {
		return 0;
	}
	return hunks.reduce((total, hunk) => total + 1 + hunk.lines.length, 0);
}

function clampViewerOffset(
	offset: number,
	hunks: Hunk[] | null,
	visibleLines = 1,
): number {
	const lineCount = viewerLineCount(hunks);
	const keptOnScreen = Math.max(Math.ceil(visibleLines * 0.8), 1);
	const maxOffset = Math.max(lineCount - keptOnScreen, 0);
	return Math.min(Math.max(offset, 0), maxOffset);
}

/**
 * Pure keyboard transition while the read-only viewer has focus. The list
 * underneath is untouched: mode, cursor and rows all pass through as-is.
 *
 * @param model current model; model.viewer is non-null
 * @param key mapped key
 * @returns next model plus at most one effect
 */
function reduceViewer(
	model: OverlayModel,
	key: OverlayKey,
	viewportHeight: number,
): OverlayStep {
	const viewer = model.viewer;
	if (viewer === null) {
		return { model, effect: null };
	}
	const pageSize = Math.max(viewportHeight - 1, 1);
	switch (key) {
		case "up":
			return {
				model: {
					...model,
					viewer: { ...viewer, offset: clampViewerOffset(viewer.offset - 1, viewer.hunks, viewportHeight) },
				},
				effect: null,
			};
		case "down":
			return {
				model: {
					...model,
					viewer: { ...viewer, offset: clampViewerOffset(viewer.offset + 1, viewer.hunks, viewportHeight) },
				},
				effect: null,
			};
		case "page-up":
			return {
				model: {
					...model,
					viewer: {
						...viewer,
						offset: clampViewerOffset(viewer.offset - pageSize, viewer.hunks, viewportHeight),
					},
				},
				effect: null,
			};
		case "page-down":
			return {
				model: {
					...model,
					viewer: {
						...viewer,
						offset: clampViewerOffset(viewer.offset + pageSize, viewer.hunks, viewportHeight),
					},
				},
				effect: null,
			};
		case "top":
			return { model: { ...model, viewer: { ...viewer, offset: 0 } }, effect: null };
		case "bottom":
			return {
				model: {
					...model,
					viewer: {
						...viewer,
						offset: clampViewerOffset(Number.POSITIVE_INFINITY, viewer.hunks, viewportHeight),
					},
				},
				effect: null,
			};
		case "next-file":
		case "prev-file": {
			const index = model.rows.findIndex((r) => r.change.path === viewer.path);
			if (index === -1) {
				return { model, effect: null };
			}
			if (model.rows.length < 2) {
				return { model, effect: null };
			}
			const step = key === "next-file" ? 1 : -1;
			const nextIndex =
				(index + step + model.rows.length) % model.rows.length;
			const nextPath = model.rows[nextIndex].change.path;
			return {
				model: {
					...model,
					cursor: nextIndex,
					viewer: { path: nextPath, hunks: null, offset: 0, isLoading: true },
				},
				effect: { kind: "load-patch", path: nextPath, mode: model.mode },
			};
		}
		case "close":
			return { model: { ...model, viewer: null }, effect: null };
		default:
			return { model, effect: null };
	}
}

/**
 * Pure keyboard transition for the overlay.
 *
 * @param model current model
 * @param key mapped key
 * @returns next model plus at most one effect; unknown transitions return
 *   the same model and null effect
 */
export function reduce(
	model: OverlayModel,
	key: OverlayKey,
	viewportHeight = 20,
): OverlayStep {
	if (model.viewer !== null) {
		return reduceViewer(model, key, viewportHeight);
	}
	// Column-scoped keys act on model.mode rather than model.rows[model.cursor],
	// so they must stay live even when the current column has zero rows —
	// otherwise a user who lands on an empty column is trapped there with only
	// close left to press.
	if (key === "close") {
		return { model, effect: { kind: "close" } };
	}
	if (key === "mode-left") {
		if (model.mode === "request") {
			return { model, effect: null };
		}
		return { model: { ...model, mode: "request" }, effect: null };
	}
	if (key === "mode-right") {
		if (model.mode === "overall") {
			return { model, effect: null };
		}
		return { model: { ...model, mode: "overall" }, effect: null };
	}
	if (model.rows.length === 0) {
		return { model, effect: null };
	}
	const row = model.rows[model.cursor];
	switch (key) {
		case "up":
			return { model: { ...model, cursor: Math.max(model.cursor - 1, 0) }, effect: null };
		case "down":
			return {
				model: { ...model, cursor: Math.min(model.cursor + 1, model.rows.length - 1) },
				effect: null,
			};
		case "open-diff":
			return {
				model: {
					...model,
					viewer: { path: row.change.path, hunks: null, offset: 0, isLoading: true },
				},
				effect: { kind: "load-patch", path: row.change.path, mode: model.mode },
			};
		case "open":
			return { model, effect: { kind: "open-in-nvim", path: row.change.path } };
		default:
			return { model, effect: null };
	}
}

/**
 * Fulfil a load-patch effect. When the viewer is open for `path`, fills its
 * hunks and clears isLoading. Otherwise attaches the hunks to the matching
 * row (legacy list-fold path, kept for callers that never open the viewer).
 *
 * @param model current model
 * @param mode mode the patch was requested for
 * @param path row path
 * @param hunks fetched hunks
 * @returns next model; unchanged when the target no longer exists or mode
 *   or viewer moved on (a stale fetch is dropped)
 */
export function applyPatch(
	model: OverlayModel,
	mode: DiffMode,
	path: string,
	hunks: Hunk[],
): OverlayModel {
	if (model.mode !== mode) {
		return model;
	}
	if (model.viewer !== null) {
		if (model.viewer.path !== path) {
			return model;
		}
		return { ...model, viewer: { ...model.viewer, hunks, isLoading: false } };
	}
	if (!model.rows.some((row) => row.change.path === path)) {
		return model;
	}
	let isAttached = false;
	return {
		...model,
		rows: model.rows.map((row) => {
			if (row.change.path !== path || isAttached) {
				return row;
			}
			isAttached = true;
			return { ...row, hunks, isFolded: false };
		}),
	};
}

function headerRow(
	model: OverlayModel,
	width: number,
	scrollHint: string | null,
): RenderRow {
	const label =
		model.mode === "request" ? "[request] overall" : " request [overall]";
	if (scrollHint === null) {
		return fit([{ text: label, tone: "header" }], width, false);
	}
	// The scroll hint is right-aligned after the label, with at least one
	// space of separation. When the two cannot both fit, the hint is dropped
	// WHOLE rather than clipped: a half-visible "↑ 4" is worse than nothing,
	// and the column names must stay legible over the hint every time.
	const gap = width - displayWidth(label) - displayWidth(scrollHint);
	if (gap < 1) {
		return fit([{ text: label, tone: "header" }], width, false);
	}
	return fit(
		[
			{ text: label, tone: "header" },
			{ text: " ".repeat(gap), tone: "header" },
			{ text: scrollHint, tone: "header" },
		],
		width,
		false,
	);
}

function hintRow(width: number): RenderRow {
	const base = "space expand · jk move · hl columns · ⏎ nvim · q close";
	return fit([{ text: base, tone: "hint" }], width, false);
}

interface BodyLine {
	row: RenderRow;
	/** Index into model.rows for a per-row line; null for a hunk or truncation line. */
	modelRowIndex: number | null;
}

function emptyStateText(mode: DiffMode): string {
	return mode === "request" ? "no changes in this request yet" : "no changes";
}

function buildBody(
	model: OverlayModel,
	width: number,
	isTruncated: boolean,
): BodyLine[] {
	if (model.rows.length === 0) {
		return [
			{
				row: fit(
					[{ text: emptyStateText(model.mode), tone: "hint" }],
					width,
					false,
				),
				modelRowIndex: null,
			},
		];
	}
	const lines: BodyLine[] = [];
	model.rows.forEach((row, index) => {
		const isSelected = index === model.cursor;
		const name = sanitize(rowName(row.change));
		const isUnfolded = !row.isFolded && row.hunks !== null;
		const marker = isUnfolded ? "▾ " : "▸ ";
		const spans: RenderSpan[] = [
			{ text: `${marker}${name}  `, tone: "path" },
		];
		const originLabel =
			model.mode === "overall" ? originText(row.change.origin) : null;
		if (originLabel !== null) {
			spans.push({ text: originLabel, tone: originTone(row.change.origin) });
		}
		if (row.change.isBinary) {
			spans.push({ text: "binary", tone: "binary" });
		} else {
			spans.push({
				text: `+${displayCount(row.change.additions)}`,
				tone: "added",
			});
			spans.push({ text: " ", tone: "path" });
			spans.push({
				text: `−${displayCount(row.change.deletions)}`,
				tone: "removed",
			});
		}
		lines.push({ row: fit(spans, width, isSelected), modelRowIndex: index });
		if (!isUnfolded || row.hunks === null) {
			return;
		}
		for (const hunk of row.hunks) {
			lines.push({
				row: fit(
					[{ text: sanitize(hunk.header), tone: "hunkHeader" }],
					width,
					false,
				),
				modelRowIndex: null,
			});
			for (const hunkLine of hunk.lines) {
				lines.push({
					row: fit(
						[
							{
								text: sanitize(hunkLine.origin + hunkLine.text),
								tone: hunkTone(hunkLine.origin),
							},
						],
						width,
						false,
					),
					modelRowIndex: null,
				});
			}
		}
	});
	if (isTruncated) {
		lines.push({
			row: fit(
				[{ text: "… more files (truncated)", tone: "truncation" }],
				width,
				false,
			),
			modelRowIndex: null,
		});
	}
	return lines;
}

/**
 * Window the body so the cursor line stays visible without recentring on
 * every move: the offset only advances or retreats the minimum amount
 * needed to keep the cursor's line inside [offset, offset + height).
 *
 * @param lines full body, in display order
 * @param height number of body lines to keep
 * @param cursorLineIndex index within `lines` that must stay visible
 * @returns the visible slice plus how many lines are hidden above and below
 */
function windowBody(
	lines: BodyLine[],
	height: number,
	cursorLineIndex: number,
): { visible: BodyLine[]; hiddenAbove: number; hiddenBelow: number } {
	if (height >= lines.length || height <= 0) {
		return { visible: lines, hiddenAbove: 0, hiddenBelow: 0 };
	}
	const maxOffset = Math.max(lines.length - height, 0);
	let offset = 0;
	if (cursorLineIndex >= offset + height) {
		offset = cursorLineIndex - height + 1;
	}
	if (cursorLineIndex < offset) {
		offset = cursorLineIndex;
	}
	offset = Math.min(Math.max(offset, 0), maxOffset);
	const visible = lines.slice(offset, offset + height);
	return { visible, hiddenAbove: offset, hiddenBelow: lines.length - offset - visible.length };
}

const VIEWER_RAIL = "│";

/**
 * Wrap a body row in the viewer's left and right rails, so every interior
 * line reads as inside the bordered rectangle rather than a bare line
 * floating between an unattached top border and hint.
 *
 * @param inner spans already fitted to `width - 2` (the rail width)
 * @param width panel width in display columns; must be >= 2
 * @returns a row whose spans concatenate to exactly `width` columns
 */
function railed(inner: RenderRow, width: number): RenderRow {
	const interiorWidth = Math.max(width - 2, 0);
	const reflowed = fit(inner.spans, interiorWidth, false);
	return {
		spans: [
			{ text: VIEWER_RAIL, tone: "header" },
			...reflowed.spans,
			{ text: VIEWER_RAIL, tone: "header" },
		],
		isSelected: false,
	};
}

/**
 * Border row for the viewer: rounded corners filled to exactly `width` with
 * horizontal rule, so the rectangle the rails draw actually closes rather
 * than a title floating in front of trailing padding. The top border
 * carries the file path plus "read-only", stated explicitly so the surface
 * is never mistaken for an editable nvim buffer, and an optional scroll
 * indicator. The bottom border carries the key hint. Either extra piece is
 * dropped WHOLE when it cannot fit beside the required text — a half
 * legend or a half "↑ 4" is worse than none, matching the list header's rule.
 */
function viewerBorderRow(
	side: "top" | "bottom",
	width: number,
	path: string,
	extra: string | null,
): RenderRow {
	const leftCorner = side === "top" ? "╭─ " : "╰─ ";
	const rightCorner = side === "top" ? "╮" : "╯";
	const titlePart =
		side === "top" ? `${sanitize(path)} ── read-only ──` : "";
	const headText = `${leftCorner}${titlePart}`;
	const fixedTailWidth = displayWidth(rightCorner);
	const availableForExtra = Math.max(
		width - displayWidth(headText) - fixedTailWidth,
		0,
	);
	// Bottom border: the key hint degrades by dropping trailing ·-separated
	// items whole, never mid-word. Top border: the scroll hint has no item
	// boundaries to trim, so it is dropped whole when it does not fit.
	const fittedExtra =
		extra === null
			? null
			: side === "bottom"
				? fitHintItems(extra, availableForExtra - 4)
				: displayWidth(`${extra} ── `) <= availableForExtra
					? extra
					: null;
	const extraPart = fittedExtra !== null ? `${fittedExtra} ── ` : "";
	const tailText = `${extraPart}${rightCorner}`;
	const ruleWidth = Math.max(
		width - displayWidth(headText) - displayWidth(tailText),
		0,
	);
	return fit(
		[
			{ text: headText, tone: "header" },
			{ text: "─".repeat(ruleWidth), tone: "header" },
			{ text: tailText, tone: "header" },
		],
		width,
		false,
	);
}

const VIEWER_HINT_TEXT = "j k scroll · ^d ^u page · g G ends · ] [ file · q back";
const VIEWER_HINT_ITEM_SEP = " · ";

/**
 * The largest whole-item prefix of a " · "-separated hint that fits within
 * `maxWidth` display columns, dropping trailing items rather than clipping
 * mid-word. Returns null when even the first item does not fit.
 *
 * @param hint the full " · "-joined hint text
 * @param maxWidth budget in display columns
 * @returns the fitted hint, or null when nothing fits
 */
function fitHintItems(hint: string, maxWidth: number): string | null {
	const items = hint.split(VIEWER_HINT_ITEM_SEP);
	for (let count = items.length; count > 0; count -= 1) {
		const candidate = items.slice(0, count).join(VIEWER_HINT_ITEM_SEP);
		if (displayWidth(candidate) <= maxWidth) {
			return candidate;
		}
	}
	return null;
}

function viewerBodyLines(hunks: Hunk[], width: number): RenderRow[] {
	const lines: RenderRow[] = [];
	for (const hunk of hunks) {
		lines.push(
			railed(
				fit([{ text: sanitize(hunk.header), tone: "hunkHeader" }], width, false),
				width,
			),
		);
		for (const hunkLine of hunk.lines) {
			lines.push(
				railed(
					fit(
						[
							{
								text: sanitize(hunkLine.origin + hunkLine.text),
								tone: hunkTone(hunkLine.origin),
							},
						],
						width,
						false,
					),
					width,
				),
			);
		}
	}
	return lines;
}

/**
 * Render the read-only viewer: a rounded, titled, explicitly-read-only
 * border around a scrollable slice of one file's diff.
 *
 * @param viewer the open viewer state
 * @param width panel width in display columns
 * @param visibleHeight rows available including both border lines
 * @returns rows padded or clipped to exactly `width`; no row is selected
 */
function renderViewer(
	viewer: ViewerState,
	width: number,
	visibleHeight: number,
): RenderRow[] {
	const bodyHeight = Math.max(
		(Number.isFinite(visibleHeight) ? visibleHeight : 24) - 2,
		1,
	);
	// Every path below emits exactly bodyHeight content rows so the box
	// always occupies visibleHeight rows total — a short message (loading,
	// binary, unavailable) is padded with blank framed rows exactly like a
	// short diff is, so the bottom border never drifts off a fixed height.
	const framed = (message: string): RenderRow[] => {
		const body = [railed(fit([{ text: message, tone: "hint" }], width, false), width)];
		while (body.length < bodyHeight) {
			body.push(railed(fit([], width, false), width));
		}
		return [
			viewerBorderRow("top", width, viewer.path, null),
			...body,
			viewerBorderRow("bottom", width, viewer.path, VIEWER_HINT_TEXT),
		];
	};
	if (viewer.isLoading) {
		return framed("opening…");
	}
	if (viewer.hunks === null) {
		return framed("patch unavailable");
	}
	if (viewer.hunks.length === 0) {
		return framed("binary file, no text diff");
	}
	const allLines = viewerBodyLines(viewer.hunks, width);
	const offset = clampViewerOffset(viewer.offset, viewer.hunks);
	const visible = allLines.slice(offset, offset + bodyHeight);
	const hiddenAbove = offset;
	const hiddenBelow = Math.max(allLines.length - offset - visible.length, 0);
	const scrollHint =
		hiddenAbove > 0 || hiddenBelow > 0
			? [
					hiddenAbove > 0 ? `↑ ${hiddenAbove}` : null,
					hiddenBelow > 0 ? `↓ ${hiddenBelow}` : null,
				]
					.filter((part): part is string => part !== null)
					.join("  ")
			: null;
	while (visible.length < bodyHeight) {
		visible.push(railed(fit([], width, false), width));
	}
	return [
		viewerBorderRow("top", width, viewer.path, scrollHint),
		...visible,
		viewerBorderRow("bottom", width, viewer.path, VIEWER_HINT_TEXT),
	];
}

/**
 * Render the model to tone-tagged rows for the overlay component.
 *
 * @param model current model
 * @param width panel width in display columns
 * @param isTruncated whether the current mode's stats were capped
 * @param visibleHeight maximum number of rows to emit, header and hint
 *   included; omitted or infinite means no windowing, which is every
 *   existing caller's behaviour
 * @returns rows padded or clipped to exactly `width` display columns, with
 *   the cursor row and only the cursor row selected. The header is always
 *   first and the hint always last; when the body is windowed, the cursor's
 *   row stays inside the visible slice and the window only scrolls when the
 *   cursor would otherwise leave it, and a hidden-rows count is appended to
 *   the header row, right-aligned, rather than competing with the hint
 *   row's key legend; when the header has no room for the count it is
 *   dropped whole rather than clipped mid-word, and the hint row is never
 *   touched. When model.viewer is open, a read-only bordered diff viewer is
 *   rendered instead of the list, and no row is selected.
 */
export function renderRows(
	model: OverlayModel,
	width: number,
	isTruncated = false,
	visibleHeight: number = Number.POSITIVE_INFINITY,
): RenderRow[] {
	if (model.viewer !== null) {
		return renderViewer(model.viewer, width, visibleHeight);
	}
	const body = buildBody(model, width, isTruncated);
	const bodyHeight = Math.max(visibleHeight - 2, 0);
	const cursorLineIndex = Math.max(
		body.findIndex((line) => line.modelRowIndex === model.cursor),
		0,
	);
	const { visible, hiddenAbove, hiddenBelow } = windowBody(
		body,
		bodyHeight,
		cursorLineIndex,
	);
	const scrollHint =
		hiddenAbove > 0 || hiddenBelow > 0
			? [
					hiddenAbove > 0 ? `↑ ${hiddenAbove}` : null,
					hiddenBelow > 0 ? `↓ ${hiddenBelow}` : null,
				]
					.filter((part): part is string => part !== null)
					.join("  ")
			: null;
	return [
		headerRow(model, width, scrollHint),
		...visible.map((line) => line.row),
		hintRow(width),
	];
}

/**
 * Render the model to plain terminal lines for the overlay component.
 *
 * @param model current model
 * @param width available columns
 * @param isTruncated whether the current mode's stats were capped
 * @returns printable lines, one string per row
 */
export function renderLines(
	model: OverlayModel,
	width: number,
	isTruncated = false,
): string[] {
	return renderRows(model, width, isTruncated).map((row) => {
		const spans = row.spans;
		const last = spans[spans.length - 1];
		const isPadOnly = last !== undefined && PAD_ONLY.test(last.text);
		const kept = isPadOnly ? spans.slice(0, -1) : spans;
		return kept.map((span) => span.text).join("");
	});
}

function hunkTone(origin: " " | "+" | "-"): RowTone {
	if (origin === "+") {
		return "hunkAdd";
	}
	return origin === "-" ? "hunkRemove" : "hunkContext";
}

function originText(origin: FileChange["origin"]): string | null {
	if (origin === "committed") {
		return "committed  ";
	}
	if (origin === "uncommitted") {
		return "uncommitted  ";
	}
	if (origin === "both") {
		return "committed+uncommitted  ";
	}
	return null;
}

function originTone(origin: FileChange["origin"]): RowTone {
	return origin === "uncommitted" ? "originUncommitted" : "originCommitted";
}

function fit(
	spans: RenderSpan[],
	width: number,
	isSelected: boolean,
): RenderRow {
	const fitted: RenderSpan[] = [];
	let used = 0;
	for (const span of spans) {
		if (used >= width) {
			break;
		}
		const text = clip(span.text, width - used);
		if (text.length > 0) {
			fitted.push({ text, tone: span.tone });
			used += displayWidth(text);
		}
	}
	if (used < width) {
		const tone = fitted.length > 0 ? fitted[fitted.length - 1].tone : "path";
		fitted.push({ text: " ".repeat(width - used), tone });
	}
	return { spans: fitted, isSelected };
}

function rowName(change: FileChange): string {
	return change.kind === "renamed" && change.renamedFrom !== null
		? `${change.renamedFrom} → ${change.path}`
		: change.path;
}

const DISPLAY_UNSAFE = /[\p{Cc}\p{Cf}\p{Zl}\p{Zp}]/gu;
const PAD_ONLY = /^ +$/u;

function sanitize(text: string): string {
	return text.replace(DISPLAY_UNSAFE, "");
}

function displayCount(value: number): number {
	return Number.isFinite(value) && value > 0 ? Math.floor(value) : 0;
}

const WIDE = /^(?:[\u1100-\u115F\u2E80-\u303E\u3041-\u33FF\u3400-\u4DBF\u4E00-\u9FFF\uA000-\uA4CF\uAC00-\uD7A3\uF900-\uFAFF\uFE30-\uFE6F\uFF00-\uFF60\uFFE0-\uFFE6]|[\u{1F300}-\u{1F64F}\u{1F900}-\u{1F9FF}\u{20000}-\u{2FFFD}])$/u;
const ZERO_WIDTH = /^[\p{Mn}\p{Me}]$/u;

function charWidth(char: string): number {
	if (ZERO_WIDTH.test(char)) {
		return 0;
	}
	return WIDE.test(char) ? 2 : 1;
}

function displayWidth(text: string): number {
	let total = 0;
	for (const char of text) {
		total += charWidth(char);
	}
	return total;
}

function clip(line: string, width: number): string {
	if (displayWidth(line) <= width) {
		return line;
	}
	let kept = "";
	let used = 0;
	for (const char of line) {
		const next = used + charWidth(char);
		if (next > width) {
			break;
		}
		kept += char;
		used = next;
	}
	return kept;
}

/**
 * Compact statusline badge text for both modes.
 *
 * @param requestStats current-request diff or null before the first request
 * @param overallStats overall worktree diff or null before the first refresh
 * @param overallLabel label for the second side: "branch" when a branch
 *   point was resolved, "all" when it fell back to the HEAD tree
 * @returns one-line badge such as "req +101 ~3 −8 · branch +214 ~9 −31", or
 *   "diff clean" when both are empty
 */
export function badgeText(
	requestStats: DiffStats | null,
	overallStats: DiffStats | null,
	overallLabel: "branch" | "all" = "all",
): string {
	const isEmpty = (stats: DiffStats | null): boolean =>
		stats === null || stats.files.length === 0;
	if (isEmpty(requestStats) && isEmpty(overallStats)) {
		return "diff clean";
	}
	const side = (label: string, stats: DiffStats): string => {
		const modifiedCount = stats.files.filter(
			(f) => f.kind === "modified",
		).length;
		return `${label} +${displayCount(stats.additions)} ~${displayCount(modifiedCount)} −${displayCount(stats.deletions)}`;
	};
	const parts: string[] = [];
	if (requestStats !== null) {
		parts.push(side("req", requestStats));
	}
	if (overallStats !== null) {
		parts.push(side(overallLabel, overallStats));
	}
	return parts.join(" · ");
}
