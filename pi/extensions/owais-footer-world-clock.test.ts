import assert from "node:assert/strict";
import { mkdtemp, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { test } from "node:test";

const extensionDirectory = fileURLToPath(new URL(".", import.meta.url));
const tuiModule = `
export const visibleWidth = (text) => text.replace(/\\x1b[^m]*m|\\x1b]8;;[^\\x1b]*\\x1b\\\\/g, "").length;
`;
const codingAgentModule = `
export const withFileMutationQueue = (_path, action) => action();
`;

async function writeModule(root: string, name: string, source: string): Promise<void> {
	const directory = join(root, "node_modules", ...name.split("/"));
	await mkdir(directory, { recursive: true });
	await writeFile(join(directory, "package.json"), '{"type":"module","exports":"./index.js"}');
	await writeFile(join(directory, "index.js"), source);
}

async function loadExtensions() {
	const root = await mkdtemp(join(tmpdir(), "owais-footer-world-clock-"));
	for (const name of ["owais-footer.ts", "world-clock.ts"]) {
		await writeFile(join(root, name), await readFile(join(extensionDirectory, name)));
	}
	await mkdir(join(root, "live-diff"), { recursive: true });
	await writeFile(join(root, "live-diff", "engine.ts"), await readFile(join(extensionDirectory, "live-diff", "engine.ts")));
	await writeModule(root, "@earendil-works/pi-tui", tuiModule);
	await writeModule(root, "@earendil-works/pi-coding-agent", codingAgentModule);
	const suffix = `${Date.now()}-${Math.random()}`;
	const footer = await import(`${pathToFileURL(join(root, "owais-footer.ts")).href}?${suffix}`);
	const clock = await import(`${pathToFileURL(join(root, "world-clock.ts")).href}?${suffix}`);
	return {
		footer,
		clock,
		dispose: () => rm(root, { recursive: true, force: true }),
	};
}

test("uses the Herdr project directory and never nested worktree paths", async () => {
	const extensions = await loadExtensions();
	try {
		assert.equal(
			extensions.footer.projectName("/Users/user/.herdr/worktrees/agents/add-to-pi-config/pi/extensions", "/Users/user/.herdr/worktrees/agents/add-to-pi-config"),
			"agents",
		);
		assert.equal(extensions.footer.projectName("/Users/user/src/repository/lib", "/Users/user/src/repository"), "repository");
	} finally {
		await extensions.dispose();
	}
});

test("renders the compact Git label and linked pull request", async () => {
	const extensions = await loadExtensions();
	const handlers = new Map<string, (...args: any[]) => unknown>();
	const calls: Array<{ command: string; args: string[] }> = [];
	let widget: { dispose?(): void; render(width: number): string[] } | undefined;
	let footer: { dispose?(): void } | undefined;
	let refreshBranch: (() => void) | undefined;
	const theme = {
		fg(_color: string, text: string) {
			return text;
		},
	};
	const api = {
		on(event: string, handler: (...args: any[]) => unknown) {
			handlers.set(event, handler);
		},
		async exec(command: string, args: string[]) {
			calls.push({ command, args });
			if (command === "git" && args[0] === "rev-parse") return { code: 0, stdout: "/Users/user/.herdr/worktrees/agents/add-to-pi-config\n" };
			if (command === "git" && args[0] === "branch") return { code: 0, stdout: "add-to-pi-config\n" };
			if (command === "git" && args[0] === "remote") return { code: 0, stdout: "origin\tgit@github.com:owner/repository.git (fetch)\n" };
			if (command === "gh") {
				return {
					code: 0,
					stdout: JSON.stringify({
						url: "https://github.com/owner/repository/pull/42",
						number: 42,
						state: "OPEN",
						isDraft: false,
						mergeStateStatus: "CLEAN",
					}),
				};
			}
			throw new Error(`unexpected command: ${command} ${args.join(" ")}`);
		},
	};
	const ctx = {
		mode: "tui",
		cwd: "/Users/user/.herdr/worktrees/agents/add-to-pi-config/pi/extensions",
		model: { provider: "test", id: "model", contextWindow: 1 },
		thinkingLevel: "off",
		getContextUsage() {
			return undefined;
		},
		ui: {
			setFooter(factory: (tui: unknown, theme: typeof theme, footerData: { onBranchChange(callback: () => void): () => void }) => { dispose?(): void }) {
				footer = factory({ requestRender() {} }, theme, {
					onBranchChange(callback) {
						refreshBranch = callback;
						return () => {};
					},
				});
			},
			setWidget(_key: string, factory: (tui: unknown, theme: typeof theme) => { dispose?(): void; render(width: number): string[] }) {
				widget = factory({ requestRender() {} }, theme);
			},
		},
	};
	try {
		extensions.footer.default(api);
		await handlers.get("session_start")?.({}, ctx);
		for (let index = 0; index < 4; index++) await new Promise((resolve) => setImmediate(resolve));
		const line = widget?.render(160)[1] ?? "";

		assert.match(line, /agents > /);
		assert.match(line, /add-to-pi-config/);
		assert.match(line, /\x1b]8;;https:\/\/github\.com\/owner\/repository\/pull\/42\x1b\\\x1b\[4mPR #42\x1b\[24m\x1b]8;;\x1b\\/);
		refreshBranch?.();
		assert.match(widget?.render(160)[1] ?? "", /PR #42/);
		await handlers.get("agent_start")?.({}, ctx);
		const activeLine = widget?.render(160)[1] ?? "";
		assert.match(activeLine, /agents > /);
		assert.match(activeLine, /add-to-pi-config/);
		assert.match(activeLine.replace(/\x1b\[[0-9;]*m/g, ""), /· [⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏] \d{2}:\d{2}\.\d/);
		assert.match(activeLine, /PR #42/);
		// below ~50 cols the PR segment shorten-tier drops the "PR " prefix before anything gets hidden.
		assert.match(widget?.render(50)[1] ?? "", /#42/);
		assert.match(widget?.render(42)[1] ?? "", /#42/);
		assert.ok(calls.some((call) => call.command === "gh" && call.args.join(" ") === "pr view --json url,number,state,isDraft,mergeStateStatus"));
	} finally {
		widget?.dispose?.();
		footer?.dispose?.();
		await handlers.get("session_shutdown")?.({}, ctx);
		await extensions.dispose();
	}
});

test("renders compact observational-memory gauges and its off state", async () => {
	const extensions = await loadExtensions();
	const footerState = globalThis as typeof globalThis & { __owaisOmFooterState?: Record<string, unknown> };
	try {
		footerState.__owaisOmFooterState = {
			enabled: true,
			nextValue: 2500,
			nextMax: 10_000,
			poolValue: 13_000,
			poolMax: 15_000,
		};
		assert.equal(extensions.footer.omLabel(), "2.5k/10.0k O -> 13.0k/15.0k C");
		footerState.__owaisOmFooterState = { enabled: false };
		assert.equal(extensions.footer.omLabel(), "Observational Memory OFF");
	} finally {
		delete footerState.__owaisOmFooterState;
		await extensions.dispose();
	}
});

test("cycles the Braille orbit at the preview cadence", async () => {
	const extensions = await loadExtensions();
	try {
		assert.equal(extensions.footer.brailleOrbit(0), "⠋");
		assert.equal(extensions.footer.brailleOrbit(80), "⠙");
		assert.equal(extensions.footer.brailleOrbit(800), "⠋");
	} finally {
		await extensions.dispose();
	}
});

test("maps pull request states to the required colors", async () => {
	const extensions = await loadExtensions();
	try {
		const pullRequest = { url: "https://github.com/owner/repository/pull/1", number: 1, isDraft: false };
		assert.equal(extensions.footer.pullRequestTone({ ...pullRequest, state: "OPEN", mergeStateStatus: "CLEAN" }), "success");
		assert.equal(extensions.footer.pullRequestTone({ ...pullRequest, state: "OPEN", mergeStateStatus: "DIRTY" }), "warning");
		assert.equal(extensions.footer.pullRequestTone({ ...pullRequest, state: "CLOSED", mergeStateStatus: "UNKNOWN" }), "error");
		assert.equal(extensions.footer.pullRequestTone({ ...pullRequest, state: "MERGED", mergeStateStatus: "UNKNOWN" }), "purple");
		assert.equal(extensions.footer.pullRequestTone({ ...pullRequest, isDraft: true, state: "OPEN", mergeStateStatus: "CLEAN" }), "muted");
	} finally {
		await extensions.dispose();
	}
});

test("renders local time bold in the default color and other times muted", async () => {
	const extensions = await loadExtensions();
	try {
		const local = extensions.clock.renderClock({
			is12Hour: false,
			zones: [{ name: "Local", zone: "local", color: "#82b8ff" }],
		}, 120);
		const remote = extensions.clock.renderClock({
			is12Hour: false,
			zones: [{ name: "GMT", zone: "Etc/GMT", color: "#8ee7f5" }],
		}, 120);

		assert.match(local, /\x1b\[39m\x1b\[1m \d{1,2}:\d{2}/);
		assert.match(remote, /\x1b\[38;2;125;142;174m \d{1,2}:\d{2}/);
	} finally {
		await extensions.dispose();
	}
});
