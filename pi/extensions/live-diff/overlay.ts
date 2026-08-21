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

function isColumnScopedKey(key: OverlayKey): key is "close" | "mode-left" | "mode-right" {
	return key === "close" || key === "mode-left" || key === "mode-right";
}

function reduceColumnScoped(
	model: OverlayModel,
	key: "close" | "mode-left" | "mode-right",
): OverlayStep {
	if (key === "close") {
		return { model, effect: { kind: "close" } };
	}
	const mode: DiffMode = key === "mode-left" ? "request" : "overall";
	if (model.mode === mode) {
		return { model, effect: null };
	}
	return { model: { ...model, mode }, effect: null };
}

function hintGapBeside(
	label: string,
	hint: string,
	width: number,
): number | null {
	const gap = width - displayWidth(label) - displayWidth(hint);
	return gap < 1 ? null : gap;
}

function padToBodyHeight(
	rows: RenderRow[],
	bodyHeight: number,
	width: number,
): RenderRow[] {
	const body = [...rows];
	while (body.length < bodyHeight) {
		body.push(railed(fit([], width, false), width));
	}
	return body;
}

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
		case "open":
			return {
				model,
				effect: { kind: "open-in-nvim", path: viewer.path },
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
					viewer: {
						path: nextPath,
						isBinaryPath: model.rows[nextIndex].change.isBinary,
						hunks: null,
						offset: 0,
						isLoading: true,
					},
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
	if (isColumnScopedKey(key)) {
		return reduceColumnScoped(model, key);
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
					viewer: {
					path: row.change.path,
					isBinaryPath: row.change.isBinary,
					hunks: null,
					offset: 0,
					isLoading: true,
				},
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
		model.mode === "request" ? "[turn] overall" : " turn [overall]";
	if (scrollHint === null) {
		return fit([{ text: label, tone: "header" }], width, false);
	}
	const gap = hintGapBeside(label, scrollHint, width);
	if (gap === null) {
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
	selectableRowIndex: number | null;
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
				selectableRowIndex: null,
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
		lines.push({ row: fit(spans, width, isSelected), selectableRowIndex: index });
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
				selectableRowIndex: null,
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
					selectableRowIndex: null,
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
			selectableRowIndex: null,
		});
	}
	return lines;
}

function windowBody(
	lines: BodyLine[],
	height: number,
	cursorLineIndex: number,
): { visible: BodyLine[]; hiddenAbove: number; hiddenBelow: number } {
	if (height >= lines.length) {
		return { visible: lines, hiddenAbove: 0, hiddenBelow: 0 };
	}
	if (height <= 0) {
		return { visible: [], hiddenAbove: 0, hiddenBelow: lines.length };
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

const VIEWER_HINT_TEXT = "j k scroll · d u page · g G ends · ] [ file · ⏎ edit · esc back";
const VIEWER_HINT_ITEM_SEP = " · ";

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

function hunkStartLines(header: string): { oldLine: number; newLine: number } {
	const match = /@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/.exec(header);
	if (match === null) {
		return { oldLine: 1, newLine: 1 };
	}
	return { oldLine: Number(match[1]), newLine: Number(match[2]) };
}

function gutter(lineNumber: number | null, gutterWidth: number): string {
	const text = lineNumber === null ? "" : String(lineNumber);
	return text.padStart(gutterWidth, " ") + " ";
}

function viewerBodyLines(hunks: Hunk[], width: number): RenderRow[] {
	const lines: RenderRow[] = [];
	let highestLine = 1;
	for (const hunk of hunks) {
		const start = hunkStartLines(hunk.header);
		highestLine = Math.max(
			highestLine,
			start.oldLine + hunk.lines.length,
			start.newLine + hunk.lines.length,
		);
	}
	const gutterWidth = String(highestLine).length;
	for (const hunk of hunks) {
		const counters = hunkStartLines(hunk.header);
		lines.push(
			railed(
				fit([{ text: sanitize(hunk.header), tone: "hunkHeader" }], width, false),
				width,
			),
		);
		for (const hunkLine of hunk.lines) {
			const shown =
				hunkLine.origin === "-" ? counters.oldLine : counters.newLine;
			if (hunkLine.origin === "-") {
				counters.oldLine += 1;
			} else if (hunkLine.origin === "+") {
				counters.newLine += 1;
			} else {
				counters.oldLine += 1;
				counters.newLine += 1;
			}
			lines.push(
				railed(
					fit(
						[
							{ text: gutter(shown, gutterWidth), tone: "truncation" },
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

function renderViewer(
	viewer: ViewerState,
	width: number,
	visibleHeight: number,
): RenderRow[] {
	const bodyHeight = Math.max(
		(Number.isFinite(visibleHeight) ? visibleHeight : 24) - 2,
		1,
	);
	const framed = (message: string): RenderRow[] => {
		const body = padToBodyHeight(
			[railed(fit([{ text: message, tone: "hint" }], width, false), width)],
			bodyHeight,
			width,
		);
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
		return framed(
			viewer.isBinaryPath ? "binary file, no text diff" : "no textual changes",
		);
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
		body.findIndex((line) => line.selectableRowIndex === model.cursor),
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

// Generated from the Unicode character database (unicodedata 16.0.0): every
// codepoint whose East Asian Width is W or F, as 122 ranges. Derived, not
// curated by hand, because a display width that is wrong by one column
// silently breaks every row-width guarantee in this module.
const WIDE = /^[\u{1100}-\u{115F}\u{231A}-\u{231B}\u{2329}-\u{232A}\u{23E9}-\u{23EC}\u{23F0}\u{23F3}\u{25FD}-\u{25FE}\u{2614}-\u{2615}\u{2630}-\u{2637}\u{2648}-\u{2653}\u{267F}\u{268A}-\u{268F}\u{2693}\u{26A1}\u{26AA}-\u{26AB}\u{26BD}-\u{26BE}\u{26C4}-\u{26C5}\u{26CE}\u{26D4}\u{26EA}\u{26F2}-\u{26F3}\u{26F5}\u{26FA}\u{26FD}\u{2705}\u{270A}-\u{270B}\u{2728}\u{274C}\u{274E}\u{2753}-\u{2755}\u{2757}\u{2795}-\u{2797}\u{27B0}\u{27BF}\u{2B1B}-\u{2B1C}\u{2B50}\u{2B55}\u{2E80}-\u{2E99}\u{2E9B}-\u{2EF3}\u{2F00}-\u{2FD5}\u{2FF0}-\u{303E}\u{3041}-\u{3096}\u{3099}-\u{30FF}\u{3105}-\u{312F}\u{3131}-\u{318E}\u{3190}-\u{31E5}\u{31EF}-\u{321E}\u{3220}-\u{3247}\u{3250}-\u{A48C}\u{A490}-\u{A4C6}\u{A960}-\u{A97C}\u{AC00}-\u{D7A3}\u{F900}-\u{FAFF}\u{FE10}-\u{FE19}\u{FE30}-\u{FE52}\u{FE54}-\u{FE66}\u{FE68}-\u{FE6B}\u{FF01}-\u{FF60}\u{FFE0}-\u{FFE6}\u{16FE0}-\u{16FE4}\u{16FF0}-\u{16FF1}\u{17000}-\u{187F7}\u{18800}-\u{18CD5}\u{18CFF}-\u{18D08}\u{1AFF0}-\u{1AFF3}\u{1AFF5}-\u{1AFFB}\u{1AFFD}-\u{1AFFE}\u{1B000}-\u{1B122}\u{1B132}\u{1B150}-\u{1B152}\u{1B155}\u{1B164}-\u{1B167}\u{1B170}-\u{1B2FB}\u{1D300}-\u{1D356}\u{1D360}-\u{1D376}\u{1F004}\u{1F0CF}\u{1F18E}\u{1F191}-\u{1F19A}\u{1F200}-\u{1F202}\u{1F210}-\u{1F23B}\u{1F240}-\u{1F248}\u{1F250}-\u{1F251}\u{1F260}-\u{1F265}\u{1F300}-\u{1F320}\u{1F32D}-\u{1F335}\u{1F337}-\u{1F37C}\u{1F37E}-\u{1F393}\u{1F3A0}-\u{1F3CA}\u{1F3CF}-\u{1F3D3}\u{1F3E0}-\u{1F3F0}\u{1F3F4}\u{1F3F8}-\u{1F43E}\u{1F440}\u{1F442}-\u{1F4FC}\u{1F4FF}-\u{1F53D}\u{1F54B}-\u{1F54E}\u{1F550}-\u{1F567}\u{1F57A}\u{1F595}-\u{1F596}\u{1F5A4}\u{1F5FB}-\u{1F64F}\u{1F680}-\u{1F6C5}\u{1F6CC}\u{1F6D0}-\u{1F6D2}\u{1F6D5}-\u{1F6D7}\u{1F6DC}-\u{1F6DF}\u{1F6EB}-\u{1F6EC}\u{1F6F4}-\u{1F6FC}\u{1F7E0}-\u{1F7EB}\u{1F7F0}\u{1F90C}-\u{1F93A}\u{1F93C}-\u{1F945}\u{1F947}-\u{1F9FF}\u{1FA70}-\u{1FA7C}\u{1FA80}-\u{1FA89}\u{1FA8F}-\u{1FAC6}\u{1FACE}-\u{1FADC}\u{1FADF}-\u{1FAE9}\u{1FAF0}-\u{1FAF8}\u{20000}-\u{2FFFD}\u{30000}-\u{3FFFD}]$/u;
const ZERO_WIDTH = /^[\p{Mn}\p{Me}]$/u;

function charWidth(char: string): number {
	if (ZERO_WIDTH.test(char)) {
		return 0;
	}
	return WIDE.test(char) ? 2 : 1;
}

export function displayWidth(text: string): number {
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
 * @returns one-line badge such as "turn +101 ~3 −8 · branch +214 ~9 −31", or
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
		parts.push(side("turn", requestStats));
	}
	if (overallStats !== null) {
		parts.push(side(overallLabel, overallStats));
	}
	return parts.join(" · ");
}
