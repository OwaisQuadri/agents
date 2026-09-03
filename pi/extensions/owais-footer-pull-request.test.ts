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
	const root = await mkdtemp(join(tmpdir(), "owais-footer-pull-request-"));
	await writeFile(join(root, "owais-footer.ts"), await readFile(join(extensionDirectory, "owais-footer.ts")));
	await mkdir(join(root, "live-diff"), { recursive: true });
	await writeFile(join(root, "live-diff", "engine.ts"), await readFile(join(extensionDirectory, "live-diff", "engine.ts")));
	await writeModule(root, "@earendil-works/pi-tui", tuiModule);
	await writeModule(root, "@earendil-works/pi-coding-agent", codingAgentModule);
	const suffix = `${Date.now()}-${Math.random()}`;
	const footer = await import(`${pathToFileURL(join(root, "owais-footer.ts")).href}?${suffix}`);
	return { footer, dispose: () => rm(root, { recursive: true, force: true }) };
}

type ExecResult = { code: number; stdout: string };

const GITHUB_REMOTE = "origin\tgit@github.com:owner/repository.git (fetch)\n";
const BRANCH_NAME = "reused-branch";

function pullRequestPayload(number: number): ExecResult {
	return {
		code: 0,
		stdout: JSON.stringify({
			url: `https://github.com/owner/repository/pull/${number}`,
			number,
			state: "OPEN",
			isDraft: false,
			mergeStateStatus: "CLEAN",
		}),
	};
}

function makeExec(gh: (callIndex: number) => Promise<ExecResult>) {
	let remoteStdout = GITHUB_REMOTE;
	let ghCallCount = 0;
	const calls: Array<{ command: string; args: string[] }> = [];

	async function exec(command: string, args: string[]): Promise<ExecResult> {
		calls.push({ command, args });
		if (command === "git" && args.join(" ") === "rev-parse --show-toplevel") {
			return { code: 0, stdout: "/Users/user/repo\n" };
		}
		if (command === "git" && args.join(" ") === "branch --show-current") {
			return { code: 0, stdout: `${BRANCH_NAME}\n` };
		}
		if (command === "git" && args.join(" ") === "remote -v") {
			return { code: 0, stdout: remoteStdout };
		}
		if (command === "git" && args.join(" ") === "rev-parse --verify --quiet HEAD") {
			return { code: 1, stdout: "" };
		}
		if (command === "gh") {
			return gh(ghCallCount++);
		}
		throw new Error(`unexpected command: ${command} ${args.join(" ")}`);
	}

	return {
		exec,
		calls,
		setRemoteStdout: (next: string) => { remoteStdout = next; },
	};
}

function makeContext(exec: (command: string, args: string[]) => Promise<ExecResult>) {
	const handlers = new Map<string, (...args: unknown[]) => unknown>();
	const theme = { fg(_color: string, text: string) { return text; } };
	let widget: { dispose?(): void; render(width: number): string[] } | undefined;
	let footer: { dispose?(): void } | undefined;
	let branchChange: (() => void) | undefined;
	const api = {
		on(event: string, handler: (...args: unknown[]) => unknown) { handlers.set(event, handler); },
		events: { on() { return () => {}; } },
		async exec(command: string, args: string[]) { return exec(command, args); },
	};
	const ctx = {
		mode: "tui",
		cwd: "/Users/user/repo",
		model: { provider: "test", id: "model", contextWindow: 1 },
		thinkingLevel: "off",
		getContextUsage() { return undefined; },
		sessionManager: { getEntries: () => [] },
		ui: {
			setWorkingVisible() {},
			setFooter(factory: (tui: unknown, theme: unknown, footerData: { onBranchChange(callback: () => void): () => void }) => { dispose?(): void }) {
				footer = factory({ requestRender() {} }, theme, {
					onBranchChange(callback) {
						branchChange = callback;
						return () => {};
					},
				});
			},
			setWidget(_key: string, factory: (tui: unknown, theme: unknown) => { dispose?(): void; render(width: number): string[] }) {
				widget = factory({ requestRender() {} }, theme);
			},
		},
	};
	return {
		api,
		ctx,
		handlers,
		line: () => widget?.render(160)[1] ?? "",
		fireBranchChange: () => branchChange?.(),
		disposeAll: () => { widget?.dispose?.(); footer?.dispose?.(); },
	};
}

async function settle(times = 8): Promise<void> {
	for (let index = 0; index < times; index++) await new Promise((resolve) => setImmediate(resolve));
}

test("pure: isPullRequest accepts a complete payload and rejects partials", async () => {
	const extensions = await loadFooter();
	try {
		const { isPullRequest } = extensions.footer;
		const complete = {
			url: "https://github.com/owner/repository/pull/42",
			number: 42,
			state: "OPEN",
			isDraft: false,
			mergeStateStatus: "CLEAN",
		};
		assert.equal(isPullRequest(complete), true, "a complete payload is a pull request");
		for (const field of ["url", "number", "state", "isDraft", "mergeStateStatus"]) {
			const missing: Record<string, unknown> = { ...complete };
			delete missing[field];
			assert.equal(isPullRequest(missing), false, `a payload missing ${field} is not a pull request`);
			assert.equal(isPullRequest({ ...complete, [field]: null }), false, `a payload with a null ${field} is not a pull request`);
		}
		assert.equal(isPullRequest({ ...complete, number: "42" }), false, "a string number is not a pull request");
		assert.equal(isPullRequest({ ...complete, isDraft: "false" }), false, "a string isDraft is not a pull request");
		assert.equal(isPullRequest(undefined), false, "undefined is not a pull request");
		assert.equal(isPullRequest(null), false, "null is not a pull request");
		assert.equal(isPullRequest(42), false, "a number is not a pull request");
		assert.equal(isPullRequest("{}"), false, "a string is not a pull request");
	} finally {
		await extensions.dispose();
	}
});

test("the PR segment disappears when the same branch name comes back with no pull request", async () => {
	const extensions = await loadFooter();
	const exec = makeExec(async (callIndex) => callIndex === 0 ? pullRequestPayload(42) : { code: 1, stdout: "" });
	const harness = makeContext(exec.exec);
	try {
		extensions.footer.default(harness.api);
		await harness.handlers.get("session_start")?.({}, harness.ctx);
		await settle();
		assert.match(harness.line(), /PR #42/, "the incumbent pull request renders first");

		harness.fireBranchChange();
		await settle();
		const line = harness.line();
		assert.doesNotMatch(line, /PR #/, "a branch name with no pull request drops the PR segment");
		assert.doesNotMatch(line, /#42/, "the stale pull request number is gone entirely");
		assert.match(line, /repo/, "the workspace segment still renders");
		assert.match(line, /Ready/, "the status segment still renders");
	} finally {
		harness.disposeAll();
		await harness.handlers.get("session_shutdown")?.({}, harness.ctx);
		await extensions.dispose();
	}
});

test("a code-0 payload with a missing field clears the incumbent", async () => {
	const extensions = await loadFooter();
	const exec = makeExec(async (callIndex) =>
		callIndex === 0
			? pullRequestPayload(42)
			: { code: 0, stdout: JSON.stringify({ url: "https://github.com/owner/repository/pull/43", number: 43 }) }
	);
	const harness = makeContext(exec.exec);
	try {
		extensions.footer.default(harness.api);
		await harness.handlers.get("session_start")?.({}, harness.ctx);
		await settle();
		assert.match(harness.line(), /PR #42/, "the incumbent pull request renders first");

		harness.fireBranchChange();
		await settle();
		const line = harness.line();
		assert.doesNotMatch(line, /#42/, "an unusable payload clears the incumbent");
		assert.doesNotMatch(line, /#43/, "an unusable payload is never rendered");
	} finally {
		harness.disposeAll();
		await harness.handlers.get("session_shutdown")?.({}, harness.ctx);
		await extensions.dispose();
	}
});

test("losing the GitHub remote clears the incumbent", async () => {
	const extensions = await loadFooter();
	const exec = makeExec(async () => pullRequestPayload(42));
	const harness = makeContext(exec.exec);
	try {
		extensions.footer.default(harness.api);
		await harness.handlers.get("session_start")?.({}, harness.ctx);
		await settle();
		assert.match(harness.line(), /PR #42/, "the incumbent pull request renders first");

		exec.setRemoteStdout("");
		harness.fireBranchChange();
		await settle();
		assert.doesNotMatch(harness.line(), /#42/, "no GitHub remote means no PR segment");
	} finally {
		harness.disposeAll();
		await harness.handlers.get("session_shutdown")?.({}, harness.ctx);
		await extensions.dispose();
	}
});

test("a slow older lookup never clobbers a newer refresh", async () => {
	const extensions = await loadFooter();
	let releaseFirstLookup: (() => void) | undefined;
	const firstLookupGate = new Promise<void>((resolve) => { releaseFirstLookup = resolve; });
	const exec = makeExec(async (callIndex) => {
		if (callIndex === 0) {
			await firstLookupGate;
			return { code: 1, stdout: "" };
		}
		return pullRequestPayload(99);
	});
	const harness = makeContext(exec.exec);
	try {
		extensions.footer.default(harness.api);
		await harness.handlers.get("session_start")?.({}, harness.ctx);
		await settle();

		harness.fireBranchChange();
		await settle();
		assert.match(harness.line(), /PR #99/, "the newer refresh populates the segment");

		releaseFirstLookup?.();
		await settle();
		assert.match(harness.line(), /PR #99/, "the superseded lookup never clears the newer result");
	} finally {
		releaseFirstLookup?.();
		harness.disposeAll();
		await harness.handlers.get("session_shutdown")?.({}, harness.ctx);
		await extensions.dispose();
	}
});

test("a thrown gh lookup keeps the incumbent", async () => {
	const extensions = await loadFooter();
	const exec = makeExec(async (callIndex) => {
		if (callIndex === 0) return pullRequestPayload(42);
		throw new Error("gh spawn failed");
	});
	const harness = makeContext(exec.exec);
	try {
		extensions.footer.default(harness.api);
		await harness.handlers.get("session_start")?.({}, harness.ctx);
		await settle();
		assert.match(harness.line(), /PR #42/, "the incumbent pull request renders first");

		harness.fireBranchChange();
		await settle();
		assert.match(harness.line(), /PR #42/, "a lookup that never reported keeps the incumbent");
	} finally {
		harness.disposeAll();
		await harness.handlers.get("session_shutdown")?.({}, harness.ctx);
		await extensions.dispose();
	}
});
