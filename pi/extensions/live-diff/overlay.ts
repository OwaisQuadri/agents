import type {
	DiffMode,
	DiffStats,
	FileChange,
	Hunk,
	OverlayKey,
	OverlayModel,
	OverlayRow,
	OverlayStep,
	RenderRow,
	RenderSpan,
	RowTone,
} from "./types.ts";

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
 * Pure keyboard transition for the overlay.
 *
 * @param model current model
 * @param key mapped key
 * @returns next model plus at most one effect; unknown transitions return
 *   the same model and null effect
 */
export function reduce(model: OverlayModel, key: OverlayKey): OverlayStep {
	if (key === "close") {
		return { model, effect: { kind: "close" } };
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
		case "fold":
			if (row.hunks === null) {
				return {
					model,
					effect: { kind: "load-patch", path: row.change.path, mode: model.mode },
				};
			}
			return {
				model: {
					...model,
					rows: model.rows.map((r, i) =>
						i === model.cursor ? { ...r, isFolded: !r.isFolded } : r,
					),
				},
				effect: null,
			};
		case "toggle-mode":
			return {
				model: {
					...model,
					mode: model.mode === "request" ? "overall" : "request",
				},
				effect: null,
			};
		case "open":
			return { model, effect: { kind: "open-in-nvim", path: row.change.path } };
		default:
			return { model, effect: null };
	}
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
	if (model.mode !== mode) {
		return model;
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

/**
 * Render the model to tone-tagged rows for the overlay component.
 *
 * @param model current model
 * @param width panel width in display columns
 * @param isTruncated whether the current mode's stats were capped
 * @returns one row per visual line, each padded or clipped to exactly `width`
 *   display columns, with the cursor row and only the cursor row selected
 */
export function renderRows(
	model: OverlayModel,
	width: number,
	isTruncated = false,
): RenderRow[] {
	const rows: RenderRow[] = [];
	rows.push(
		fit(
			[
				{
					text:
						model.mode === "request"
							? "[request] overall"
							: " request [overall]",
					tone: "header",
				},
			],
			width,
			false,
		),
	);
	model.rows.forEach((row, index) => {
		const isSelected = index === model.cursor;
		const name = sanitize(rowName(row.change));
		const isUnfolded = !row.isFolded && row.hunks !== null;
		const marker = isUnfolded ? "▾ " : "▸ ";
		const spans: RenderSpan[] = [
			{ text: `${marker}${name}  `, tone: "path" },
		];
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
		rows.push(fit(spans, width, isSelected));
		if (!isUnfolded || row.hunks === null) {
			return;
		}
		for (const hunk of row.hunks) {
			rows.push(
				fit(
					[{ text: sanitize(hunk.header), tone: "hunkHeader" }],
					width,
					false,
				),
			);
			for (const hunkLine of hunk.lines) {
				rows.push(
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
				);
			}
		}
	});
	if (isTruncated) {
		rows.push(
			fit(
				[{ text: "… more files (truncated)", tone: "truncation" }],
				width,
				false,
			),
		);
	}
	rows.push(
		fit(
			[
				{
					text:
						"TAB fold · ↑↓ move · ⏎ open in nvim · tab request/overall · q close",
					tone: "hint",
				},
			],
			width,
			false,
		),
	);
	return rows;
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
 * @returns one-line badge such as "req +101 ~3 −8 · all +214 ~9 −31", or
 *   "diff clean" when both are empty
 */
export function badgeText(
	requestStats: DiffStats | null,
	overallStats: DiffStats | null,
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
		parts.push(side("all", overallStats));
	}
	return parts.join(" · ");
}
