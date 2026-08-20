import { test } from "node:test";
import assert from "node:assert/strict";
import {
	applyPatch,
	badgeText,
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

test("TC-05 fold on a hunks-null row emits load-patch once, cached after applyPatch", () => {
	const model = initialModel(threeFileRequest(), null);
	const first = reduce(model, "fold");
	assert.deepEqual(first.effect, {
		kind: "load-patch",
		path: "a.ts",
		mode: "request",
	});
	assert.equal(first.model, model);

	const loaded = applyPatch(model, "request", "a.ts", hunks);
	assert.equal(loaded.rows[0].isFolded, false);
	assert.equal(loaded.rows[0].hunks, hunks);

	const second = reduce(loaded, "fold");
	assert.equal(second.effect, null);
	assert.equal(second.model.rows[0].isFolded, true);
	const third = reduce(second.model, "fold");
	assert.equal(third.effect, null);
	assert.equal(third.model.rows[0].isFolded, false);
});

test("TC-05 toggle-mode flips mode with rows unchanged; rebuildRows re-ranks and preserves state", () => {
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

	const flipped = reduce(model, "toggle-mode");
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

test("TC-09 empty stats give zero rows and inert keys except close", () => {
	const model = initialModel(null, stats([]));
	assert.equal(model.rows.length, 0);
	const keys: OverlayKey[] = ["up", "down", "fold", "toggle-mode", "open"];
	for (const key of keys) {
		const step = reduce(model, key);
		assert.equal(step.model, model);
		assert.equal(step.effect, null);
	}
	assert.deepEqual(reduce(model, "close").effect, { kind: "close" });
	assert.equal(badgeText(null, null), "diff clean");
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
	assert.equal(badgeText(broken, null), "req +0 ~1 −0");
	assert.equal(
		badgeText(stats([change("a.ts")], { additions: 10, deletions: Infinity }), null),
		"req +10 ~1 −0",
	);
	assert.equal(
		badgeText(stats([change("a.ts")], { additions: -100, deletions: 5 }), null),
		"req +0 ~1 −5",
	);
	assert.equal(
		badgeText(stats([change("a.ts")], { additions: -Infinity, deletions: 5 }), null),
		"req +0 ~1 −5",
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
		"fold",
		"toggle-mode",
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

function measureColumns(text: string): number {
	const wideRanges: [number, number][] = [
		[0x1100, 0x115f],
		[0x2e80, 0x303e],
		[0x3041, 0x33ff],
		[0x3400, 0x4dbf],
		[0x4e00, 0x9fff],
		[0xa000, 0xa4cf],
		[0xac00, 0xd7a3],
		[0xf900, 0xfaff],
		[0xfe30, 0xfe6f],
		[0xff00, 0xff60],
		[0xffe0, 0xffe6],
		[0x1f300, 0x1f64f],
		[0x1f900, 0x1f9ff],
		[0x20000, 0x2fffd],
	];
	let columns = 0;
	for (const char of text) {
		if (/[\p{Mn}\p{Me}]/u.test(char)) {
			continue;
		}
		const code = char.codePointAt(0) ?? 0;
		const isWide = wideRanges.some(([lo, hi]) => code >= lo && code <= hi);
		columns += isWide ? 2 : 1;
	}
	return columns;
}

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
		"req +101 ~2 −8 · all +214 ~1 −31",
	);
	assert.equal(badgeText(requestStats, null), "req +101 ~2 −8");
	assert.equal(badgeText(null, overallStats), "all +214 ~1 −31");
	assert.equal(
		badgeText(stats([]), overallStats),
		"req +0 ~0 −0 · all +214 ~1 −31",
	);
});

function testCharWidth(char: string): number {
	const code = char.codePointAt(0) ?? 0;
	if (/^[\p{Mn}\p{Me}]$/u.test(char)) {
		return 0;
	}
	const isWide =
		(code >= 0x1100 && code <= 0x115f) ||
		(code >= 0x2e80 && code <= 0x303e) ||
		(code >= 0x3041 && code <= 0x33ff) ||
		(code >= 0x3400 && code <= 0x4dbf) ||
		(code >= 0x4e00 && code <= 0x9fff) ||
		(code >= 0xa000 && code <= 0xa4cf) ||
		(code >= 0xac00 && code <= 0xd7a3) ||
		(code >= 0xf900 && code <= 0xfaff) ||
		(code >= 0xfe30 && code <= 0xfe6f) ||
		(code >= 0xff00 && code <= 0xff60) ||
		(code >= 0xffe0 && code <= 0xffe6) ||
		(code >= 0x1f300 && code <= 0x1f64f) ||
		(code >= 0x1f900 && code <= 0x1f9ff) ||
		(code >= 0x20000 && code <= 0x2fffd);
	return isWide ? 2 : 1;
}

function testDisplayWidth(text: string): number {
	let total = 0;
	for (const char of text) {
		total += testCharWidth(char);
	}
	return total;
}

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
