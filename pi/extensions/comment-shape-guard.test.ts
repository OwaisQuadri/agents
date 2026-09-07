import assert from "node:assert/strict";
import { chmod, cp, lstat, mkdir, mkdtemp, readFile, realpath, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { test, type TestContext } from "node:test";
import { execFile } from "node:child_process";
import { promisify } from "node:util";

const packageRoot = process.env.PI_PACKAGE_ROOT ?? "/opt/homebrew/lib/node_modules/@earendil-works/pi-coding-agent";
const nativeEdit = pathToFileURL(join(packageRoot, "dist/core/tools/edit.js")).href;
const nativeWrite = pathToFileURL(join(packageRoot, "dist/core/tools/write.js")).href;
await import(nativeEdit);
await import(nativeWrite);

async function fixture(t: TestContext) {
	const root = await realpath(await mkdtemp(join(tmpdir(), "guard-integration-")));
	t.after(() => rm(root, { recursive: true, force: true }));
	await mkdir(join(root, "config"));
	await cp(new URL("../../config/model-tiers.json", import.meta.url), join(root, "config/model-tiers.json"));
	const extensions = join(root, "pi", "extensions");
	await mkdir(extensions, { recursive: true });
	await cp(new URL("./comment-shape-guard.ts", import.meta.url), join(extensions, "comment-shape-guard.ts"));
	await cp(new URL("./comment-shape-guard/", import.meta.url), join(extensions, "comment-shape-guard"), { recursive: true, filter: (path) => !path.endsWith(".test.ts") && !path.includes("node_modules") });
	await mkdir(join(extensions, "tier-settings"));
	await cp(new URL("./tier-settings/model.ts", import.meta.url), join(extensions, "tier-settings/model.ts"));
	const moduleRoot = join(root, "node_modules", "@earendil-works", "pi-coding-agent");
	await mkdir(moduleRoot, { recursive: true });
	await writeFile(join(moduleRoot, "package.json"), JSON.stringify({ type: "module", exports: "./index.js" }));
	await writeFile(join(moduleRoot, "index.js"), `export { createEditToolDefinition } from ${JSON.stringify(nativeEdit)}; export { createWriteToolDefinition } from ${JSON.stringify(nativeWrite)};`);
	const entry = pathToFileURL(join(extensions, "comment-shape-guard.ts")).href;
	return { root, entry };
}

test("registration owns only native edit/write, with no initialization work", async (t) => {
	const { entry } = await fixture(t);
	await import(entry);
	for (let trial = 0; trial < 2; trial++) {
		const tools: any[] = [];
		const start = performance.now();
		const module = await import(`${entry}?trial=${trial}`);
		const imported = performance.now();
		module.default({ registerTool: (tool: unknown) => tools.push(tool) });
		const elapsed = performance.now() - start;
		console.log(`PI_TIMING=${process.env.PI_TIMING ?? "unset"} trial=${trial + 1} import=${(imported - start).toFixed(3)}ms factory=${(performance.now() - imported).toFixed(3)}ms combined=${elapsed.toFixed(3)}ms`);
		assert.deepEqual(tools.map((tool) => tool.name), ["edit", "write"]);
		assert.ok(elapsed <= 50, `warm import and registration ${elapsed}ms exceeds 50ms`);
	}
});

for (const operation of ["edit", "write"]) {
	test(`${operation}: missing checker blocks native mutation even for plain text`, async (t) => {
		const { root, entry } = await fixture(t);
		const tools: any[] = [];
		(await import(entry)).default({ registerTool: (tool: unknown) => tools.push(tool) });
		const path = join(root, "notes.txt");
		await writeFile(path, "original");
		await assert.rejects(tools.find((tool) => tool.name === operation).execute("fixture", { path, content: "new", edits: [{ oldText: "original", newText: "new" }] }, undefined, undefined, { cwd: root }), /required validation failed/);
		assert.equal(await readFile(path, "utf8"), "original");
	});
}

for (const terminal of ["pass", "mismatch", "extra"]) {
	test(`native wrapper accepts only a matching single terminal pass: ${terminal}`, async (t) => {
		const { root, entry } = await fixture(t);
		const tools: any[] = [];
		(await import(entry)).default({ registerTool: (tool: unknown) => tools.push(tool) });
		const directory = join(root, "tools", "edit-time-check", "target", "release");
		await mkdir(directory, { recursive: true });
		const binary = join(directory, "edit-time-check");
		await writeFile(binary, `#!/usr/bin/env node\nlet raw=""; process.stdin.setEncoding("utf8"); process.stdin.on("data", x => raw += x); process.stdin.on("end", () => { const request=JSON.parse(raw); const response={version:1,request_id:${terminal === "mismatch" ? JSON.stringify("other") : "request.request_id"},decision:"pass",diagnostics:[],judgments:[],elapsed_ms:1,budget_ms:request.budget_ms}; const encoded=JSON.stringify(response); process.stdout.write(encoded${terminal === "extra" ? "+encoded" : ""}); });`);
		await chmod(binary, 0o755);
		const path = join(root, "notes.txt");
		await writeFile(path, "original");
		const promise = tools.find((tool) => tool.name === "write").execute("fixture", { path, content: "candidate" }, undefined, undefined, { cwd: root });
		if (terminal === "pass") {
			await promise;
			assert.equal(await readFile(path, "utf8"), "candidate");
		} else {
			await assert.rejects(promise, /required validation failed/);
			assert.equal(await readFile(path, "utf8"), "original");
		}
	});
}

for (const decision of ["pass", "needs_judgment"]) {
	test(`reduced ${decision} budget counts native queue wait before validation`, async (t) => {
		const { root, entry } = await fixture(t);
		const tools: any[] = [];
		(await import(entry)).default({ registerTool: (tool: unknown) => tools.push(tool) });
		const directory = join(root, "tools/edit-time-check/target/release");
		await mkdir(directory, { recursive: true });
		const binary = join(directory, "edit-time-check");
		const previousPath = process.env.PATH;
		await mkdir(join(root, "bin"));
		process.env.PATH = `${join(root, "bin")}:${previousPath}`;
		t.after(() => { if (previousPath === undefined) delete process.env.PATH; else process.env.PATH = previousPath; });
		await writeFile(join(root, "bin/pi"), `#!/usr/bin/env node\nconst fs=require("node:fs"); fs.writeFileSync(${JSON.stringify(join(root, "judge-called"))}, "called"); fs.writeFileSync(process.env.CSG_RESULT_PATH, JSON.stringify({shape:"TODO",reason:"explicit follow-up"}));`);
		await chmod(join(root, "bin/pi"), 0o755);
		const input = { comment: "TODO", code_context: "", language: "typescript", rule_document: "rules", prompt_version: 1, schema_version: 1, judgment_configuration: "fixture" };
		await writeFile(binary, `#!/usr/bin/env node\nlet raw=""; process.stdin.on("data", x => raw += x); process.stdin.on("end", () => { const r=JSON.parse(raw); if(r.budget_ms!==20000 || process.argv[2]!=="--config") process.exit(2); process.stdout.write(JSON.stringify({version:1,request_id:r.request_id,decision:${JSON.stringify(decision)},diagnostics:[],judgments:${JSON.stringify(decision === "pass" ? [] : [{ line: 1, input }])},elapsed_ms:1,budget_ms:500})); });`);
		await chmod(binary, 0o755);
		const path = join(root, "candidate.ts");
		await writeFile(path, "original");
		const before = await lstat(path);
		let release!: () => void;
		let entered!: () => void;
		const waiting = new Promise<void>((resolve) => { entered = resolve; });
		const gate = new Promise<void>((resolve) => { release = resolve; });
		const native = (await import(nativeWrite)).createWriteToolDefinition(root, { operations: { mkdir: async () => {}, writeFile: async () => { entered(); await gate; } } });
		const holder = native.execute("holder", { path, content: "unused" });
		await waiting;
		const result = tools.find((tool) => tool.name === "write").execute("queued", { path, content: "candidate" }, undefined, undefined, { cwd: root });
		await new Promise((resolve) => setTimeout(resolve, 600));
		release();
		await holder;
		await assert.rejects(result, /deadline exceeded/);
		await assert.rejects(lstat(join(root, "judge-called")), { code: "ENOENT" });
		assert.equal(await readFile(path, "utf8"), "original");
		const after = await lstat(path);
		for (const field of ["dev", "ino", "mode", "nlink", "size", "mtimeMs", "ctimeMs"] as const) assert.equal(after[field], before[field]);
	});
}

test("built Rust checker crosses the native Pi boundary with config and unchanged rejection", async (t) => {
	const { root, entry } = await fixture(t);
	const target = await mkdtemp(join(tmpdir(), "guard-rust-target-"));
	t.after(() => rm(target, { recursive: true, force: true }));
	const build = await promisify(execFile)("cargo", ["build", "--release", "--locked", "--manifest-path", new URL("../../tools/edit-time-check/Cargo.toml", import.meta.url).pathname, "--target-dir", target], { env: { ...process.env, RUSTFLAGS: "-D warnings" }, maxBuffer: 4 * 1024 * 1024 });
	t.diagnostic(`isolated Rust build: ${build.stdout}${build.stderr}`);
	await mkdir(join(root, "docs"));
	for (const name of ["code-style.md", "comment-style.md"]) await cp(new URL(`../../docs/${name}`, import.meta.url), join(root, "docs", name));
	await cp(new URL("../../config/edit-time.toml", import.meta.url), join(root, "config/edit-time.toml"));
	await mkdir(join(root, "tools/edit-time-check/target/release"), { recursive: true });
	await symlink(join(target, "release/edit-time-check"), join(root, "tools/edit-time-check/target/release/edit-time-check"));
	await mkdir(join(root, ".git"));
	const tools: any[] = [];
	(await import(entry)).default({ registerTool: (tool: unknown) => tools.push(tool) });
	const path = join(root, "candidate.ts");
	const write = (content: string) => tools.find((tool) => tool.name === "write").execute("real", { path, content }, undefined, undefined, { cwd: root });
	const start = performance.now();
	const result = await write("const isReady = true;\n");
	assert.equal(result.content[0].text, `Successfully wrote to ${path}`);
	assert.equal(await readFile(path, "utf8"), "const isReady = true;\n");
	t.diagnostic(`real process + native write clean candidate: ${(performance.now() - start).toFixed(3)}ms; judgments=0`);
	const before = await lstat(path);
	await assert.rejects(write("const ready = true;\n"), (error: Error) => {
		assert.match(error.message, /candidate\.ts":1: boolean-name: Rename/);
		assert.doesNotMatch(error.message, /const ready|required validation failed/);
		return true;
	});
	assert.equal(await readFile(path, "utf8"), "const isReady = true;\n");
	const afterBlock = await lstat(path);
	for (const field of ["dev", "ino", "mode", "nlink", "size", "mtimeMs", "ctimeMs"] as const) assert.equal(afterBlock[field], before[field]);
	await writeFile(join(root, ".edit-time.toml"), "version = 1\ntotal_ms = 1\n");
	await assert.rejects(write("const isDone = true;\n"), (error: Error) => {
		assert.match(error.message, /(?:deadline exceeded|Blocked before write)/);
		assert.doesNotMatch(error.message, /privacy|const isDone/);
		return true;
	});
	assert.equal(await readFile(path, "utf8"), "const isReady = true;\n");
	const after = await lstat(path);
	for (const field of ["dev", "ino", "mode", "nlink", "size", "mtimeMs", "ctimeMs"] as const) assert.equal(after[field], before[field]);
});

test("reduced deadline remains active during filesystem recheck before commit", async (t) => {
	const { root, entry } = await fixture(t);
	const directory = join(root, "pi/extensions/comment-shape-guard");
	await writeFile(join(directory, "validate.ts"), "export async function validateProposal(proposal, start, deadline) { return Math.min(deadline, performance.now() + 50); }");
	const direct = await readFile(join(directory, "direct.ts"), "utf8");
	await writeFile(join(directory, "direct.ts"), direct.replace("const current = await snapshot(path);", "await new Promise(resolve => setTimeout(resolve, 100));\nconst current = await snapshot(path);"));
	const tools: any[] = [];
	(await import(entry)).default({ registerTool: (tool: unknown) => tools.push(tool) });
	const path = join(root, "candidate.txt");
	await writeFile(path, "original");
	const before = await lstat(path);
	await assert.rejects(tools.find((tool) => tool.name === "write").execute("fixture", { path, content: "candidate" }, undefined, undefined, { cwd: root }), /deadline exceeded/);
	assert.equal(await readFile(path, "utf8"), "original");
	const after = await lstat(path);
	for (const field of ["dev", "ino", "mode", "nlink", "size", "mtimeMs", "ctimeMs"] as const) assert.equal(after[field], before[field]);
});

test("validation queue time uses the original execution start", async (t) => {
	const { root } = await fixture(t);
	const directory = join(root, "tools/edit-time-check/target/release");
	await mkdir(directory, { recursive: true });
	const binary = join(directory, "edit-time-check");
	const marker = join(root, "checker-started");
	await writeFile(binary, `#!/usr/bin/env node\nlet raw=""; process.stdin.on("data", x => raw+=x); process.stdin.on("end", () => { const r=JSON.parse(raw); require("node:fs").writeFileSync(${JSON.stringify(marker)}, "started"); setTimeout(() => process.stdout.write(JSON.stringify({version:1,request_id:r.request_id,decision:"pass",diagnostics:[],judgments:[],elapsed_ms:1,budget_ms:r.proposed_text==="first"?20000:500})), r.proposed_text==="first"?700:0); });`);
	await chmod(binary, 0o755);
	const { validateProposal } = await import(pathToFileURL(join(root, "pi/extensions/comment-shape-guard/validate.ts")).href);
	const proposal = { operation: "write", path: join(root, "candidate.txt"), repository_root: null, original_text: null, proposed_text: "first" };
	const start = performance.now();
	const first = validateProposal(proposal, start, start + 20_000);
	while (true) {
		try { await lstat(marker); break; } catch (error) { if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error; }
		assert.ok(performance.now() - start < 3000);
		await new Promise((resolve) => setTimeout(resolve, 5));
	}
	const secondStart = performance.now();
	const second = assert.rejects(validateProposal({ ...proposal, proposed_text: "second" }, secondStart, secondStart + 20_000), /deadline exceeded/);
	assert.equal(await first, start + 20_000);
	await second;
});

for (const interruption of ["deadline", "cancel", "release-race"]) {
	test(`validation queue ${interruption} settles promptly without launching or overtaking`, async (t) => {
		const { root } = await fixture(t);
		const directory = join(root, "pi/extensions/comment-shape-guard");
		await writeFile(join(directory, "process.ts"), `
export const calls = [];
let release;
const gate = new Promise(resolve => { release = resolve; });
export { release };
export async function runProcess(options) {
 const request = JSON.parse(options.input);
 calls.push(request.proposed_text);
 if (request.proposed_text === "first") await gate;
 return { exitCode: 0, stdout: JSON.stringify({ version: 1, request_id: request.request_id, decision: "pass", diagnostics: [], judgments: [], elapsed_ms: 0, budget_ms: request.budget_ms }) };
}`);
		const transport = await import(pathToFileURL(join(directory, "process.ts")).href);
		const { validateProposal } = await import(pathToFileURL(join(directory, "validate.ts")).href);
		const proposal = { operation: "write", path: join(root, "candidate.txt"), repository_root: null, original_text: null, proposed_text: "first" };
		const start = performance.now();
		const first = validateProposal(proposal, start, start + 3000);
		t.after(() => transport.release());
		while (!transport.calls.length) {
			assert.ok(performance.now() - start < 1000);
			await new Promise(resolve => setTimeout(resolve, 1));
		}
		const controller = new AbortController();
		const queued = performance.now();
		const second = assert.rejects(validateProposal({ ...proposal, proposed_text: "expired" }, queued, queued + (interruption === "deadline" ? 60 : 3000), controller.signal), interruption === "deadline" ? /deadline exceeded/ : /aborted before commit/);
		if (interruption === "release-race") transport.release();
		if (interruption !== "deadline") controller.abort();
		await second;
		const elapsed = performance.now() - queued;
		assert.ok(elapsed < 500, `queued interruption took ${elapsed}ms`);
		const thirdStart = performance.now();
		const third = validateProposal({ ...proposal, proposed_text: "third" }, thirdStart, thirdStart + 3000);
		if (interruption !== "release-race") {
			await new Promise(resolve => setTimeout(resolve, 30));
			assert.deepEqual(transport.calls, ["first"]);
			transport.release();
		}
		await first;
		await third;
		assert.deepEqual(transport.calls, ["first", "third"]);
		t.diagnostic(`queue ${interruption} response=${elapsed.toFixed(3)}ms; expired worker launches=0; ordered launches=first,third`);
	});
}

test("checker process deadline is not rewritten as build advice", async (t) => {
	const { root } = await fixture(t);
	const directory = join(root, "pi/extensions/comment-shape-guard");
	await writeFile(join(directory, "process.ts"), `import { ValidationExpired } from "./direct.ts"; export async function runProcess() { throw new ValidationExpired(); }`);
	const { validateProposal } = await import(pathToFileURL(join(directory, "validate.ts")).href);
	const start = performance.now();
	await assert.rejects(validateProposal({ operation: "write", path: "/fixture.ts", repository_root: null, original_text: null, proposed_text: "candidate" }, start, start + 3000), (error: Error) => {
		assert.match(error.message, /deadline exceeded/);
		assert.doesNotMatch(error.message, /Build|repair|required validation failed/);
		return true;
	});
});
