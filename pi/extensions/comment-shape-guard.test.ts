import assert from "node:assert/strict";
import { chmod, cp, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { afterEach, beforeEach, test } from "node:test";

const extensionPath = fileURLToPath(new URL("./comment-shape-guard.ts", import.meta.url));
const subdirPath = fileURLToPath(new URL("./comment-shape-guard", import.meta.url));

const PI_CODING_AGENT_STUB = `
export function isToolCallEventType(toolName, event) {
	return event.toolName === toolName;
}
`;

async function writeStubModule(root: string, name: string, source: string): Promise<void> {
	const directory = join(root, "node_modules", ...name.split("/"));
	await mkdir(directory, { recursive: true });
	await writeFile(join(directory, "package.json"), '{"type":"module","exports":"./index.js"}');
	await writeFile(join(directory, "index.js"), source);
}

let root: string;
let realHome: string | undefined;
let realPath: string | undefined;

beforeEach(async () => {
	root = await mkdtemp(join(tmpdir(), "comment-shape-guard-e2e-"));
	await writeFile(join(root, "comment-shape-guard.ts"), await readFile(extensionPath));
	await cp(subdirPath, join(root, "comment-shape-guard"), { recursive: true, filter: (src) => !src.includes("node_modules") && !src.endsWith(".test.ts") });
	await writeStubModule(root, "@earendil-works/pi-coding-agent", PI_CODING_AGENT_STUB);
	await writeStubModule(root, "typebox", "export const Type = { Object: (v) => v, String: (v) => v };");
	realHome = process.env.HOME;
	realPath = process.env.PATH;
});

afterEach(async () => {
	await rm(root, { recursive: true, force: true });
	if (realHome !== undefined) process.env.HOME = realHome;
	if (realPath !== undefined) process.env.PATH = realPath;
});

/** Lays the scratch extension out as `<repoRoot>/pi/extensions/comment-shape-guard.ts`
 * so the guard's own `realpathSync(...).resolve("..","..")` repo-root walk lands on
 * `<repoRoot>`, matching its real deployed layout. */
async function stageRepo(): Promise<{ repoRoot: string; docPath: string }> {
	const repoRoot = await mkdtemp(join(tmpdir(), "comment-shape-guard-repo-"));
	const extDir = join(repoRoot, "pi", "extensions");
	await mkdir(extDir, { recursive: true });
	await cp(root, extDir, { recursive: true });
	// spawn/launch.ts resolves the judge's model tier via ../../tier-settings/model.ts —
	// the real repo module, staged alongside comment-shape-guard.ts the same way it sits
	// in the real pi/extensions/ layout.
	const tierSettingsSrc = fileURLToPath(new URL("./tier-settings/model.ts", import.meta.url));
	await mkdir(join(extDir, "tier-settings"), { recursive: true });
	await cp(tierSettingsSrc, join(extDir, "tier-settings", "model.ts"));
	const docPath = join(repoRoot, "docs", "comment-style.md");
	await mkdir(join(repoRoot, "docs"), { recursive: true });
	await writeFile(docPath, "## the whitelist\n\n- TODO — explicit and deliberate only\n- inexpressible concept or architecture\n");
	// comment-check binary: point at the real built one so span extraction is real.
	const binDir = join(repoRoot, "tools", "comment-check", "target", "release");
	await mkdir(binDir, { recursive: true });
	const realBinary = fileURLToPath(new URL("../../tools/comment-check/target/release/comment-check", import.meta.url));
	await cp(realBinary, join(binDir, "comment-check"));
	await chmod(join(binDir, "comment-check"), 0o755);
	return { repoRoot, docPath };
}

async function loadGuard(repoRoot: string): Promise<any> {
	const entry = join(repoRoot, "pi", "extensions", "comment-shape-guard.ts");
	return import(`${pathToFileURL(entry).href}?${Date.now()}-${Math.random()}`);
}

test("allows a write whose only comment already has a cached approved verdict — no worker spawn needed", async () => {
	const { repoRoot } = await stageRepo();
	process.env.HOME = repoRoot; // isolate the verdict cache to this test's own scratch tree
	const module = await loadGuard(repoRoot);
	const captured: { handler?: (e: unknown) => Promise<unknown> } = {};
	const api = { on: (_e: string, h: unknown) => (captured.handler = h as typeof captured.handler) };
	module.default(api);

	// pre-seed the cache so no subprocess is ever dispatched
	const { appendVerdict, hashCommentText } = await import(`${pathToFileURL(join(repoRoot, "pi", "extensions", "comment-shape-guard", "cache.ts")).href}?${Date.now()}`);
	appendVerdict({ hash: hashCommentText("// TODO(x): follow up later"), shape: "TODO", reason: "explicit and deliberate", judgedAt: new Date().toISOString() });

	const result = await captured.handler!({
		type: "tool_call",
		toolCallId: "1",
		toolName: "write",
		input: { path: join(repoRoot, "src", "new.rs"), content: "// TODO(x): follow up later\nfn main() {}\n" },
	});
	assert.equal(result, undefined);
	await rm(repoRoot, { recursive: true, force: true });
});

test("blocks a write whose only comment has a cached none verdict", async () => {
	const { repoRoot } = await stageRepo();
	process.env.HOME = repoRoot;
	const module = await loadGuard(repoRoot);
	const captured: { handler?: (e: unknown) => Promise<any> } = {};
	const api = { on: (_e: string, h: unknown) => (captured.handler = h) };
	module.default(api);

	const { appendVerdict, hashCommentText } = await import(`${pathToFileURL(join(repoRoot, "pi", "extensions", "comment-shape-guard", "cache.ts")).href}?${Date.now()}`);
	appendVerdict({ hash: hashCommentText("// narrates what RAG-0038 needed"), shape: "none", reason: "ticket narration, not a whitelist shape", judgedAt: new Date().toISOString() });

	const result = await captured.handler!({
		type: "tool_call",
		toolCallId: "1",
		toolName: "write",
		input: { path: join(repoRoot, "src", "new.rs"), content: "// narrates what RAG-0038 needed\nfn main() {}\n" },
	});
	assert.equal(result?.block, true);
	assert.ok(result?.reason.includes("does not fit any docs/comment-style.md whitelist shape"));
	assert.ok(result?.reason.includes("ticket narration"));
	await rm(repoRoot, { recursive: true, force: true });
});

test("skips a file extension comment-check does not recognize — no crash, no block", async () => {
	const { repoRoot } = await stageRepo();
	process.env.HOME = repoRoot;
	const module = await loadGuard(repoRoot);
	const captured: { handler?: (e: unknown) => Promise<any> } = {};
	const api = { on: (_e: string, h: unknown) => (captured.handler = h) };
	module.default(api);

	const result = await captured.handler!({
		type: "tool_call",
		toolCallId: "1",
		toolName: "write",
		input: { path: join(repoRoot, "README.md"), content: "# not a language comment-check parses\n" },
	});
	assert.equal(result, undefined);
	await rm(repoRoot, { recursive: true, force: true });
});

test("a non-edit non-write tool call is ignored entirely", async () => {
	const { repoRoot } = await stageRepo();
	process.env.HOME = repoRoot;
	const module = await loadGuard(repoRoot);
	const captured: { handler?: (e: unknown) => Promise<any> } = {};
	const api = { on: (_e: string, h: unknown) => (captured.handler = h) };
	module.default(api);

	const result = await captured.handler!({ type: "tool_call", toolCallId: "1", toolName: "bash", input: { command: "ls" } });
	assert.equal(result, undefined);
	await rm(repoRoot, { recursive: true, force: true });
});

test("a cache miss with no working judge binary on PATH fails open and logs to unverified.jsonl", async () => {
	const { repoRoot } = await stageRepo();
	process.env.HOME = repoRoot;
	// a fake `pi` on PATH that always exits non-zero — simulates "the judge worker could
	// not run" without ever spawning a real model.
	const fakeBinDir = join(repoRoot, "fakebin");
	await mkdir(fakeBinDir, { recursive: true });
	await writeFile(join(fakeBinDir, "pi"), "#!/bin/sh\nexit 1\n");
	await chmod(join(fakeBinDir, "pi"), 0o755);
	process.env.PATH = `${fakeBinDir}:${process.env.PATH}`;

	const module = await loadGuard(repoRoot);
	const captured: { handler?: (e: unknown) => Promise<any> } = {};
	const api = { on: (_e: string, h: unknown) => (captured.handler = h) };
	module.default(api);

	const result = await captured.handler!({
		type: "tool_call",
		toolCallId: "1",
		toolName: "write",
		input: { path: join(repoRoot, "src", "new.rs"), content: "// a brand new comment never judged before\nfn main() {}\n" },
	});
	assert.equal(result, undefined); // fail open
	const unverified = await readFile(join(repoRoot, ".local", "state", "comment-shape-guard", "unverified.jsonl"), "utf-8");
	assert.ok(unverified.trim().length > 0);
	await rm(repoRoot, { recursive: true, force: true });
});
