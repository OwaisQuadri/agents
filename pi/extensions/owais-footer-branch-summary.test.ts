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

async function loadFooter() {
	const root = await mkdtemp(join(tmpdir(), "owais-footer-branch-summary-"));
	await writeFile(join(root, "owais-footer.ts"), await readFile(join(extensionDirectory, "owais-footer.ts")));
	await mkdir(join(root, "live-diff"), { recursive: true });
	await writeFile(join(root, "live-diff", "engine.ts"), await readFile(join(extensionDirectory, "live-diff", "engine.ts")));
	await writeModule(root, "@earendil-works/pi-tui", tuiModule);
	await writeModule(root, "@earendil-works/pi-coding-agent", codingAgentModule);
	const suffix = `${Date.now()}-${Math.random()}`;
	const footer = await import(`${pathToFileURL(join(root, "owais-footer.ts")).href}?${suffix}`);
	return { footer, dispose: () => rm(root, { recursive: true, force: true }) };
}

const HEAD_SHA = "abc1230000000000000000000000000000000head";
const MERGE_BASE_SHA = "def4560000000000000000000000000000000base";

function makeExec(overrides: {
	commits?: string[];
	fmAvailable?: boolean;
	fmResponse?: string;
	headSha?: string;
} = {}) {
	const commits = overrides.commits ?? ["abc1230 add branch summary segment", "def4560 fix footer alignment"];
	const fmAvailable = overrides.fmAvailable ?? true;
	const fmResponse = overrides.fmResponse ?? "Added branch summary, fixed footer alignment.";
	const headSha = overrides.headSha ?? HEAD_SHA;
	const calls: Array<{ command: string; args: string[] }> = [];

	async function exec(command: string, args: string[]) {
		calls.push({ command, args });
		if (command === "git" && args[0] === "rev-parse" && args[1] === "--show-toplevel") {
			return { code: 0, stdout: "/Users/user/repo\n" };
		}
		if (command === "git" && args[0] === "branch" && args[1] === "--show-current") {
			return { code: 0, stdout: "feature-branch\n" };
		}
		if (command === "git" && args[0] === "remote" && args[1] === "-v") {
			return { code: 0, stdout: "" };
		}
		if (command === "git" && args[0] === "rev-parse" && args.join(" ") === "rev-parse HEAD") {
			return { code: 0, stdout: `${headSha}\n` };
		}
		if (command === "git" && args.join(" ") === "rev-parse --verify --quiet HEAD") {
			return { code: 0, stdout: `${headSha}\n` };
		}
		if (command === "git" && args.join(" ") === "symbolic-ref --quiet refs/remotes/origin/HEAD") {
			return { code: 1, stdout: "" };
		}
		if (command === "git" && args[0] === "rev-parse" && args[1] === "--verify" && args[2] === "--quiet" && args[3] === "origin/main^{commit}") {
			return { code: 0, stdout: `${MERGE_BASE_SHA}\n` };
		}
		if (command === "git" && args[0] === "merge-base" && args[1] === "HEAD" && args[2] === "origin/main") {
			return { code: 0, stdout: `${MERGE_BASE_SHA}\n` };
		}
		if (command === "git" && args[0] === "log" && args.includes(`${MERGE_BASE_SHA}..HEAD`)) {
			return { code: 0, stdout: commits.length ? `${commits.join("\n")}\n` : "" };
		}
		if (command === "fm" && args[0] === "available") {
			return fmAvailable
				? { code: 0, stdout: "System model available\n" }
				: { code: 1, stdout: "" };
		}
		if (command === "fm" && args[0] === "respond") {
			return { code: 0, stdout: `${fmResponse}\n` };
		}
		throw new Error(`unexpected command: ${command} ${args.join(" ")}`);
	}

	return { exec, calls };
}

function makeContext(exec: (command: string, args: string[]) => Promise<{ code: number; stdout: string }>) {
	const handlers = new Map<string, (...args: unknown[]) => unknown>();
	const theme = { fg(_color: string, text: string) { return text; } };
	let widget: { dispose?(): void; render(width: number): string[] } | undefined;
	let footer: { dispose?(): void } | undefined;
	const api = {
		on(event: string, handler: (...args: unknown[]) => unknown) { handlers.set(event, handler); },
		async exec(command: string, args: string[]) { return exec(command, args); },
	};
	const ctx = {
		mode: "tui",
		cwd: "/Users/user/repo",
		model: { provider: "test", id: "model", contextWindow: 1 },
		thinkingLevel: "off",
		getContextUsage() { return undefined; },
		ui: {
			setFooter(factory: (tui: unknown, theme: typeof theme, footerData: { onBranchChange(callback: () => void): () => void }) => { dispose?(): void }) {
				footer = factory({ requestRender() {} }, theme, { onBranchChange() { return () => {}; } });
			},
			setWidget(_key: string, factory: (tui: unknown, theme: typeof theme) => { dispose?(): void; render(width: number): string[] }) {
				widget = factory({ requestRender() {} }, theme);
			},
		},
	};
	return { api, ctx, handlers, getWidget: () => widget, disposeAll: () => { widget?.dispose?.(); footer?.dispose?.(); } };
}

async function settle(times = 6): Promise<void> {
	for (let index = 0; index < times; index++) await new Promise((resolve) => setImmediate(resolve));
}

test("pure: shouldRecomputeBranchSummary", async () => {
	const extensions = await loadFooter();
	try {
		const { shouldRecomputeBranchSummary } = extensions.footer;
		assert.equal(shouldRecomputeBranchSummary(undefined, "sha1"), true);
		assert.equal(shouldRecomputeBranchSummary("sha1", "sha1"), false);
		assert.equal(shouldRecomputeBranchSummary("sha1", "sha2"), true);
		assert.equal(shouldRecomputeBranchSummary("sha1", undefined), false);
	} finally {
		await extensions.dispose();
	}
});

test("pure: buildBranchSummaryPrompt joins commit subjects", async () => {
	const extensions = await loadFooter();
	try {
		const prompt = extensions.footer.buildBranchSummaryPrompt(["fix bug", "add feature"]);
		assert.match(prompt, /fix bug/);
		assert.match(prompt, /add feature/);
	} finally {
		await extensions.dispose();
	}
});

test("pure: truncateSegmentText collapses whitespace and truncates with an ellipsis", async () => {
	const extensions = await loadFooter();
	try {
		const { truncateSegmentText } = extensions.footer;
		assert.equal(truncateSegmentText("short text", 60), "short text");
		assert.equal(truncateSegmentText("line one\nline two", 60), "line one line two");
		const truncated = truncateSegmentText("a".repeat(100), 20);
		assert.equal(truncated.length, 20);
		assert.ok(truncated.endsWith("\u2026"));
	} finally {
		await extensions.dispose();
	}
});

test("segment appears between the PR position and activity once a summary is computed", async () => {
	const extensions = await loadFooter();
	const { exec } = makeExec();
	const { api, ctx, handlers, getWidget, disposeAll } = makeContext(exec);
	try {
		extensions.footer.default(api);
		await handlers.get("session_start")?.({}, ctx);
		await settle();
		const line = getWidget()?.render(160)[1] ?? "";
		assert.match(line, /Added branch summary, fixed footer alignment\./);
	} finally {
		disposeAll();
		await handlers.get("session_shutdown")?.({}, ctx);
		await extensions.dispose();
	}
});

test("segment stays absent when fm reports the system model unavailable", async () => {
	const extensions = await loadFooter();
	const { exec } = makeExec({ fmAvailable: false });
	const { api, ctx, handlers, getWidget, disposeAll } = makeContext(exec);
	try {
		extensions.footer.default(api);
		await handlers.get("session_start")?.({}, ctx);
		await settle();
		const line = getWidget()?.render(160)[1] ?? "";
		assert.doesNotMatch(line, /Added branch summary/);
	} finally {
		disposeAll();
		await handlers.get("session_shutdown")?.({}, ctx);
		await extensions.dispose();
	}
});

test("segment stays absent before the first successful compute (no commits ahead of the branch point)", async () => {
	const extensions = await loadFooter();
	const { exec } = makeExec({ commits: [] });
	const { api, ctx, handlers, getWidget, disposeAll } = makeContext(exec);
	try {
		extensions.footer.default(api);
		await handlers.get("session_start")?.({}, ctx);
		await settle();
		const line = getWidget()?.render(160)[1] ?? "";
		assert.doesNotMatch(line, /Added branch summary/);
	} finally {
		disposeAll();
		await handlers.get("session_shutdown")?.({}, ctx);
		await extensions.dispose();
	}
});

test("HEAD-unchanged guard skips a second fm respond call on a repeat agent_settled", async () => {
	const extensions = await loadFooter();
	const { exec, calls } = makeExec();
	const { api, ctx, handlers, getWidget, disposeAll } = makeContext(exec);
	try {
		extensions.footer.default(api);
		await handlers.get("session_start")?.({}, ctx);
		await settle();
		const respondCallsAfterFirst = calls.filter((call) => call.command === "fm" && call.args[0] === "respond").length;
		assert.equal(respondCallsAfterFirst, 1);

		await handlers.get("agent_settled")?.({}, ctx);
		await settle();
		const respondCallsAfterSecond = calls.filter((call) => call.command === "fm" && call.args[0] === "respond").length;
		assert.equal(respondCallsAfterSecond, 1, "HEAD did not move, so no second fm respond call should fire");
	} finally {
		disposeAll();
		await handlers.get("session_shutdown")?.({}, ctx);
		await extensions.dispose();
	}
});
