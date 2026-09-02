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
	fmRespondGate?: Promise<void>;
} = {}) {
	let commits = overrides.commits ?? ["abc1230 add branch summary segment", "def4560 fix footer alignment"];
	let fmResponse = overrides.fmResponse ?? "Branch summary widget";
	const fmAvailable = overrides.fmAvailable ?? true;
	const headSha = overrides.headSha ?? HEAD_SHA;
	const fmRespondGate = overrides.fmRespondGate;
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
			if (fmRespondGate) await fmRespondGate;
			return { code: 0, stdout: `${fmResponse}\n` };
		}
		throw new Error(`unexpected command: ${command} ${args.join(" ")}`);
	}

	return {
		exec,
		calls,
		setCommits: (next: string[]) => { commits = next; },
		setFmResponse: (next: string) => { fmResponse = next; },
	};
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

test("pure: computeBranchSummaryRelevance is symmetric and 0 with no incumbent", async () => {
	const extensions = await loadFooter();
	try {
		const { computeBranchSummaryRelevance } = extensions.footer;
		assert.equal(computeBranchSummaryRelevance(undefined, 4), 0, "no incumbent means no relevance to preserve");
		assert.equal(computeBranchSummaryRelevance(4, 4), 1, "commit count unchanged");
		assert.equal(computeBranchSummaryRelevance(4, 8), 0.5, "growth erodes relevance");
		assert.equal(computeBranchSummaryRelevance(8, 4), 0.5, "shrinkage erodes relevance the same way");
		assert.equal(computeBranchSummaryRelevance(4, 0), 0, "nothing ahead means no relevance");
	} finally {
		await extensions.dispose();
	}
});

test("pure: isBranchSummaryChallengerBetter rejects empty or identical challengers", async () => {
	const extensions = await loadFooter();
	try {
		const { isBranchSummaryChallengerBetter } = extensions.footer;
		assert.equal(isBranchSummaryChallengerBetter(undefined, "New headline"), true, "no incumbent — anything non-empty wins");
		assert.equal(isBranchSummaryChallengerBetter("Old headline", ""), false, "empty challenger never wins");
		assert.equal(isBranchSummaryChallengerBetter("Old headline", "  "), false, "whitespace-only challenger never wins");
		assert.equal(isBranchSummaryChallengerBetter("Old headline", "old headline"), false, "case-insensitive same text is not better");
		assert.equal(isBranchSummaryChallengerBetter("Old headline", "New headline"), true, "a different non-empty challenger wins");
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

test("pure: truncateSegmentText fromStart keeps the trailing text and leads with the ellipsis", async () => {
	const extensions = await loadFooter();
	try {
		const { truncateSegmentText } = extensions.footer;
		assert.equal(truncateSegmentText("short text", 60, true), "short text");
		const truncated = truncateSegmentText("worktree/green-forest-a59b", 12, true);
		assert.ok(truncated.startsWith("\u2026"), "leading truncation drops the front, so the ellipsis leads");
		assert.ok(truncated.endsWith("a59b"), "the distinguishing suffix survives");
		assert.equal(truncated.length, 12);
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
		assert.match(line, /Branch summary widget/);
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
		assert.doesNotMatch(line, /Branch summary widget/);
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
		assert.doesNotMatch(line, /Branch summary widget/);
	} finally {
		disposeAll();
		await handlers.get("session_shutdown")?.({}, ctx);
		await extensions.dispose();
	}
});

test("relevance guard skips a second fm respond call when the commit count hasn't moved enough", async () => {
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
		assert.equal(respondCallsAfterSecond, 1, "commit count did not move, so no second fm respond call should fire");
	} finally {
		disposeAll();
		await handlers.get("session_shutdown")?.({}, ctx);
		await extensions.dispose();
	}
});

test("regenerates once commit count crosses the relevance threshold and adopts a different challenger", async () => {
	const extensions = await loadFooter();
	const { exec, calls, setCommits, setFmResponse } = makeExec({ commits: ["c1 one", "c2 two"] });
	const { api, ctx, handlers, getWidget, disposeAll } = makeContext(exec);
	try {
		extensions.footer.default(api);
		await handlers.get("session_start")?.({}, ctx);
		await settle();
		assert.match(getWidget()?.render(160)[1] ?? "", /Branch summary widget/);

		// 2 commits -> 6 commits is relevance 2/6 ≈ 0.33, below the 0.5 threshold, so this should regenerate.
		setCommits(["c1 one", "c2 two", "c3 three", "c4 four", "c5 five", "c6 six"]);
		setFmResponse("Sweeping rewrite");
		await handlers.get("agent_settled")?.({}, ctx);
		await settle();
		const line = getWidget()?.render(160)[1] ?? "";
		assert.match(line, /Sweeping rewrite/);
		assert.doesNotMatch(line, /Branch summary widget/);
		const respondCalls = calls.filter((call) => call.command === "fm" && call.args[0] === "respond").length;
		assert.equal(respondCalls, 2);
	} finally {
		disposeAll();
		await handlers.get("session_shutdown")?.({}, ctx);
		await extensions.dispose();
	}
});

test("keeps the incumbent headline visible and unchanged when a regenerated challenger is not better", async () => {
	const extensions = await loadFooter();
	const { exec, calls, setCommits } = makeExec({ commits: ["c1 one", "c2 two"], fmResponse: "Branch summary widget" });
	const { api, ctx, handlers, getWidget, disposeAll } = makeContext(exec);
	try {
		extensions.footer.default(api);
		await handlers.get("session_start")?.({}, ctx);
		await settle();
		assert.match(getWidget()?.render(160)[1] ?? "", /Branch summary widget/);

		// commit count crosses the threshold, but fm returns the same text back — not a better challenger.
		setCommits(["c1 one", "c2 two", "c3 three", "c4 four", "c5 five", "c6 six"]);
		await handlers.get("agent_settled")?.({}, ctx);
		await settle();
		assert.match(getWidget()?.render(160)[1] ?? "", /Branch summary widget/, "identical challenger never blanks or changes the incumbent");

		// the commit-count anchor still advanced from the identical-challenger round, so a third settle
		// with the same commit count should not fire fm again.
		await handlers.get("agent_settled")?.({}, ctx);
		await settle();
		const respondCalls = calls.filter((call) => call.command === "fm" && call.args[0] === "respond").length;
		assert.equal(respondCalls, 2, "the anchor advanced even though the visible text did not change");
	} finally {
		disposeAll();
		await handlers.get("session_shutdown")?.({}, ctx);
		await extensions.dispose();
	}
});

function deferred(): { promise: Promise<void>; resolve: () => void } {
	let resolve!: () => void;
	const promise = new Promise<void>((res) => { resolve = res; });
	return { promise, resolve };
}

test("shows the generating segment only while fm respond is in flight, then swaps to the real headline", async () => {
	const extensions = await loadFooter();
	const gate = deferred();
	const { exec } = makeExec({ fmRespondGate: gate.promise });
	const { api, ctx, handlers, getWidget, disposeAll } = makeContext(exec);
	try {
		extensions.footer.default(api);
		await handlers.get("session_start")?.({}, ctx);
		await settle();
		const inFlightLine = getWidget()?.render(160)[1] ?? "";
		assert.match(inFlightLine, /Generating headline…/, "the fm respond call is still gated, so the transient segment should be showing");
		assert.doesNotMatch(inFlightLine, /Branch summary widget/, "no summary exists yet — only the transient segment stands in");

		gate.resolve();
		await settle();
		const doneLine = getWidget()?.render(160)[1] ?? "";
		assert.match(doneLine, /Branch summary widget/, "the real summary replaces the transient segment once fm respond resolves");
		assert.doesNotMatch(doneLine, /Generating headline…/, "the transient segment does not linger once a summary exists");
	} finally {
		disposeAll();
		await handlers.get("session_shutdown")?.({}, ctx);
		await extensions.dispose();
	}
});

test("pulses one character of the incumbent headline while a regeneration is in flight, then stops", async () => {
	const extensions = await loadFooter();
	let secondGate: Promise<void> | undefined;
	const { exec: baseExec, setCommits } = makeExec({ commits: ["c1 one", "c2 two"], fmResponse: "Branch summary widget" });
	const exec = async (command: string, args: string[]) => {
		if (command === "fm" && args[0] === "respond" && secondGate) await secondGate;
		return baseExec(command, args);
	};
	const { api, ctx, handlers, getWidget, disposeAll } = makeContext(exec);
	try {
		extensions.footer.default(api);
		await handlers.get("session_start")?.({}, ctx);
		await settle();
		const firstLine = getWidget()?.render(160)[1] ?? "";
		assert.match(firstLine, /Branch summary widget/, "the incumbent is populated before any regeneration starts");
		assert.doesNotMatch(firstLine, /\x1b\[1m/, "no pulse markup while idle");

		// commit count crosses the relevance threshold, so agent_settled starts a regeneration —
		// gate this second fm respond call so the incumbent must stay on screen while it's in flight.
		const gate = deferred();
		secondGate = gate.promise;
		setCommits(["c1 one", "c2 two", "c3 three", "c4 four", "c5 five", "c6 six"]);
		await handlers.get("agent_settled")?.({}, ctx);
		await settle();
		const inFlightLine = getWidget()?.render(160)[1] ?? "";
		const inFlightPlain = inFlightLine.replace(/\x1b\[[0-9;]*m/g, "");
		assert.match(inFlightPlain, /Branch summary widget/, "the incumbent headline stays put, never swapped for the transient 'Generating headline…' text");
		assert.doesNotMatch(inFlightLine, /Generating headline…/, "the first-generation transient text is reserved for when there's no incumbent yet");
		assert.match(inFlightLine, /\x1b\[1m/, "one character is bolded as the loading cue while regenerating");

		// advancing the render clock should move the pulse to a different character, not repaint the same one forever.
		const boldedIndexes = new Set<number>();
		for (let tick = 0; tick < 5; tick++) {
			const line = getWidget()?.render(160)[1] ?? "";
			const match = /(.)\x1b\[1m(.)\x1b\[22m/.exec(line);
			if (match) boldedIndexes.add(line.indexOf("\x1b[1m"));
			await new Promise((resolve) => setTimeout(resolve, 40));
		}
		assert.ok(boldedIndexes.size > 1, "the bolded position moves over time rather than sitting on one character");

		gate.resolve();
		await settle();
		const doneLine = getWidget()?.render(160)[1] ?? "";
		assert.doesNotMatch(doneLine, /\x1b\[1m/, "the pulse stops once the regeneration settles");
	} finally {
		disposeAll();
		await handlers.get("session_shutdown")?.({}, ctx);
		await extensions.dispose();
	}
});

test("never shows the generating segment while idle — fm respond never fires when the system model is unavailable", async () => {
	const extensions = await loadFooter();
	const { exec } = makeExec({ fmAvailable: false });
	const { api, ctx, handlers, getWidget, disposeAll } = makeContext(exec);
	try {
		extensions.footer.default(api);
		await handlers.get("session_start")?.({}, ctx);
		await settle();
		const line = getWidget()?.render(160)[1] ?? "";
		assert.doesNotMatch(line, /Generating headline…/, "an idle 120ms render tick must never fabricate an in-flight state");
	} finally {
		disposeAll();
		await handlers.get("session_shutdown")?.({}, ctx);
		await extensions.dispose();
	}
});

test("clears the generating segment to nothing, never stuck, when fm respond returns an empty response", async () => {
	const extensions = await loadFooter();
	const gate = deferred();
	const { exec } = makeExec({ fmRespondGate: gate.promise, fmResponse: "" });
	const { api, ctx, handlers, getWidget, disposeAll } = makeContext(exec);
	try {
		extensions.footer.default(api);
		await handlers.get("session_start")?.({}, ctx);
		await settle();
		assert.match(getWidget()?.render(160)[1] ?? "", /Generating headline…/, "still in flight");

		gate.resolve();
		await settle();
		const line = getWidget()?.render(160)[1] ?? "";
		assert.doesNotMatch(line, /Generating headline…/, "a failed/empty response must not leave the transient segment stuck");
		assert.doesNotMatch(line, /Branch summary widget/, "no summary was produced, so nothing takes its place");
	} finally {
		disposeAll();
		await handlers.get("session_shutdown")?.({}, ctx);
		await extensions.dispose();
	}
});

function textForm(text: string): { text: string; render: () => string } {
	return { text, render: () => text };
}

test("pure: resolveFooterSegments fits without degrading anything", async () => {
	const extensions = await loadFooter();
	try {
		const items = [
			{ id: "branch" as const, full: textForm("feature-branch") },
			{ id: "workspace" as const, full: textForm("agents") },
			{ id: "model" as const, full: textForm("model") },
		];
		const forms = extensions.footer.resolveFooterSegments(items, 80);
		assert.equal(forms.get("branch")?.text, "feature-branch");
		assert.equal(forms.get("workspace")?.text, "agents");
		assert.equal(forms.get("model")?.text, "model");
	} finally {
		await extensions.dispose();
	}
});

test("pure: resolveFooterSegments shortens every item bottom-to-top before truncating any", async () => {
	const extensions = await loadFooter();
	try {
		// full: "branch-name-long · workspace-name" (33 wide) does not fit in 20; shortening branch alone
		// (lower priority) to "short" (7 wide) brings the line to "short · workspace-name" (23) which still
		// does not fit, so workspace also shortens even though branch already gave up its shorten tier.
		const items = [
			{ id: "branch" as const, full: textForm("branch-name-long"), shorten: textForm("short"), truncate: textForm("b") },
			{ id: "workspace" as const, full: textForm("workspace-name"), shorten: textForm("ws"), truncate: textForm("w") },
		];
		const forms = extensions.footer.resolveFooterSegments(items, 20);
		assert.equal(forms.get("branch")?.text, "short");
		assert.equal(forms.get("workspace")?.text, "ws");
	} finally {
		await extensions.dispose();
	}
});

test("pure: resolveFooterSegments hides the lowest-priority item before touching a higher one", async () => {
	const extensions = await loadFooter();
	try {
		const items = [
			{ id: "branch" as const, full: textForm("feature-branch") },
			{ id: "model" as const, full: textForm("model-name") },
		];
		const forms = extensions.footer.resolveFooterSegments(items, 11);
		assert.equal(forms.get("branch"), undefined, "branch has lower priority and no shorten/truncate form, so it is hidden first");
		assert.equal(forms.get("model")?.text, "model-name", "model keeps its highest-priority slot");
	} finally {
		await extensions.dispose();
	}
});

test("pure: assembleFooterLine keeps the workspace/branch arrow only when both are visible", async () => {
	const extensions = await loadFooter();
	try {
		const pick = (form: { text: string }) => form.text;
		const both = new Map([
			["workspace", textForm("agents")],
			["branch", textForm("main")],
		]);
		assert.equal(extensions.footer.assembleFooterLine(both, pick, " · ", " > ").left, "agents > main");

		const branchOnly = new Map([
			["workspace", undefined],
			["branch", textForm("main")],
			["pr", textForm("#42")],
		]);
		assert.equal(extensions.footer.assembleFooterLine(branchOnly, pick, " · ", " > ").left, "main · #42");
	} finally {
		await extensions.dispose();
	}
});

test("pure: assembleFooterLine builds the model cluster from provider/thinking/model independently", async () => {
	const extensions = await loadFooter();
	try {
		const pick = (form: { text: string }) => form.text;
		const full = new Map([
			["provider", textForm("anthropic")],
			["thinking", textForm("high")],
			["model", textForm("claude")],
		]);
		assert.equal(extensions.footer.assembleFooterLine(full, pick, " · ", " > ").right, "anthropic/claude (high)");

		const noProviderNoThinking = new Map([
			["provider", undefined],
			["thinking", undefined],
			["model", textForm("claude")],
		]);
		assert.equal(extensions.footer.assembleFooterLine(noProviderNoThinking, pick, " · ", " > ").right, "claude");

		const modelHidden = new Map([
			["provider", textForm("anthropic")],
			["thinking", textForm("high")],
			["model", undefined],
		]);
		assert.equal(extensions.footer.assembleFooterLine(modelHidden, pick, " · ", " > ").right, "", "provider/thinking never render without a model");
	} finally {
		await extensions.dispose();
	}
});
