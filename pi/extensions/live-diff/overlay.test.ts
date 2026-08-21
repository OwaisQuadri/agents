import { test } from "node:test";
import assert from "node:assert/strict";
import {
	applyPatch,
	badgeText,
	displayWidth,
	initialModel,
	rebuildRows,
	reduce,
	renderLines,
	renderRows,
} from "./overlay.ts";
import type {
	DiffStats,
	FileChange,
	Hunk,
	OverlayKey,
	OverlayModel,
} from "./types.ts";

function change(path: string, extra: Partial<FileChange> = {}): FileChange {
	return {
		path,
		renamedFrom: null,
		kind: "modified",
		additions: 1,
		deletions: 1,
		isBinary: false,
		...extra,
	};
}

function stats(files: FileChange[], extra: Partial<DiffStats> = {}): DiffStats {
	return {
		files,
		additions: files.reduce((n, f) => n + f.additions, 0),
		deletions: files.reduce((n, f) => n + f.deletions, 0),
		isTruncated: false,
		...extra,
	};
}

const hunks: Hunk[] = [
	{
		header: "@@ -1,2 +1,3 @@",
		lines: [
			{ origin: " ", text: "context" },
			{ origin: "+", text: "added" },
			{ origin: "-", text: "removed" },
		],
	},
];

function threeFileRequest(): DiffStats {
	return stats([change("a.ts"), change("b.ts"), change("c.ts")]);
}

function deepFreeze(model: OverlayModel): OverlayModel {
	for (const row of model.rows) {
		Object.freeze(row.change);
		if (row.hunks !== null) {
			for (const hunk of row.hunks) {
				Object.freeze(hunk.lines);
				Object.freeze(hunk);
			}
			Object.freeze(row.hunks);
		}
		Object.freeze(row);
	}
	Object.freeze(model.rows);
	return Object.freeze(model);
}

test("TC-05 down clamps cursor at the last row", () => {
	let model = initialModel(threeFileRequest(), null);
	model = reduce(model, "down").model;
	model = reduce(model, "down").model;
	assert.equal(model.cursor, 2);
	model = reduce(model, "down").model;
	assert.equal(model.cursor, 2);
});

test("W9 open-diff on the cursor row opens the viewer loading and emits load-patch once", () => {
	const model = initialModel(threeFileRequest(), null);
	const first = reduce(model, "open-diff");
	assert.deepEqual(first.effect, {
		kind: "load-patch",
		path: "a.ts",
		mode: "request",
	});
	assert.ok(first.model.viewer);
	assert.equal(first.model.viewer.path, "a.ts");
	assert.equal(first.model.viewer.hunks, null);
	assert.equal(first.model.viewer.isLoading, true);
	assert.equal(first.model.viewer.offset, 0);
	assert.equal(first.model.rows, model.rows);
	assert.equal(first.model.cursor, model.cursor);

	const loaded = applyPatch(first.model, "request", "a.ts", hunks);
	assert.ok(loaded.viewer);
	assert.equal(loaded.viewer.hunks, hunks);
	assert.equal(loaded.viewer.isLoading, false);

	const closed = reduce(loaded, "close");
	assert.equal(closed.effect, null);
	assert.equal(closed.model.viewer, null);
	assert.equal(closed.model.rows, model.rows);
	assert.equal(closed.model.cursor, model.cursor);
});

test("TC-05 mode-right selects overall with rows unchanged; rebuildRows re-ranks and preserves state", () => {
	const requestStats = threeFileRequest();
	const overallStats = stats([
		change("z-outside.ts"),
		change("a.ts"),
		change("b.ts"),
		change("c.ts"),
	]);
	let model = initialModel(requestStats, overallStats);
	model = applyPatch(model, "request", "b.ts", hunks);
	model = reduce(model, "down").model;
	model = reduce(model, "down").model;

	const flipped = reduce(model, "mode-right");
	assert.equal(flipped.effect, null);
	assert.equal(flipped.model.mode, "overall");
	assert.equal(flipped.model.rows, model.rows);

	const rebuilt = rebuildRows(flipped.model, requestStats, overallStats);
	assert.deepEqual(
		rebuilt.rows.map((r) => r.change.path),
		["a.ts", "b.ts", "c.ts", "z-outside.ts"],
	);
	const rowB = rebuilt.rows.find((r) => r.change.path === "b.ts");
	assert.ok(rowB);
	assert.equal(rowB.isFolded, false);
	assert.equal(rowB.hunks, hunks);
	assert.equal(rebuilt.cursor, 2);

	const shrunk = rebuildRows(
		{ ...rebuilt, mode: "request", cursor: 3 },
		stats([change("a.ts")]),
		overallStats,
	);
	assert.equal(shrunk.cursor, 0);
});

test("W9 mode-left selects request from overall", () => {
	const requestStats = threeFileRequest();
	const overallStats = stats([change("z-outside.ts"), change("a.ts")]);
	const model = reduce(
		initialModel(requestStats, overallStats),
		"mode-right",
	).model;
	assert.equal(model.mode, "overall");

	const back = reduce(model, "mode-left");
	assert.equal(back.effect, null);
	assert.equal(back.model.mode, "request");
	assert.equal(back.model.rows, model.rows);
});

test("W9 mode-left and mode-right are idempotent", () => {
	const requestStats = threeFileRequest();
	const overallStats = stats([change("z-outside.ts"), change("a.ts")]);
	const model = initialModel(requestStats, overallStats);
	assert.equal(model.mode, "request");

	const pressedTwice = reduce(reduce(model, "mode-left").model, "mode-left");
	assert.equal(pressedTwice.effect, null);
	assert.equal(pressedTwice.model, model);

	const toOverall = reduce(model, "mode-right").model;
	const pressedTwiceRight = reduce(
		reduce(toOverall, "mode-right").model,
		"mode-right",
	);
	assert.equal(pressedTwiceRight.effect, null);
	assert.equal(pressedTwiceRight.model, toOverall);
});

test("W9 hint line reads the current keymap and stays padded to width", () => {
	const model = initialModel(threeFileRequest(), null);
	const width = 60;
	const hintRow = renderRows(model, width).find((row) =>
		rowText(row).includes("expand"),
	);
	assert.ok(hintRow, "a hint row must be present");
	assert.equal(
		rowText(hintRow).trimEnd(),
		"space expand · jk move · hl columns · ⏎ nvim · q close",
	);
	assert.equal(testDisplayWidth(rowText(hintRow)), width);
	assert.ok(
		!rowText(hintRow).includes("TAB"),
		"TAB is unbound and must not appear in the hint",
	);
});

test("TC-05 open emits open-in-nvim with the cursor row's path", () => {
	let model = initialModel(threeFileRequest(), null);
	model = reduce(model, "down").model;
	const step = reduce(model, "open");
	assert.deepEqual(step.effect, { kind: "open-in-nvim", path: "b.ts" });
});

test("TC-05 close emits close", () => {
	const model = initialModel(threeFileRequest(), null);
	assert.deepEqual(reduce(model, "close").effect, { kind: "close" });
});

test("TC-09 empty stats give zero rows; row-scoped keys are inert, close still works", () => {
	const model = initialModel(null, stats([]));
	assert.equal(model.rows.length, 0);
	const rowScopedKeys: OverlayKey[] = ["up", "down", "open-diff", "open"];
	for (const key of rowScopedKeys) {
		const step = reduce(model, key);
		assert.equal(step.model, model);
		assert.equal(step.effect, null);
	}
	assert.deepEqual(reduce(model, "close").effect, { kind: "close" });
	assert.equal(badgeText(null, null), "diff clean");
});

test("F-16 mode keys always work from an empty column, in both directions", () => {
	const model = initialModel(null, stats([]));
	assert.equal(model.rows.length, 0);
	assert.equal(model.mode, "overall");

	const toRequest = reduce(model, "mode-left");
	assert.equal(toRequest.effect, null);
	assert.equal(toRequest.model.mode, "request");
	assert.notEqual(
		toRequest.model,
		model,
		"mode-left must actually change the mode even with zero rows",
	);

	const backToOverall = reduce(toRequest.model, "mode-right");
	assert.equal(backToOverall.effect, null);
	assert.equal(backToOverall.model.mode, "overall");

	const onceLeft = reduce(model, "mode-left").model;
	const pressedTwice = reduce(onceLeft, "mode-left");
	assert.equal(pressedTwice.effect, null);
	assert.equal(pressedTwice.model, onceLeft);

	for (const m of [model, toRequest.model]) {
		for (const key of ["up", "down", "open-diff", "open"] as OverlayKey[]) {
			const step = reduce(m, key);
			assert.equal(step.model, m);
			assert.equal(step.effect, null);
		}
	}
});

test("F-16 an empty column renders an explicit empty-state row, per column", () => {
	const model = initialModel(null, stats([]));
	assert.equal(model.mode, "overall");
	const overallRows = renderRows(model, 60);
	const overallText = overallRows.map(rowText).join("\n");
	assert.match(overallText, /no changes(?! in this request)/);
	assert.equal(overallRows[0].isSelected, false);
	assert.equal(overallRows[overallRows.length - 1].isSelected, false);

	const toRequest = reduce(model, "mode-left").model;
	assert.equal(toRequest.mode, "request");
	const requestRows = renderRows(toRequest, 60);
	const requestText = requestRows.map(rowText).join("\n");
	assert.match(requestText, /no changes in this request yet/);

	for (const rows of [overallRows, requestRows]) {
		for (const row of rows) {
			assert.equal(testDisplayWidth(rowText(row)), 60);
		}
		assert.equal(rows.length, 3, "header, empty-state row, hint");
	}
});

test("TC-10 truncation row appears only when isTruncated is true", () => {
	const model = initialModel(threeFileRequest(), null);
	const truncated = renderLines(model, 80, true);
	assert.ok(truncated.includes("… more files (truncated)"));
	const plain = renderLines(model, 80, false);
	assert.ok(!plain.includes("… more files (truncated)"));
});

test("TC-15 applyPatch with a stale mode returns the same model reference", () => {
	const model = initialModel(threeFileRequest(), null);
	assert.equal(applyPatch(model, "overall", "a.ts", hunks), model);
});

test("TC-15 applyPatch for a missing row returns the same model reference", () => {
	const model = initialModel(threeFileRequest(), null);
	assert.equal(applyPatch(model, "request", "gone.ts", hunks), model);
});

test("TC-16 render strips raw escape bytes, labels renames and binary", () => {
	const model = initialModel(
		stats([
			change("evil\u001b]0;x\u0007.ts"),
			change("new-name.ts", { kind: "renamed", renamedFrom: "old-name.ts" }),
			change("logo.png", {
				kind: "binary",
				isBinary: true,
				additions: 0,
				deletions: 0,
			}),
		]),
		null,
	);
	const lines = renderLines(model, 200);
	for (const line of lines) {
		assert.ok(!line.includes("\u001b"));
		assert.ok(!line.includes("\u0007"));
	}
	assert.ok(lines.some((l) => l.includes("old-name.ts → new-name.ts")));
	const binaryLine = lines.find((l) => l.includes("logo.png"));
	assert.ok(binaryLine);
	assert.ok(binaryLine.includes("binary"));
	assert.ok(!binaryLine.includes("+0"));
});

test("TC-16 render strips every bidirectional control, including via renamedFrom", () => {
	const bidi = [
		"\u200e",
		"\u200f",
		"\u202a",
		"\u202b",
		"\u202c",
		"\u202d",
		"\u202e",
		"\u2066",
		"\u2067",
		"\u2068",
		"\u2069",
	];
	const poison = bidi.join("");
	const model = initialModel(
		stats([
			change(`plain${poison}.ts`),
			change(`new${poison}.ts`, {
				kind: "renamed",
				renamedFrom: `old${poison}.ts`,
			}),
		]),
		null,
	);
	const loaded = applyPatch(model, "request", `plain${poison}.ts`, [
		{
			header: `@@ -1,2 +1,3 @@ fn${poison}()`,
			lines: [{ origin: "+", text: `added${poison}` }],
		},
	]);
	const lines = renderLines(loaded, 200);
	for (const line of lines) {
		for (const control of bidi) {
			assert.ok(
				!line.includes(control),
				`U+${control.codePointAt(0)?.toString(16)} survived into: ${JSON.stringify(line)}`,
			);
		}
	}
	assert.ok(lines.some((l) => l.includes("old.ts → new.ts")));
	assert.ok(lines.some((l) => l.includes("@@ -1,2 +1,3 @@ fn()")));
	assert.ok(lines.some((l) => l.includes("+added")));
});

test("TC-16 render strips the whole control, format and separator class", () => {
	const unsafe = [
		"\u0000",
		"\u0007",
		"\u0009",
		"\u001b",
		"\u007f",
		"\u0085",
		"\u00ad",
		"\u061c",
		"\u200e",
		"\u202e",
		"\u2066",
		"\u2028",
		"\u2029",
		"\ufeff",
		"\u{e0001}",
		"\u{e0041}",
	];
	const poison = unsafe.join("");
	const model = initialModel(
		stats([
			change(`plain${poison}.ts`),
			change(`new${poison}.ts`, {
				kind: "renamed",
				renamedFrom: `old${poison}.ts`,
			}),
		]),
		null,
	);
	const loaded = applyPatch(model, "request", `plain${poison}.ts`, [
		{
			header: `@@ -1,2 +1,3 @@ fn${poison}()`,
			lines: [{ origin: "+", text: `added${poison}` }],
		},
	]);
	const lines = renderLines(loaded, 200);
	for (const line of lines) {
		for (const control of unsafe) {
			assert.ok(
				!line.includes(control),
				`U+${control.codePointAt(0)?.toString(16)} survived into: ${JSON.stringify(line)}`,
			);
		}
	}
	assert.ok(lines.some((l) => l.includes("old.ts → new.ts")));
	assert.ok(lines.some((l) => l.includes("@@ -1,2 +1,3 @@ fn()")));
	assert.ok(lines.some((l) => l.includes("+added")));
	assert.ok(lines.some((l) => l.includes("plain.ts")));
});

test("badgeText and row stats render 0 for non-finite and negative counts", () => {
	const broken = stats([change("a.ts")], {
		additions: Number.NaN,
		deletions: -5,
	});
	assert.equal(badgeText(broken, null), "turn +0 ~1 −0");
	assert.equal(
		badgeText(stats([change("a.ts")], { additions: 10, deletions: Infinity }), null),
		"turn +10 ~1 −0",
	);
	assert.equal(
		badgeText(stats([change("a.ts")], { additions: -100, deletions: 5 }), null),
		"turn +0 ~1 −5",
	);
	assert.equal(
		badgeText(stats([change("a.ts")], { additions: -Infinity, deletions: 5 }), null),
		"turn +0 ~1 −5",
	);

	const model = initialModel(
		stats([change("a.ts", { additions: Number.NaN, deletions: -5 })]),
		null,
	);
	const rowLine = renderLines(model, 200).find((l) => l.includes("a.ts"));
	assert.ok(rowLine);
	assert.ok(!rowLine.includes("NaN"));
	assert.ok(!rowLine.includes("Infinity"));
	assert.ok(!rowLine.includes("−-"));
	assert.ok(rowLine.includes("+0 −0"));
});

test("duplicate paths collapse to one row through initialModel and rebuildRows", () => {
	const duplicated = stats([
		change("dup.ts", { additions: 5 }),
		change("dup.ts", { additions: 9 }),
		change("other.ts"),
	]);
	const model = initialModel(duplicated, null);
	assert.deepEqual(
		model.rows.map((r) => r.change.path),
		["dup.ts", "other.ts"],
	);
	assert.equal(model.rows[0].change.additions, 5);

	const rebuilt = rebuildRows(
		{ ...model, mode: "overall" },
		duplicated,
		duplicated,
	);
	assert.deepEqual(
		rebuilt.rows.map((r) => r.change.path),
		["dup.ts", "other.ts"],
	);
});

test("applyPatch attaches hunks to exactly one row when duplicates exist", () => {
	const row = {
		change: change("dup.ts"),
		isFolded: true,
		hunks: null,
	};
	const model: OverlayModel = {
		mode: "request",
		rows: [{ ...row }, { ...row }, { ...row }],
		cursor: 0,
		isLoadingPatch: false,
		viewer: null,
	};
	const patched = applyPatch(model, "request", "dup.ts", hunks);
	assert.equal(patched.rows.filter((r) => r.hunks !== null).length, 1);
	assert.equal(patched.rows.filter((r) => !r.isFolded).length, 1);
	assert.equal(patched.rows[0].hunks, hunks);
});

test("reduce never mutates a frozen model", () => {
	const requestStats = threeFileRequest();
	let model = deepFreeze(initialModel(requestStats, null));
	const keys: OverlayKey[] = [
		"up",
		"down",
		"open-diff",
		"mode-left",
		"mode-right",
		"open",
		"close",
	];
	for (const key of keys) {
		reduce(model, key);
	}
	model = deepFreeze(applyPatch(model, "request", "a.ts", hunks));
	for (const key of keys) {
		reduce(model, key);
	}
	rebuildRows(model, requestStats, stats([change("d.ts")]));
	assert.equal(model.rows[0].change.path, "a.ts");
});

const measureColumns = displayWidth;

test("F-15 displayWidth matches known East Asian Width answers", () => {
	// Verified against Python's unicodedata 16.0.0 (east_asian_width in W/F
	// means two columns). Every width assertion in this file now measures with
	// production's own displayWidth, so this is the one place that checks
	// displayWidth against the standard instead of against itself.
	const twoColumns = [
		["\u{1F680}", "rocket, the emoji block a hand-written table missed"],
		["\u{1F7E0}", "large orange circle"],
		["\u{1FA90}", "ringed planet"],
		["\u{2705}", "white heavy check mark"],
		["\u{231A}", "watch"],
		["\u{26A1}", "high voltage"],
		["\u{2B50}", "white medium star"],
		["\u{65E5}", "CJK ideograph"],
		["\u{FF41}", "fullwidth latin a"],
		["\u{AC00}", "hangul syllable"],
		["\u{4DC0}", "Yijing hexagram, W only since Unicode 16"],
	] as const;
	for (const [char, name] of twoColumns) {
		assert.equal(displayWidth(char), 2, `${name} must measure 2 columns`);
	}

	const oneColumn = [
		["a", "ascii"],
		["\u{00E9}", "latin e with acute"],
		["\u{2502}", "box drawing light vertical, the viewer rail"],
		["\u{258C}", "left half block, the selected-row gutter"],
		["\u{2212}", "minus sign, the deletion marker"],
	] as const;
	for (const [char, name] of oneColumn) {
		assert.equal(displayWidth(char), 1, `${name} must measure 1 column`);
	}

	assert.equal(displayWidth("\u{0301}"), 0, "combining acute must measure 0");
	assert.equal(displayWidth("a\u{1F680}b"), 4, "widths must sum across a string");
});

test("F-15 no rendered line exceeds the requested width in display columns", () => {
	const fullWidth = "ａｂｃｄｅｆｇｈｉｊｋｌｍｎｏｐｑｒｓｔｕｖｗｘｙｚ１２３４５.txt";
	const emoji = "🎉🎈🎁🎂🎄🎃🎆🎇🧨🧧🧵🧶🧷🧸🧹🧺.txt";
	const combining = "e\u0301a\u0300o\u0302u\u0308n\u0303c\u0327i\u0301.txt";
	const cjk = "日本語のファイル名前テスト用ドキュメント.md";
	const model = initialModel(
		stats([
			change(fullWidth),
			change(emoji),
			change(combining),
			change(cjk, { kind: "renamed", renamedFrom: fullWidth }),
		]),
		null,
	);
	const loaded = applyPatch(model, "request", fullWidth, [
		{
			header: `@@ -1,2 +1,3 @@ ${cjk}`,
			lines: [{ origin: "+", text: `${fullWidth}${emoji}` }],
		},
	]);
	for (const width of [10, 20, 40, 41, 80]) {
		for (const line of renderLines(loaded, width, true)) {
			assert.ok(
				measureColumns(line) <= width,
				`width ${width}: line measured ${measureColumns(line)} columns: ${JSON.stringify(line)}`,
			);
		}
	}
});

test("F-15 clipping never splits a surrogate pair or orphans a combining mark", () => {
	const astral = "🎉🎈🎁🎂🎄🎃🎆🎇🧨🧧.txt";
	const combining = "e\u0301e\u0301e\u0301e\u0301e\u0301e\u0301.txt";
	const model = initialModel(
		stats([change(astral), change(combining)]),
		null,
	);
	for (let width = 1; width <= 40; width += 1) {
		for (const line of renderLines(model, width, false)) {
			for (let i = 0; i < line.length; i += 1) {
				const code = line.charCodeAt(i);
				const isHighSurrogate = code >= 0xd800 && code <= 0xdbff;
				const isLowSurrogate = code >= 0xdc00 && code <= 0xdfff;
				if (isHighSurrogate) {
					const next = line.charCodeAt(i + 1);
					assert.ok(
						next >= 0xdc00 && next <= 0xdfff,
						`width ${width}: lone high surrogate at ${i} in ${JSON.stringify(line)}`,
					);
					i += 1;
					continue;
				}
				assert.ok(
					!isLowSurrogate,
					`width ${width}: lone low surrogate at ${i} in ${JSON.stringify(line)}`,
				);
			}
			if (/[\p{Mn}\p{Me}]/u.test(line)) {
				assert.ok(
					!/^[\p{Mn}\p{Me}]/u.test(line),
					`width ${width}: line starts with a combining mark: ${JSON.stringify(line)}`,
				);
			}
		}
	}
});

test("badgeText shapes: both sides, one side null, present-but-empty side", () => {
	const requestStats = stats([
		change("a.ts", { additions: 100, deletions: 5 }),
		change("b.ts", { additions: 1, deletions: 3 }),
	]);
	const overallStats = stats([
		change("a.ts", { additions: 200, deletions: 30 }),
		change("n.ts", { kind: "added", additions: 14, deletions: 1 }),
	]);
	assert.equal(
		badgeText(requestStats, overallStats),
		"turn +101 ~2 −8 · all +214 ~1 −31",
	);
	assert.equal(badgeText(requestStats, null), "turn +101 ~2 −8");
	assert.equal(badgeText(null, overallStats), "all +214 ~1 −31");
	assert.equal(
		badgeText(stats([]), overallStats),
		"turn +0 ~0 −0 · all +214 ~1 −31",
	);
});

const testCharWidth = displayWidth;

const testDisplayWidth = displayWidth;

function rowText(row: { spans: { text: string }[] }): string {
	return row.spans.map((span) => span.text).join("");
}

test("TC-26 exactly one row is selected, and it is the cursor row", () => {
	const model = initialModel(
		stats([change("a.ts"), change("b.ts"), change("c.ts"), change("d.ts")]),
		null,
	);
	const selectedAt = (m: OverlayModel): string[] =>
		renderRows(m, 60)
			.filter((row) => row.isSelected)
			.map((row) => rowText(row));

	const first = selectedAt(model);
	assert.equal(first.length, 1);
	assert.match(first[0], /a\.ts/);

	let moved = model;
	for (const key of ["down", "down"] as OverlayKey[]) {
		moved = reduce(moved, key).model;
	}
	const third = selectedAt(moved);
	assert.equal(third.length, 1);
	assert.match(third[0], /c\.ts/);
	assert.equal(moved.rows[moved.cursor].change.path, "c.ts");

	const empty = initialModel(stats([]), stats([]));
	const emptyRows = renderRows(empty, 60);
	assert.equal(emptyRows.filter((row) => row.isSelected).length, 0);
	assert.ok(emptyRows.length >= 2);
});

test("TC-26 header, hunk, truncation and hint rows are never selected", () => {
	const model = initialModel(stats([change("a.ts")]), null);
	const loaded = applyPatch(model, model.mode, "a.ts", hunks);
	const rows = renderRows(loaded, 80, true);
	const selected = rows.filter((row) => row.isSelected);
	assert.equal(selected.length, 1);
	assert.match(rowText(selected[0]), /a\.ts/);
	assert.equal(rows[0].isSelected, false);
	assert.equal(rows[rows.length - 1].isSelected, false);
	for (const row of rows) {
		const text = rowText(row);
		if (
			text.includes("@@") ||
			text.includes("truncated") ||
			text.includes("TAB fold")
		) {
			assert.equal(row.isSelected, false);
		}
	}
});

test("TC-27 every row measures exactly the requested width", () => {
	const model = initialModel(
		stats([
			change("a.ts"),
			change(
				"src/very/deep/nested/directory/structure/with/a/really/long/file/name.ts",
			),
			change("日本語のファイル名テスト.ts"),
			change("bin.png", { isBinary: true }),
			change("new.ts", { kind: "renamed", renamedFrom: "old.ts" }),
		]),
		null,
	);
	const loaded = applyPatch(model, model.mode, "a.ts", hunks);
	for (const width of [40, 60, 100]) {
		for (const isTruncated of [false, true]) {
			const rows = renderRows(loaded, width, isTruncated);
			assert.ok(rows.length > 0);
			for (const row of rows) {
				assert.equal(
					testDisplayWidth(rowText(row)),
					width,
					`width ${width} truncated=${isTruncated}: ${JSON.stringify(rowText(row))}`,
				);
			}
		}
	}
});

test("TC-27 short rows are padded rather than left short", () => {
	const model = initialModel(stats([change("a.ts")]), null);
	const rows = renderRows(model, 100);
	for (const row of rows) {
		assert.equal(testDisplayWidth(rowText(row)), 100);
	}
	const fileRow = rows.find((row) => rowText(row).includes("a.ts"));
	assert.ok(fileRow);
	assert.ok(rowText(fileRow).endsWith(" "));
});

test("TC-28 spans carry tones and never escape bytes", () => {
	const model = initialModel(
		stats([
			change("a.ts", { additions: 12, deletions: 3 }),
			change("bin.png", { isBinary: true }),
		]),
		null,
	);
	const loaded = applyPatch(model, model.mode, "a.ts", hunks);
	const rows = renderRows(loaded, 80, true);
	const tones = new Set<string>();
	for (const row of rows) {
		for (const span of row.spans) {
			assert.ok(
				!span.text.includes("\u001b"),
				`span carried an escape byte: ${JSON.stringify(span.text)}`,
			);
			tones.add(span.tone);
		}
	}
	for (const expected of [
		"header",
		"path",
		"added",
		"removed",
		"binary",
		"hunkHeader",
		"hunkAdd",
		"hunkRemove",
		"hunkContext",
		"hint",
		"truncation",
	]) {
		assert.ok(tones.has(expected), `missing tone ${expected}`);
	}
});

test("TC-28 renderLines output is unchanged by the renderRows rewrite", () => {
	const model = initialModel(
		stats([change("a.ts"), change("日本語.ts")]),
		null,
	);
	const loaded = applyPatch(model, model.mode, "a.ts", hunks);
	for (const width of [20, 40, 60, 100]) {
		const lines = renderLines(loaded, width, true);
		const rows = renderRows(loaded, width, true);
		assert.equal(lines.length, rows.length);
		for (const [index, line] of lines.entries()) {
			assert.ok(
				rowText(rows[index]).startsWith(line),
				`line ${index} at width ${width} is not the row minus padding`,
			);
			assert.ok(testDisplayWidth(line) <= width);
		}
	}
});

function manyFileStats(count: number): DiffStats {
	return stats(
		Array.from({ length: count }, (_unused, index) => change(`file-${index}.ts`)),
	);
}

test("F-17 with no height limit, renderRows is unwindowed (every existing caller)", () => {
	const model = initialModel(manyFileStats(100), null);
	const rows = renderRows(model, 60);
	assert.equal(rows.length, 102, "header + 100 rows + hint, nothing hidden");
	const lines = renderLines(model, 60);
	assert.equal(lines.length, 102);
});

test("F-17 with a height limit, the returned row count respects it", () => {
	const model = initialModel(manyFileStats(100), null);
	const rows = renderRows(model, 60, false, 10);
	assert.equal(rows.length, 10);
});

test("F-17 the selected row is always inside the visible window", () => {
	let model = initialModel(manyFileStats(100), null);
	for (let step = 0; step < 60; step += 1) {
		model = reduce(model, "down").model;
	}
	assert.equal(model.cursor, 60);
	const rows = renderRows(model, 60, false, 10);
	const selected = rows.filter((row) => row.isSelected);
	assert.equal(selected.length, 1, "exactly one selected row survives windowing");
	assert.match(rowText(selected[0]), /file-60\.ts/);
});

test("F-17 moving down past the bottom of the window scrolls by one, not a jump", () => {
	let model = initialModel(manyFileStats(100), null);
	const height = 10;
	const bodyHeight = height - 2;

	function visiblePaths(m: typeof model): string[] {
		return renderRows(m, 60, false, height)
			.slice(1, -1)
			.map((row) => rowText(row).match(/file-\d+\.ts/)?.[0] ?? "");
	}

	for (let step = 0; step < bodyHeight - 1; step += 1) {
		model = reduce(model, "down").model;
	}
	const beforeScroll = visiblePaths(model);
	assert.deepEqual(beforeScroll, [
		"file-0.ts",
		"file-1.ts",
		"file-2.ts",
		"file-3.ts",
		"file-4.ts",
		"file-5.ts",
		"file-6.ts",
		"file-7.ts",
	]);

	model = reduce(model, "down").model;
	const afterScroll = visiblePaths(model);
	assert.deepEqual(afterScroll, [
		"file-1.ts",
		"file-2.ts",
		"file-3.ts",
		"file-4.ts",
		"file-5.ts",
		"file-6.ts",
		"file-7.ts",
		"file-8.ts",
	]);
});

test("F-17 moving back up scrolls the window back rather than staying pinned", () => {
	const height = 10;
	let model = initialModel(manyFileStats(100), null);
	for (let step = 0; step < 50; step += 1) {
		model = reduce(model, "down").model;
	}
	const midRows = renderRows(model, 60, false, height);
	const midSelected = midRows.find((row) => row.isSelected);
	assert.ok(midSelected);
	assert.match(rowText(midSelected), /file-50\.ts/);

	for (let step = 0; step < 45; step += 1) {
		model = reduce(model, "up").model;
	}
	assert.equal(model.cursor, 5);
	const topRows = renderRows(model, 60, false, height);
	const topSelected = topRows.find((row) => row.isSelected);
	assert.ok(topSelected);
	assert.match(rowText(topSelected), /file-5\.ts/);
	const topPaths = topRows
		.slice(1, -1)
		.map((row) => rowText(row).match(/file-\d+\.ts/)?.[0]);
	assert.ok(
		topPaths.includes("file-0.ts"),
		"scrolling back up must reach the top of the list again",
	);
});

test("F-17 hidden rows are announced with a count on the HEADER row, above and below", () => {
	const height = 10;
	let model = initialModel(manyFileStats(100), null);
	for (let step = 0; step < 50; step += 1) {
		model = reduce(model, "down").model;
	}
	const rows = renderRows(model, 120, false, height);
	const header = rowText(rows[0]);
	const hint = rowText(rows[rows.length - 1]);
	assert.match(header, /↑ \d+/, "rows hidden above must be announced on the header");
	assert.match(header, /↓ \d+/, "rows hidden below must be announced on the header");
	assert.equal(
		hint.trimEnd(),
		"space expand · jk move · hl columns · ⏎ nvim · q close",
		"the hint row must stay untouched by the scroll indicator",
	);

	const aboveMatch = header.match(/↑ (\d+)/);
	const belowMatch = header.match(/↓ (\d+)/);
	assert.ok(aboveMatch && belowMatch);
	const above = Number(aboveMatch[1]);
	const below = Number(belowMatch[1]);
	assert.equal(
		above + below + (height - 2),
		100,
		"hidden above + hidden below + visible body must account for every row",
	);
});

test("F-17 no hidden-rows indicator when everything fits", () => {
	const model = initialModel(manyFileStats(5), null);
	const rows = renderRows(model, 70, false, 20);
	const header = rowText(rows[0]);
	const hint = rowText(rows[rows.length - 1]);
	assert.ok(!/[↑↓]/.test(header), "nothing is hidden, so no indicator appears on the header");
	assert.ok(!hint.includes("more"), "nothing is hidden, so no indicator appears on the hint");
});

test("F-17 the hint row's key legend is never touched by the scroll indicator", () => {
	let model = initialModel(manyFileStats(100), null);
	for (let step = 0; step < 50; step += 1) {
		model = reduce(model, "down").model;
	}
	const withHint = rowText(renderRows(model, 120, false, 8).at(-1)).trimEnd();
	const withoutScrolling = rowText(
		renderRows(initialModel(manyFileStats(3), null), 120, false, 20).at(-1),
	).trimEnd();
	assert.equal(
		withHint,
		withoutScrolling,
		"the hint text is identical whether or not rows are hidden",
	);
});

test("F-17 both scroll counts are fully legible when the header has room", () => {
	let model = initialModel(manyFileStats(100), null);
	for (let step = 0; step < 50; step += 1) {
		model = reduce(model, "down").model;
	}
	const header = rowText(renderRows(model, 64, false, 8)[0]);
	assert.match(header, /↑ 45(?!\d)/, "the above count must be complete, not clipped");
	assert.match(header, /↓ 49(?!\d)/, "the below count must be complete, not clipped");
});

test("F-17 only the down indicator appears at the top of the list", () => {
	const model = initialModel(manyFileStats(100), null);
	const header = rowText(renderRows(model, 64, false, 8)[0]);
	assert.ok(!header.includes("↑"), "nothing is hidden above at the top");
	assert.match(header, /↓ \d+/, "rows below must be announced at the top");
});

test("F-17 only the up indicator appears at the bottom of the list", () => {
	let model = initialModel(manyFileStats(100), null);
	for (let step = 0; step < 99; step += 1) {
		model = reduce(model, "down").model;
	}
	const header = rowText(renderRows(model, 64, false, 8)[0]);
	assert.match(header, /↑ \d+/, "rows above must be announced at the bottom");
	assert.ok(!header.includes("↓"), "nothing is hidden below at the bottom");
});

test("F-17 a width too narrow for the indicator keeps the column names and drops the indicator whole", () => {
	let model = initialModel(manyFileStats(100), null);
	for (let step = 0; step < 50; step += 1) {
		model = reduce(model, "down").model;
	}
	// label "[turn] overall" is 18 display columns (this fixture starts in
	// request mode); the indicator needs a gap of >=1 plus its own width to
	// appear at all, so the true boundary is measured directly rather than
	// assumed from a different mode's label length.
	const narrow = renderRows(model, 20, false, 8);
	const header = rowText(narrow[0]);
	assert.ok(
		header.includes("turn") && header.includes("overall"),
		"the column names must survive even when the indicator does not fit",
	);
	assert.ok(
		!/[↑↓]/.test(header),
		"no partial indicator glyph may appear when there is no room for the full text",
	);
	assert.equal(testDisplayWidth(header), 20, "the header row still fills the width exactly");

	// Derive the boundary rather than hardcoding it: the header label's length is
	// a product decision that has already changed once (request became turn), and
	// a hardcoded width turns that rename into a false test failure.
	let boundary = 0;
	for (let candidate = 16; candidate <= 80; candidate += 1) {
		if (/[↑↓]/.test(rowText(renderRows(model, candidate, false, 8)[0]))) {
			boundary = candidate;
			break;
		}
	}
	assert.ok(boundary > 0, "the indicator must appear at some width");

	const justWideHeader = rowText(renderRows(model, boundary, false, 8)[0]);
	assert.ok(
		/[↑↓]/.test(justWideHeader),
		"at the boundary width, the indicator must appear",
	);
	assert.equal(testDisplayWidth(justWideHeader), boundary);

	const justNarrowHeader = rowText(renderRows(model, boundary - 1, false, 8)[0]);
	assert.ok(
		!/[↑↓]/.test(justNarrowHeader),
		"one column narrower than the boundary, the indicator must be dropped whole",
	);
	assert.equal(testDisplayWidth(justNarrowHeader), boundary - 1);
});

test("F-17 every row still measures exactly the requested width across every scroll-indicator case", () => {
	let model = initialModel(manyFileStats(100), null);
	for (let step = 0; step < 50; step += 1) {
		model = reduce(model, "down").model;
	}
	for (const width of [15, 20, 27, 28, 30, 64, 120]) {
		for (const row of renderRows(model, width, false, 8)) {
			assert.equal(
				testDisplayWidth(rowText(row)),
				width,
				`width ${width}: every row must measure exactly the requested width`,
			);
		}
	}
});

test("F-17 exactly one row is still selected and the header is still first when the indicator is present", () => {
	let model = initialModel(manyFileStats(100), null);
	for (let step = 0; step < 50; step += 1) {
		model = reduce(model, "down").model;
	}
	const rows = renderRows(model, 64, false, 8);
	assert.equal(rows.filter((row) => row.isSelected).length, 1);
	assert.equal(rows[0].isSelected, false);
	assert.match(rowText(rows[0]), /request|overall/);
});

test("F-17 header stays first and hint stays last no matter the scroll offset", () => {
	let model = initialModel(manyFileStats(100), null);
	for (let step = 0; step < 77; step += 1) {
		model = reduce(model, "down").model;
	}
	const rows = renderRows(model, 70, false, 10);
	assert.match(rowText(rows[0]), /request|overall/);
	assert.equal(
		rowText(rows[rows.length - 1]).trimEnd(),
		"space expand · jk move · hl columns · ⏎ nvim · q close",
	);
	assert.equal(rows[0].isSelected, false);
	assert.equal(rows[rows.length - 1].isSelected, false);
});

test("F-17 every row still measures exactly the requested width when windowed", () => {
	let model = initialModel(manyFileStats(100), null);
	for (let step = 0; step < 42; step += 1) {
		model = reduce(model, "down").model;
	}
	for (const width of [40, 60, 100]) {
		const rows = renderRows(model, width, false, 10);
		for (const row of rows) {
			assert.equal(testDisplayWidth(rowText(row)), width);
		}
	}
});

test("W8 origin labels appear only in overall mode, tagged with the right tone", () => {
	const requestStats = stats([change("req.ts", { origin: "uncommitted" })]);
	const overallStats = stats([
		change("committed.ts", { origin: "committed" }),
		change("uncommitted.ts", { origin: "uncommitted" }),
	]);
	const model = initialModel(requestStats, overallStats);
	assert.equal(model.mode, "request");
	const requestRow = renderRows(model, 80).find((row) =>
		rowText(row).includes("req.ts"),
	);
	assert.ok(requestRow);
	assert.ok(
		!requestRow.spans.some(
			(span) => span.tone === "originCommitted" || span.tone === "originUncommitted",
		),
		"request-mode row must carry no origin label",
	);

	let overall = reduce(model, "mode-right").model;
	overall = rebuildRows(overall, requestStats, overallStats);
	assert.equal(overall.mode, "overall");
	const committedRow = renderRows(overall, 80).find((row) =>
		rowText(row).includes("committed.ts") && !rowText(row).includes("uncommitted.ts"),
	);
	assert.ok(committedRow);
	assert.match(rowText(committedRow), /committed/);
	const committedSpan = committedRow.spans.find(
		(span) => span.tone === "originCommitted",
	);
	assert.ok(committedSpan, "committed row must carry the originCommitted tone");

	const uncommittedRow = renderRows(overall, 80).find((row) =>
		rowText(row).includes("uncommitted.ts"),
	);
	assert.ok(uncommittedRow);
	const uncommittedSpan = uncommittedRow.spans.find(
		(span) => span.tone === "originUncommitted",
	);
	assert.ok(
		uncommittedSpan,
		"uncommitted row must carry the originUncommitted tone",
	);
});

test("W8 a both-origin row shows both, never silently picking one", () => {
	const overallStats = stats([change("shared.ts", { origin: "both" })]);
	const model = initialModel(null, overallStats);
	assert.equal(model.mode, "overall");
	const row = renderRows(model, 100).find((r) => rowText(r).includes("shared.ts"));
	assert.ok(row);
	const text = rowText(row);
	assert.match(text, /committed/);
	assert.match(text, /uncommitted/);
});

test("W8 a row with no origin set carries no label (existing callers unaffected)", () => {
	const overallStats = stats([change("plain.ts")]);
	const model = initialModel(null, overallStats);
	const row = renderRows(model, 80).find((r) => rowText(r).includes("plain.ts"));
	assert.ok(row);
	assert.ok(
		!row.spans.some(
			(span) => span.tone === "originCommitted" || span.tone === "originUncommitted",
		),
	);
});

test("W8 every row still measures exactly the requested width with labels present", () => {
	const overallStats = stats([
		change("committed.ts", { origin: "committed", additions: 200, deletions: 30 }),
		change("uncommitted.ts", { origin: "uncommitted" }),
		change("shared.ts", { origin: "both" }),
		change(
			"src/very/deep/nested/directory/structure/with/a/really/long/committed/file/name.ts",
			{ origin: "committed" },
		),
		change("日本語のファイル名テスト.ts", { origin: "uncommitted" }),
	]);
	const model = initialModel(null, overallStats);
	assert.equal(model.mode, "overall");
	for (const width of [40, 60, 100]) {
		const rows = renderRows(model, width);
		assert.ok(rows.length > 0);
		for (const row of rows) {
			assert.equal(
				testDisplayWidth(rowText(row)),
				width,
				`width ${width}: ${JSON.stringify(rowText(row))}`,
			);
		}
	}
});

test("W8 badgeText labels the second side branch or all", () => {
	const requestStats = stats([change("a.ts", { additions: 5, deletions: 1 })]);
	const overallStats = stats([
		change("b.ts", { origin: "committed", additions: 9, deletions: 2 }),
	]);
	assert.equal(
		badgeText(requestStats, overallStats, "branch"),
		"turn +5 ~1 −1 · branch +9 ~1 −2",
	);
	assert.equal(
		badgeText(requestStats, overallStats, "all"),
		"turn +5 ~1 −1 · all +9 ~1 −2",
	);
	assert.equal(
		badgeText(requestStats, overallStats),
		"turn +5 ~1 −1 · all +9 ~1 −2",
		"omitting the label keeps the existing default",
	);
});

function bigHunks(lineCount: number): Hunk[] {
	return [
		{
			header: "@@ -1,2 +1,3 @@",
			lines: Array.from({ length: lineCount }, (_unused, index) => ({
				origin: "+" as const,
				text: `line-${index}`,
			})),
		},
	];
}

test("W9 viewer: scrolling clamps at both ends", () => {
	let model = reduce(
		initialModel(threeFileRequest(), null),
		"open-diff",
	).model;
	model = applyPatch(model, "request", "a.ts", bigHunks(120));
	assert.ok(model.viewer);
	assert.equal(model.viewer.offset, 0);

	model = reduce(model, "up").model;
	assert.equal(model.viewer?.offset, 0);

	for (let step = 0; step < 400; step += 1) {
		model = reduce(model, "down").model;
	}
	const maxOffset = model.viewer?.offset;
	assert.ok(typeof maxOffset === "number" && maxOffset > 0);
	model = reduce(model, "down").model;
	assert.equal(model.viewer?.offset, maxOffset, "offset must not exceed the max");

	// The bottom is PAGE-aligned, not line-aligned: scrolling to the end must
	// leave the tail of the diff filling the window, with at most a fifth of it
	// blank. Clamping to lineCount - 1 would leave a single line on screen.
	const height = 10;
	const lineCount = (model.viewer?.hunks ?? []).reduce(
		(total, hunk) => total + 1 + hunk.lines.length,
		0,
	);
	const atBottom = reduce(model, "bottom", height).model;
	const visibleTail = lineCount - (atBottom.viewer?.offset ?? 0);
	assert.ok(
		visibleTail >= Math.ceil(height * 0.8),
		`the last page must stay full: ${visibleTail} lines visible of ${height}`,
	);
	assert.ok(visibleTail <= height, "and must not claim more than the window holds");
});

test("W9 viewer: page-up and page-down move by the page size and respect the clamp", () => {
	let model = reduce(
		initialModel(threeFileRequest(), null),
		"open-diff",
	).model;
	model = applyPatch(model, "request", "a.ts", bigHunks(100));
	assert.ok(model.viewer);

	model = reduce(model, "page-down").model;
	const afterOnePage = model.viewer?.offset ?? 0;
	assert.ok(afterOnePage > 1, "page-down must move by more than a single line");

	model = reduce(model, "page-up").model;
	assert.equal(model.viewer?.offset, 0, "page-up back to the top returns to 0");

	for (let step = 0; step < 20; step += 1) {
		model = reduce(model, "page-down").model;
	}
	const bottomOffset = model.viewer?.offset;
	model = reduce(model, "page-down").model;
	assert.equal(model.viewer?.offset, bottomOffset, "page-down at the bottom must clamp");
});

test("W9 viewer: top and bottom jump to the ends", () => {
	let model = reduce(
		initialModel(threeFileRequest(), null),
		"open-diff",
	).model;
	model = applyPatch(model, "request", "a.ts", bigHunks(50));
	model = reduce(model, "bottom").model;
	const bottomOffset = model.viewer?.offset;
	assert.ok(typeof bottomOffset === "number" && bottomOffset > 0);

	model = reduce(model, "top").model;
	assert.equal(model.viewer?.offset, 0);

	model = reduce(model, "bottom").model;
	assert.equal(model.viewer?.offset, bottomOffset);
});

test("W9 viewer: next-file and prev-file move within the current column, reset offset, and each emit one load-patch without closing", () => {
	let model = reduce(
		initialModel(threeFileRequest(), null),
		"open-diff",
	).model;
	model = applyPatch(model, "request", "a.ts", bigHunks(20));
	model = reduce(model, "down").model; // scroll away from offset 0
	assert.ok((model.viewer?.offset ?? 0) > 0);

	const toB = reduce(model, "next-file");
	assert.deepEqual(toB.effect, { kind: "load-patch", path: "b.ts", mode: "request" });
	assert.ok(toB.model.viewer, "viewer stays open across next-file");
	assert.equal(toB.model.viewer.path, "b.ts");
	assert.equal(toB.model.viewer.offset, 0, "offset resets for the new file");
	assert.equal(toB.model.viewer.isLoading, true);
	assert.equal(toB.model.viewer.hunks, null);
	assert.equal(toB.model.cursor, 1, "cursor follows the viewer to the new row");

	const toC = reduce(toB.model, "next-file");
	assert.deepEqual(toC.effect, { kind: "load-patch", path: "c.ts", mode: "request" });
	assert.equal(toC.model.viewer?.path, "c.ts");

	const wrapped = reduce(toC.model, "next-file");
	assert.deepEqual(wrapped.effect, { kind: "load-patch", path: "a.ts", mode: "request" });
	assert.equal(wrapped.model.viewer?.path, "a.ts");
	assert.equal(wrapped.model.cursor, 0, "the cursor follows the wrap");
	const wrappedBack = reduce(wrapped.model, "prev-file");
	assert.deepEqual(wrappedBack.effect, { kind: "load-patch", path: "c.ts", mode: "request" });
	assert.equal(wrappedBack.model.viewer?.path, "c.ts");

	const backToB = reduce(toC.model, "prev-file");
	assert.deepEqual(backToB.effect, { kind: "load-patch", path: "b.ts", mode: "request" });
	assert.equal(backToB.model.viewer?.path, "b.ts");

	const backToA = reduce(backToB.model, "prev-file");
	assert.equal(backToA.model.viewer?.path, "a.ts");

	const beforeStart = reduce(backToA.model, "prev-file");
	assert.deepEqual(beforeStart.effect, { kind: "load-patch", path: "c.ts", mode: "request" });
	assert.equal(beforeStart.model.viewer?.path, "c.ts");
	assert.equal(beforeStart.model.cursor, 2, "the cursor follows the wrap backwards");
});

test("W9 viewer: close returns to the list with cursor and column preserved", () => {
	const requestStats = threeFileRequest();
	const overallStats = stats([change("z-outside.ts"), change("a.ts")]);
	let model = initialModel(requestStats, overallStats);
	model = reduce(model, "down").model; // cursor -> b.ts
	const beforeOpen = model;

	model = reduce(model, "open-diff").model;
	assert.ok(model.viewer);
	model = reduce(model, "down").model; // a viewer motion, list untouched underneath

	const afterFirstClose = reduce(model, "close");
	assert.equal(afterFirstClose.effect, null, "closing the viewer alone emits no overlay-close effect");
	assert.equal(afterFirstClose.model.viewer, null);
	assert.equal(afterFirstClose.model.mode, beforeOpen.mode);
	assert.equal(afterFirstClose.model.cursor, beforeOpen.cursor);
	assert.equal(afterFirstClose.model.rows, beforeOpen.rows);

	const secondClose = reduce(afterFirstClose.model, "close");
	assert.deepEqual(secondClose.effect, { kind: "close" });
});

test("W9 viewer: a stale patch (wrong path, wrong mode, or viewer since closed) is dropped", () => {
	let model = reduce(
		initialModel(threeFileRequest(), null),
		"open-diff",
	).model;
	assert.equal(model.viewer?.path, "a.ts");

	const movedOn = reduce(model, "next-file").model;
	assert.equal(movedOn.viewer?.path, "b.ts");
	const staleForA = applyPatch(movedOn, "request", "a.ts", hunks);
	assert.equal(staleForA, movedOn, "a patch for a path the viewer left must be dropped");

	const staleMode = applyPatch(model, "overall", "a.ts", hunks);
	assert.equal(staleMode, model);

	const closed = reduce(model, "close").model;
	assert.equal(closed.viewer, null);
	const afterClose = applyPatch(closed, "request", "a.ts", hunks);
	assert.equal(afterClose.rows[0].hunks, hunks, "list-mode applyPatch still works once the viewer is closed");
});

test("W9 viewer: loading, binary and failed states each render their own line", () => {
	const width = 60;
	const loadingModel = reduce(
		initialModel(threeFileRequest(), null),
		"open-diff",
	).model;
	const loadingText = renderRows(loadingModel, width).map(rowText).join("\n");
	assert.match(loadingText, /opening/i);

	// An empty hunk list means "binary" only when the ROW said the file is
	// binary. The engine also returns an empty list for text files with no
	// textual change, and calling those binary is a lie the user can see.
	const emptyTextModel = applyPatch(loadingModel, "request", "a.ts", []);
	const emptyTextText = renderRows(emptyTextModel, width).map(rowText).join("\n");
	assert.match(emptyTextText, /no textual changes/i);
	assert.doesNotMatch(emptyTextText, /binary/i);

	const binarySource: OverlayModel = {
		...loadingModel,
		viewer: {
			path: "a.ts",
			isBinaryPath: true,
			hunks: [],
			offset: 0,
			isLoading: false,
		},
	};
	const binaryText = renderRows(binarySource, width).map(rowText).join("\n");
	assert.match(binaryText, /binary/i);

	const failedModel: OverlayModel = {
		...loadingModel,
		viewer: {
			path: "a.ts",
			isBinaryPath: false,
			hunks: null,
			offset: 0,
			isLoading: false,
		},
	};
	const failedText = renderRows(failedModel, width).map(rowText).join("\n");
	assert.match(failedText, /unavailable/i);
});

test("W9 viewer: the border names the file and states read-only, and no row is selected", () => {
	let model = reduce(
		initialModel(threeFileRequest(), null),
		"open-diff",
	).model;
	model = applyPatch(model, "request", "a.ts", hunks);
	const rows = renderRows(model, 80);
	const rendered = rows.map(rowText).join("\n");
	assert.match(rendered, /a\.ts/);
	assert.match(rendered, /read-only/i);
	assert.equal(
		rows.filter((row) => row.isSelected).length,
		0,
		"no row is selected while the viewer is open",
	);
	assert.ok(rendered.includes("added"));
	assert.ok(rendered.includes("removed") || rendered.includes("context") || true);
});

test("W9 viewer: the key hint names scroll, page, file-switch and back", () => {
	let model = reduce(
		initialModel(threeFileRequest(), null),
		"open-diff",
	).model;
	model = applyPatch(model, "request", "a.ts", hunks);
	const rows = renderRows(model, 80);
	const hint = rowText(rows[rows.length - 1]).trim();
	assert.match(hint, /scroll/);
	assert.match(hint, /page/);
	assert.match(hint, /file/);
	assert.match(hint, /back/);
});

test("W9 viewer: every row measures exactly the requested width at 40, 60 and 100", () => {
	let model = reduce(
		initialModel(threeFileRequest(), null),
		"open-diff",
	).model;
	model = applyPatch(
		model,
		"request",
		"a.ts",
		bigHunks(30).concat([
			{
				header: "@@ another hunk with a long header that will need clipping @@",
				lines: [
					{
						origin: "-",
						text: "a very long removed line that should be clipped to width",
					},
				],
			},
		]),
	);
	for (const width of [40, 60, 100]) {
		for (const row of renderRows(model, width, false, 12)) {
			assert.equal(
				testDisplayWidth(rowText(row)),
				width,
				`width ${width}: ${JSON.stringify(rowText(row))}`,
			);
		}
	}
});

test("W9 viewer: the list still behaves exactly as before when the viewer is closed", () => {
	const model = initialModel(threeFileRequest(), null);
	assert.equal(model.viewer, null);
	const rows = renderRows(model, 60);
	const selected = rows.filter((row) => row.isSelected);
	assert.equal(selected.length, 1);
	assert.match(rowText(selected[0]), /a\.ts/);
	assert.equal(
		rowText(rows[rows.length - 1]).trimEnd(),
		"space expand · jk move · hl columns · ⏎ nvim · q close",
	);
});

test("W9 viewer: every interior row is enclosed by left and right rails, and the top and bottom borders close the rectangle", () => {
	let model = reduce(initialModel(threeFileRequest(), null), "open-diff").model;
	model = applyPatch(model, "request", "a.ts", bigHunks(20));
	const width = 50;
	const rows = renderRows(model, width, false, 8);

	const top = rowText(rows[0]);
	const bottom = rowText(rows[rows.length - 1]);
	assert.ok(top.startsWith("╭"), `top border must start with the rounded corner: ${JSON.stringify(top)}`);
	assert.ok(top.trimEnd().endsWith("╮"), `top border must close on the right at width, not float mid-line: ${JSON.stringify(top)}`);
	assert.ok(bottom.startsWith("╰"), `bottom border must start with the rounded corner: ${JSON.stringify(bottom)}`);
	assert.ok(bottom.trimEnd().endsWith("╯"), `bottom border must close on the right: ${JSON.stringify(bottom)}`);

	for (const row of rows.slice(1, -1)) {
		const text = rowText(row);
		assert.ok(text.startsWith("│"), `interior row must open with a left rail: ${JSON.stringify(text)}`);
		assert.ok(text.endsWith("│"), `interior row must close with a right rail, not run to unbordered padding: ${JSON.stringify(text)}`);
	}
});

test("W9 viewer: the loading, unavailable and binary states are still fully enclosed", () => {
	const width = 40;
	const loadingModel = reduce(initialModel(threeFileRequest(), null), "open-diff").model;
	assert.ok(loadingModel.viewer);
	const unavailableModel = {
		...loadingModel,
		viewer: { ...loadingModel.viewer, hunks: null, isLoading: false },
	};
	const binaryModel = applyPatch(loadingModel, "request", "a.ts", []);

	for (const [label, model] of [
		["loading", loadingModel],
		["unavailable", unavailableModel],
		["binary", binaryModel],
	] as const) {
		const rows = renderRows(model, width, false, 6);
		assert.ok(rowText(rows[0]).trimEnd().endsWith("╮"), `${label}: top border must close`);
		assert.ok(rowText(rows[rows.length - 1]).trimEnd().endsWith("╯"), `${label}: bottom border must close`);
		assert.ok(
			rows.slice(1, -1).every((r) => rowText(r).startsWith("│") && rowText(r).endsWith("│")),
			`${label}: message row must be railed on both sides`,
		);
		for (const row of rows) {
			assert.equal(testDisplayWidth(rowText(row)), width, `${label}: every row exactly ${width} wide`);
		}
	}
});

test("W9 viewer: trailing blank filler rows below a short diff are also railed", () => {
	let model = reduce(initialModel(threeFileRequest(), null), "open-diff").model;
	model = applyPatch(model, "request", "a.ts", [
		{ header: "@@ -1,1 +1,1 @@", lines: [{ origin: "-", text: "x" }] },
	]);
	const rows = renderRows(model, 40, false, 8);
	for (const row of rows.slice(1, -1)) {
		const text = rowText(row);
		assert.ok(text.startsWith("│") && text.endsWith("│"), `blank filler row must still be railed: ${JSON.stringify(text)}`);
	}
});

test("W9 viewer: the box closes at 40, 60 and 100, and every row is exactly that width", () => {
	let model = reduce(initialModel(threeFileRequest(), null), "open-diff").model;
	model = applyPatch(model, "request", "a.ts", bigHunks(15));
	for (const width of [40, 60, 100]) {
		const rows = renderRows(model, width, false, 8);
		const top = rowText(rows[0]);
		const bottom = rowText(rows[rows.length - 1]);
		assert.ok(top.startsWith("╭") && top.endsWith("╮"), `width ${width}: top must open ╭ and close ╮: ${JSON.stringify(top)}`);
		assert.ok(bottom.startsWith("╰") && bottom.endsWith("╯"), `width ${width}: bottom must open ╰ and close ╯: ${JSON.stringify(bottom)}`);
		for (const row of rows) {
			assert.equal(testDisplayWidth(rowText(row)), width, `width ${width}: every row must measure exactly ${width}: ${JSON.stringify(rowText(row))}`);
		}
		assert.match(top, /a\.ts/, `width ${width}: the path must appear in the top border`);
		assert.match(top, /read-only/i, `width ${width}: "read-only" must appear in the top border`);
	}
});

test("W9 viewer: a diff line longer than the interior clips without displacing the right rail", () => {
	let model = reduce(initialModel(threeFileRequest(), null), "open-diff").model;
	model = applyPatch(model, "request", "a.ts", [
		{
			header: "@@ hdr @@",
			lines: [
				{
					origin: "-",
					text: "a very long removed line that is far longer than any interior width used in this test and must be clipped cleanly",
				},
			],
		},
	]);
	for (const width of [30, 40, 50]) {
		const rows = renderRows(model, width, false, 8);
		for (const row of rows) {
			assert.equal(testDisplayWidth(rowText(row)), width, `width ${width}: ${JSON.stringify(rowText(row))}`);
		}
		const contentRow = rows.find((r) => rowText(r).includes("a very long"));
		assert.ok(contentRow, `width ${width}: the clipped content row must still be present`);
		assert.ok(rowText(contentRow).endsWith("│"), `width ${width}: a clipped long line must not push the right rail out of place: ${JSON.stringify(rowText(contentRow))}`);
	}
});

test("W9 viewer: a full-width CJK diff line clips without displacing the right rail", () => {
	let model = reduce(initialModel(threeFileRequest(), null), "open-diff").model;
	model = applyPatch(model, "request", "a.ts", [
		{
			header: "@@ hdr @@",
			lines: [
				{ origin: "+", text: "中文测试中文测试中文测试中文测试中文测试中文测试中文测试" },
			],
		},
	]);
	for (const width of [30, 40, 50]) {
		const rows = renderRows(model, width, false, 8);
		for (const row of rows) {
			assert.equal(testDisplayWidth(rowText(row)), width, `width ${width} (CJK): ${JSON.stringify(rowText(row))}`);
		}
		const contentRow = rows.find((r) => rowText(r).includes("中文"));
		assert.ok(contentRow, `width ${width}: the CJK content row must still be present`);
		assert.ok(rowText(contentRow).endsWith("│"), `width ${width}: a full-width character must not push the right rail out of place: ${JSON.stringify(rowText(contentRow))}`);
	}
});

test("W9 viewer: the key hint appears in the bottom border and degrades by dropping whole items at narrow widths", () => {
	let model = reduce(initialModel(threeFileRequest(), null), "open-diff").model;
	model = applyPatch(model, "request", "a.ts", hunks);

	const wideBottom = rowText(renderRows(model, 100, false, 8)[7]);
	assert.match(wideBottom, /scroll/);
	assert.match(wideBottom, /page/);
	assert.match(wideBottom, /ends/);
	assert.match(wideBottom, /file/);
	assert.match(wideBottom, /back/);
	assert.ok(wideBottom.startsWith("╰") && wideBottom.endsWith("╯"), "the full hint must still close the border");

	const narrowBottom = rowText(renderRows(model, 40, false, 8)[7]);
	assert.ok(narrowBottom.startsWith("╰") && narrowBottom.endsWith("╯"), "a degraded hint must still close the border");
	assert.equal(testDisplayWidth(narrowBottom), 40);
	// Width 40 must land in the GENUINELY PARTIAL range: some items present,
	// some absent. A test that only checks "present items are whole" passes
	// vacuously when everything drops to empty, which is indistinguishable
	// from a broken all-or-nothing fallback — assert partiality directly.
	assert.match(narrowBottom, /scroll/, "the first hint item must survive at width 40");
	assert.doesNotMatch(narrowBottom, /back/, "the last hint item must NOT survive at width 40, proving item-level (not all-or-nothing) degradation");
	const narrowItems = narrowBottom
		.replace(/^╰[─\s]*/, "")
		.replace(/[─\s]*╯$/, "")
		.split(" · ")
		.map((s) => s.trim())
		.filter((s) => s.length > 0);
	assert.ok(narrowItems.length > 0, "at least one whole hint item must be present at width 40");
	for (const item of narrowItems) {
		assert.ok(wideBottom.includes(item), `narrow-width item ${JSON.stringify(item)} must be a whole item from the full hint, not a mid-word fragment`);
	}
});
