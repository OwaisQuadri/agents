import assert from "node:assert/strict";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { test } from "node:test";

import {
	getPlannotatorPhase,
	renderPlanningBorder,
	type SessionEntry,
} from "./plannotator-plan-border/policy.ts";

function phaseEntry(phase: string): SessionEntry {
	return { type: "custom", customType: "plannotator", data: { phase } };
}

test("reads the latest authoritative Plannotator phase", () => {
	assert.equal(getPlannotatorPhase([]), "idle");
	assert.equal(getPlannotatorPhase([phaseEntry("planning")]), "planning");
	assert.equal(getPlannotatorPhase([phaseEntry("planning"), phaseEntry("executing")]), "executing");
	assert.equal(getPlannotatorPhase([phaseEntry("planning"), { type: "message" }]), "planning");
});

test("fails closed to idle for malformed or unknown Plannotator state", () => {
	assert.equal(getPlannotatorPhase([{ type: "custom", customType: "plannotator", data: null }]), "idle");
	assert.equal(getPlannotatorPhase([phaseEntry("unknown")]), "idle");
	assert.equal(
		getPlannotatorPhase([phaseEntry("planning"), { type: "custom", customType: "plannotator", data: {} }]),
		"idle",
	);
});

test("dots only the planning border lines", () => {
	const lines = ["\u001b[90m────────\u001b[39m", "input ─ stays solid", "────────"];
	assert.deepEqual(renderPlanningBorder(lines, "planning"), [
		"\u001b[90m┈┈┈┈┈┈┈┈\u001b[39m",
		"input ─ stays solid",
		"┈┈┈┈┈┈┈┈",
	]);
	assert.deepEqual(renderPlanningBorder(lines, "idle"), lines);
	assert.deepEqual(renderPlanningBorder(lines, "executing"), lines);
	assert.deepEqual(lines, ["\u001b[90m────────\u001b[39m", "input ─ stays solid", "────────"]);
});

test("finds the centered scroll border before autocomplete rows", () => {
	const lines = [
		"────── ↑ 5 more ──────",
		"─2─",
		"────── ↓ 23 more ──────",
		"> /plan command",
	];
	assert.deepEqual(renderPlanningBorder(lines, "planning"), [
		"┈┈┈┈┈┈ ↑ 5 more ┈┈┈┈┈┈",
		"─2─",
		"┈┈┈┈┈┈ ↓ 23 more ┈┈┈┈┈┈",
		"> /plan command",
	]);
});

test("preserves labels while dotting real top-border variants", () => {
	const workingStatus = "⠋ Working (esc to interrupt)";
	const narrowWorkingBorder = `── ${workingStatus} ${"─".repeat(80 - workingStatus.length - 4)}`;
	for (const top of [
		`${"─".repeat(30)} voice: hold space`,
		`── 12s ${"─".repeat(30)}`,
		`${"─".repeat(20)} ↑ 5 more ${"─".repeat(20)} voice: hold space`,
		`${narrowWorkingBorder.slice(0, 62)} voice: hold space`,
		"─── ↑ 5...",
	]) {
		const rendered = renderPlanningBorder([top, "input", "────────"], "planning");
		assert.equal(rendered[0], top.replaceAll("─", "┈"));
	}
});

test("leaves a borderless custom editor unchanged", () => {
	assert.deepEqual(renderPlanningBorder(["I drew a ─ here"], "planning"), ["I drew a ─ here"]);
	assert.deepEqual(renderPlanningBorder(["──▶ diff hunk header", "const x = 1"], "planning"), [
		"──▶ diff hunk header",
		"const x = 1",
	]);
});

test("preserves rendered width at different terminal widths", () => {
	for (const width of [8, 41, 120]) {
		const lines = ["─".repeat(width), "value", "─".repeat(width)];
		const rendered = renderPlanningBorder(lines, "planning");
		assert.equal(rendered[0]?.length, width);
		assert.equal(rendered.at(-1)?.length, width);
		assert.equal(rendered[1], "value");
	}
});

const extensionPath = new URL("./plannotator-plan-border.ts", import.meta.url);
const policyPath = new URL("./plannotator-plan-border/policy.ts", import.meta.url);

async function writeModule(root: string, name: string, source: string): Promise<void> {
	const directory = join(root, "node_modules", ...name.split("/"));
	await mkdir(directory, { recursive: true });
	await writeFile(join(directory, "package.json"), '{"type":"module","exports":"./index.js"}');
	await writeFile(join(directory, "index.js"), source);
}

async function createExtensionFixture() {
	const root = await mkdtemp(join(tmpdir(), "plannotator-plan-border-"));
	await mkdir(join(root, "plannotator-plan-border"));
	await writeFile(join(root, "plannotator-plan-border.ts"), await readFile(extensionPath));
	await writeFile(join(root, "plannotator-plan-border", "policy.ts"), await readFile(policyPath));
	await writeModule(root, "@earendil-works/pi-coding-agent", `
export class CustomEditor {
	constructor(...args) { this.text = ""; this.options = args[3]; }
	render(width) { return ["─".repeat(width), this.text, "─".repeat(width)]; }
	getText() { return this.text; }
	setText(text) { this.text = text; }
	handleInput() {}
	invalidate() {}
}
`);
	await writeModule(root, "@earendil-works/pi-tui", "");
	return { root, entry: pathToFileURL(join(root, "plannotator-plan-border.ts")).href };
}

async function loadExtension() {
	const fixture = await createExtensionFixture();
	try {
		return (await import(fixture.entry)).default;
	} finally {
		await rm(fixture.root, { recursive: true, force: true });
	}
}

const plannotatorPlanBorder = await loadExtension();

test("registers only event wiring within the warm startup budget", async () => {
	const fixture = await createExtensionFixture();
	try {
		await import(fixture.entry);
		for (let trial = 0; trial < 2; trial += 1) {
			const events: string[] = [];
			const start = performance.now();
			const module = await import(`${fixture.entry}?trial=${trial}`);
			const imported = performance.now();
			module.default({ on: (event: string) => events.push(event) });
			await new Promise((resolve) => setTimeout(resolve, 0));
			const elapsed = performance.now() - start;
			console.log(`PI_TIMING=${process.env.PI_TIMING ?? "unset"} trial=${trial + 1} import=${(imported - start).toFixed(3)}ms factory=${(performance.now() - imported).toFixed(3)}ms combined=${elapsed.toFixed(3)}ms`);
			assert.deepEqual(events, ["session_start", "message_end", "tool_result", "agent_end", "session_tree", "session_shutdown"]);
			assert.ok(elapsed <= 50, `warm import and registration ${elapsed}ms exceeds 50ms`);
		}
	} finally {
		await rm(fixture.root, { recursive: true, force: true });
	}
});

type Handler = (event: unknown, context: ReturnType<typeof createContext>) => unknown;

type Editor = {
	render: (width: number) => string[];
	getText: () => string;
	setText: (text: string) => void;
	handleInput: (data: string) => void;
	invalidate: () => void;
};

function createEditor(): Editor {
	return {
		render: (width) => ["─".repeat(width), "input", "─".repeat(width)],
		getText: () => "input",
		setText: () => undefined,
		handleInput: () => undefined,
		invalidate: () => undefined,
	};
}

function createContext(entries: SessionEntry[], previousFactory?: (...args: unknown[]) => Editor) {
	let currentFactory: ((...args: unknown[]) => Editor) | undefined = previousFactory;
	let branchReads = 0;
	const factoryChanges: Array<((...args: unknown[]) => Editor) | undefined> = [];
	return {
		mode: "tui",
		sessionManager: {
			getLeafId: () => String(entries.length),
			getBranch: () => {
				branchReads += 1;
				return entries;
			},
		},
		ui: {
			getEditorComponent: () => currentFactory,
			setEditorComponent: (factory: ((...args: unknown[]) => Editor) | undefined) => {
				currentFactory = factory;
				factoryChanges.push(factory);
			},
		},
		currentFactory: () => currentFactory,
		branchReads: () => branchReads,
		factoryChanges,
		setCurrentFactory: (factory: (...args: unknown[]) => Editor) => {
			currentFactory = factory;
		},
	};
}

function createRuntime() {
	const handlers = new Map<string, Handler[]>();
	const pi = {
		on: (event: string, handler: Handler) => {
			handlers.set(event, [...(handlers.get(event) ?? []), handler]);
		},
	};
	plannotatorPlanBorder(pi as never);
	return {
		run: async (event: string, context: ReturnType<typeof createContext>) => {
			for (const handler of handlers.get(event) ?? []) await handler({}, context);
		},
	};
}

test("decorates the existing editor and follows live phase state", async () => {
	const entries = [phaseEntry("planning")];
	const editor = createEditor();
	const previousFactory = () => editor;
	const context = createContext(entries, previousFactory);
	const runtime = createRuntime();
	await runtime.run("session_start", context);
	await new Promise((resolve) => setTimeout(resolve, 0));

	const installedFactory = context.currentFactory();
	assert.notEqual(installedFactory, previousFactory);
	const renders: number[] = [];
	const wrapped = installedFactory?.({ requestRender: () => renders.push(renders.length) }, {}, {});
	assert.equal(wrapped, editor);
	assert.deepEqual(wrapped?.render(8), ["┈┈┈┈┈┈┈┈", "input", "┈┈┈┈┈┈┈┈"]);

	entries.push(phaseEntry("executing"));
	await runtime.run("tool_result", context);
	assert.equal(renders.length, 1);
	assert.deepEqual(wrapped?.render(8), ["────────", "input", "────────"]);
});

test("caches the phase until the session leaf changes", async () => {
	const entries = [phaseEntry("planning")];
	const context = createContext(entries, () => createEditor());
	const runtime = createRuntime();
	await runtime.run("session_start", context);
	await new Promise((resolve) => setTimeout(resolve, 0));
	const editor = context.currentFactory()?.({ requestRender: () => undefined }, {}, {});

	editor?.render(8);
	editor?.render(8);
	assert.equal(context.branchReads(), 1);
	entries.push(phaseEntry("executing"));
	assert.deepEqual(editor?.render(8), ["────────", "input", "────────"]);
	assert.equal(context.branchReads(), 2);
});

test("falls back to a solid border when session state becomes stale", async () => {
	const context = createContext([phaseEntry("planning")], () => createEditor());
	const runtime = createRuntime();
	await runtime.run("session_start", context);
	await new Promise((resolve) => setTimeout(resolve, 0));
	const editor = context.currentFactory()?.({ requestRender: () => undefined }, {}, {});
	context.sessionManager.getLeafId = () => { throw new Error("session is stale"); };
	assert.deepEqual(editor?.render(8), ["────────", "input", "────────"]);
});

test("requests repaint after every phase-bearing lifecycle event", async () => {
	const context = createContext([phaseEntry("idle")], () => createEditor());
	const runtime = createRuntime();
	await runtime.run("session_start", context);
	await new Promise((resolve) => setTimeout(resolve, 0));
	const renders: number[] = [];
	context.currentFactory()?.({ requestRender: () => renders.push(renders.length) }, {}, {});

	for (const event of ["message_end", "tool_result", "agent_end", "session_tree"]) {
		await runtime.run(event, context);
	}
	assert.equal(renders.length, 4);
});

test("preserves embedded working status without an earlier custom editor", async () => {
	const context = createContext([phaseEntry("idle")]);
	const runtime = createRuntime();
	await runtime.run("session_start", context);
	await new Promise((resolve) => setTimeout(resolve, 0));
	const editor = context.currentFactory()?.({ requestRender: () => undefined }, {}, {});
	assert.equal((editor as any).options?.embedWorkingStatus, true);
});

test("decorates an editor installed later in the shared session-start dispatch", async () => {
	const initialFactory = () => createEditor();
	const laterEditor = createEditor();
	const laterFactory = () => laterEditor;
	const context = createContext([phaseEntry("planning")], initialFactory);
	const runtime = createRuntime();
	await runtime.run("session_start", context);
	context.setCurrentFactory(laterFactory);
	await new Promise((resolve) => setTimeout(resolve, 0));

	const installedFactory = context.currentFactory();
	assert.notEqual(installedFactory, initialFactory);
	assert.notEqual(installedFactory, laterFactory);
	assert.equal(installedFactory?.({ requestRender: () => undefined }, {}, {}), laterEditor);
	assert.deepEqual(laterEditor.render(4), ["┈┈┈┈", "input", "┈┈┈┈"]);
});

test("reuses the base editor across repeated session starts", async () => {
	const baseEditor = createEditor();
	const baseFactory = () => baseEditor;
	const context = createContext([phaseEntry("planning")], baseFactory);
	const runtime = createRuntime();
	await runtime.run("session_start", context);
	await new Promise((resolve) => setTimeout(resolve, 0));
	await runtime.run("session_start", context);
	await new Promise((resolve) => setTimeout(resolve, 0));

	assert.equal(context.currentFactory()?.({ requestRender: () => undefined }, {}, {}), baseEditor);
	assert.deepEqual(baseEditor.render(4), ["┈┈┈┈", "input", "┈┈┈┈"]);
});

test("restores the previous editor only while it owns the slot", async () => {
	const sharedEditor = createEditor();
	const originalRender = sharedEditor.render;
	const previousFactory = () => sharedEditor;
	const firstContext = createContext([phaseEntry("planning")], previousFactory);
	const firstRuntime = createRuntime();
	await firstRuntime.run("session_start", firstContext);
	await new Promise((resolve) => setTimeout(resolve, 0));
	firstContext.currentFactory()?.({ requestRender: () => undefined }, {}, {});
	assert.notEqual(sharedEditor.render, originalRender);
	await firstRuntime.run("session_shutdown", firstContext);
	assert.equal(firstContext.currentFactory(), previousFactory);
	assert.equal(sharedEditor.render, originalRender);

	const secondContext = createContext([phaseEntry("planning")], previousFactory);
	const secondRuntime = createRuntime();
	await secondRuntime.run("session_start", secondContext);
	await new Promise((resolve) => setTimeout(resolve, 0));
	const laterFactory = () => createEditor();
	secondContext.setCurrentFactory(laterFactory);
	await secondRuntime.run("session_shutdown", secondContext);
	assert.equal(secondContext.currentFactory(), laterFactory);
});
