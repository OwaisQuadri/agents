import { test } from "node:test";
import assert from "node:assert/strict";
import {
	applyPatch,
	badgeText,
	initialModel,
	rebuildRows,
	reduce,
	renderLines,
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
