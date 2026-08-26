import assert from "node:assert/strict";
import { mkdtemp, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { test } from "node:test";

const extensionPath = fileURLToPath(new URL("./ask-user-question.ts", import.meta.url));
const herdrActivityStatePath = fileURLToPath(new URL("./herdr-activity/state.ts", import.meta.url));

const tuiModule = `
export const Key = { up: "UP", down: "DOWN", enter: "ENTER", space: "SPACE", backspace: "BACKSPACE", escape: "ESC" };
export const matchesKey = (data, key) => data === key;
export const truncateToWidth = (text) => text;
export const wrapTextWithAnsi = (text) => [text];
export class Text { constructor(text) { this.text = text; } }
export class Editor {
	constructor() { this.text = ""; this.onSubmit = undefined; }
	setText(text) { this.text = text; }
	handleInput(data) {
		if (data === "ENTER") { this.onSubmit?.(this.text); return; }
		if (data === "BACKSPACE") { this.text = this.text.slice(0, -1); return; }
		this.text += data;
	}
	render() { return []; }
}
`;

const typeboxModule = `
export const Type = {
	Object: (value) => value,
	String: (value) => value,
	Optional: (value) => value,
	Array: (value) => value,
	Boolean: (value) => value,
};
`;

async function writeModule(root: string, name: string, source: string): Promise<void> {
	const directory = join(root, "node_modules", ...name.split("/"));
	await mkdir(directory, { recursive: true });
	await writeFile(join(directory, "package.json"), '{"type":"module","exports":"./index.js"}');
	await writeFile(join(directory, "index.js"), source);
}

async function loadPicker(mode = "rpc") {
	const root = await mkdtemp(join(tmpdir(), "ask-user-question-"));
	await writeFile(join(root, "ask-user-question.ts"), await readFile(extensionPath));
	await mkdir(join(root, "herdr-activity"));
	await writeFile(join(root, "herdr-activity", "state.ts"), await readFile(herdrActivityStatePath));
	await writeModule(root, "@earendil-works/pi-tui", tuiModule);
	await writeModule(root, "typebox", typeboxModule);
	await writeModule(root, "@earendil-works/pi-coding-agent", "");

	const calls: Array<{ command: string; args: string[] }> = [];
	let tool: any;
	let component: any;
	let resolveCustom: ((value: unknown) => void) | undefined;
	const customResult = new Promise((resolve) => {
		resolveCustom = resolve;
	});
	const theme = {
		fg(_name: string, text: string) {
			return text;
		},
		bold(text: string) {
			return text;
		},
	};
	const ctx = {
		hasUI: true,
		mode,
		sessionManager: { getSessionId: () => "session-1" },
		ui: {
			async editor() {
				return "  free text  ";
			},
			custom(factory: any) {
				component = factory({ requestRender() {} }, theme, {}, resolveCustom);
				return customResult;
			},
		},
	};
	const api = {
		registerTool(value: unknown) {
			tool = value;
		},
		async exec(command: string, args: string[]) {
			calls.push({ command, args });
			return { code: 0, stdout: "", stderr: "" };
		},
	};
	const module = await import(`${pathToFileURL(join(root, "ask-user-question.ts")).href}?${Date.now()}-${Math.random()}`);
	module.default(api);
	return {
		tool,
		ctx,
		calls,
		component: () => component,
		dispose: async () => {
			await rm(root, { recursive: true, force: true });
		},
	};
}

async function waitForComponent(getComponent: () => any): Promise<any> {
	for (let attempt = 0; attempt < 20; attempt++) {
		const component = getComponent();
		if (component !== undefined) {
			return component;
		}
		await new Promise((resolve) => setTimeout(resolve, 0));
	}
	throw new Error("picker did not open");
}

test("keeps free-text questions and rejects a supplied single option", async () => {
	const picker = await loadPicker();
	try {
		const textResult = await picker.tool.execute("text", { question: "What?", options: [] }, undefined, undefined, picker.ctx);
		assert.equal(textResult.details.mode, "text");
		assert.equal(textResult.details.answers[0].value, "free text");

		const invalid = await picker.tool.execute(
			"invalid",
			{ question: "Choose", options: [{ label: "Only" }] },
			undefined,
			undefined,
			picker.ctx,
		);
		assert.equal(invalid.details.status, "invalid");
		assert.match(invalid.content[0].text, /at least two non-blank options/);
	} finally {
		await picker.dispose();
	}
});

test("keeps single-select option answers", async () => {
	const picker = await loadPicker();
	try {
		const result = picker.tool.execute(
			"single",
			{ question: "Choose", options: [{ label: "First" }, { label: "Second" }] },
			undefined,
			undefined,
			picker.ctx,
		);
		const component = await waitForComponent(picker.component);
		component.handleInput("ENTER");
		const settled = await result;
		assert.deepEqual(settled.details.answers, [{ type: "option", label: "First", value: "First", index: 1 }]);
	} finally {
		await picker.dispose();
	}
});

test("selects the final item from an unbounded option list", async () => {
	const picker = await loadPicker();
	try {
		const result = picker.tool.execute(
			"many",
			{
				question: "Choose",
				multiSelect: true,
				options: Array.from({ length: 100 }, (_, index) => ({ label: `Option ${index + 1}` })),
			},
			undefined,
			undefined,
			picker.ctx,
		);
		const component = await waitForComponent(picker.component);
		for (let index = 0; index < 99; index++) {
			component.handleInput("DOWN");
		}
		component.handleInput("SPACE");
		component.handleInput("DOWN");
		component.handleInput("DOWN");
		component.handleInput("ENTER");

		const settled = await result;
		assert.deepEqual(settled.details.answers, [{ type: "option", label: "Option 100", value: "Option 100", index: 100 }]);
	} finally {
		await picker.dispose();
	}
});

test("edits and removes an individual Other answer", async () => {
	const picker = await loadPicker();
	try {
		const result = picker.tool.execute(
			"other",
			{ question: "Choose", multiSelect: true, options: [{ label: "One" }, { label: "Two" }] },
			undefined,
			undefined,
			picker.ctx,
		);
		const component = await waitForComponent(picker.component);
		component.handleInput("DOWN");
		component.handleInput("DOWN");
		component.handleInput("ENTER");
		component.handleInput("First answer");
		component.handleInput("ENTER");
		component.handleInput("ENTER");
		component.handleInput("Second answer");
		component.handleInput("ENTER");
		component.handleInput("UP");
		component.handleInput("ENTER");
		for (let index = 0; index < "Second answer".length; index++) {
			component.handleInput("BACKSPACE");
		}
		component.handleInput("Updated second answer");
		component.handleInput("ENTER");

		let lines = component.render(120).join("\n");
		assert.match(lines, /> \[x\] Other: Updated second answer/);
		assert.match(lines, /\[x\] Other: First answer/);

		component.handleInput("SPACE");
		lines = component.render(120).join("\n");
		assert.doesNotMatch(lines, /Updated second answer/);
		assert.match(lines, /\[x\] Other: First answer/);

		component.handleInput("DOWN");
		component.handleInput("ENTER");
		const settled = await result;
		assert.deepEqual(settled.details.answers, [
			{ type: "other", label: "First answer", value: "First answer" },
		]);
	} finally {
		await picker.dispose();
	}
});

test("adds a second Other answer from the final Add Other row", async () => {
	const picker = await loadPicker();
	try {
		const result = picker.tool.execute(
			"other",
			{ question: "Choose", multiSelect: true, options: [{ label: "One" }, { label: "Two" }] },
			undefined,
			undefined,
			picker.ctx,
		);
		const component = await waitForComponent(picker.component);
		component.handleInput("DOWN");
		component.handleInput("DOWN");
		component.handleInput("ENTER");
		component.handleInput("First answer");
		component.handleInput("ENTER");
		component.handleInput("ENTER");
		component.handleInput("Second answer");
		component.handleInput("ENTER");
		component.handleInput("ENTER");
		component.handleInput("first answer");
		component.handleInput("ENTER");

		const lines = component.render(120).join("\n");
		assert.equal((lines.match(/\[ \] Add Other/g) ?? []).length, 1);
		assert.match(lines, /\[x\] Other: First answer/);
		assert.match(lines, /\[x\] Other: Second answer/);
		assert.doesNotMatch(lines, /custom answers/);

		component.handleInput("DOWN");
		component.handleInput("ENTER");
		const settled = await result;
		assert.deepEqual(settled.details.answers, [
			{ type: "other", label: "First answer", value: "First answer" },
			{ type: "other", label: "Second answer", value: "Second answer" },
		]);
	} finally {
		await picker.dispose();
	}
});

test("reports Herdr blocked until picker cancellation", async () => {
	const previous = {
		HERDR_ENV: process.env.HERDR_ENV,
		HERDR_SOCKET_PATH: process.env.HERDR_SOCKET_PATH,
		HERDR_PANE_ID: process.env.HERDR_PANE_ID,
	};
	process.env.HERDR_ENV = "1";
	process.env.HERDR_SOCKET_PATH = "/tmp/herdr.sock";
	process.env.HERDR_PANE_ID = "w1:p1";
	const picker = await loadPicker("tui");
	try {
		const result = picker.tool.execute(
			"cancel",
			{ question: "Choose", options: [{ label: "One" }, { label: "Two" }] },
			undefined,
			undefined,
			picker.ctx,
		);
		const component = await waitForComponent(picker.component);
		assert.equal(picker.calls[0]?.args[8], "blocked");
		component.handleInput("ESC");
		const settled = await result;
		assert.equal(settled.details.status, "cancelled");
		assert.equal(picker.calls[1]?.args[8], "idle");
	} finally {
		await picker.dispose();
		for (const [name, value] of Object.entries(previous)) {
			if (value === undefined) {
				delete process.env[name];
			} else {
				process.env[name] = value;
			}
		}
	}
});
