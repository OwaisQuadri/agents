/**
 * Shared test-only harness for the extension files in this directory (and `judge/`)
 * that import `typebox` / `@earendil-works/pi-coding-agent` at the top level. Neither
 * package is installed in this bare repo (only inside separately-packaged extensions
 * like observational-memory), so a plain `node --test` fails to resolve them. The
 * fix, matching `pi/extensions/ask-user-question.test.ts`'s own established pattern:
 * copy the file under test into a scratch directory, write minimal stub packages
 * into that directory's own `node_modules`, then dynamically import from there.
 */
import { cp, mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const TYPEBOX_STUB = `
export const Type = {
	Object: (value) => value,
	String: (value) => value,
	Optional: (value) => value,
	Array: (value) => value,
	Boolean: (value) => value,
};
`;

async function writeStubModule(root: string, name: string, source: string): Promise<void> {
	const directory = join(root, "node_modules", ...name.split("/"));
	await mkdir(directory, { recursive: true });
	await writeFile(join(directory, "package.json"), '{"type":"module","exports":"./index.js"}');
	await writeFile(join(directory, "index.js"), source);
}

/**
 * Copies this whole `comment-shape-guard/` directory (so relative imports between
 * `judge/`, `spawn/`, and `cache.ts` keep resolving) into a scratch root with stub
 * `typebox`/`pi-coding-agent` packages, then dynamically imports `relativeEntryPath`
 * (e.g. `"judge/agent/tool.ts"`) from that root. Returns the loaded module and a
 * `dispose()` to clean the scratch root up.
 */
export async function loadExtensionModule(relativeEntryPath: string): Promise<{ module: any; dispose: () => Promise<void> }> {
	const guardRoot = fileURLToPath(new URL(".", import.meta.url));
	const root = await mkdtemp(join(tmpdir(), "comment-shape-guard-test-"));
	await cp(guardRoot, root, { recursive: true, filter: (src) => !src.includes("node_modules") && !src.endsWith(".test.ts") });
	await writeStubModule(root, "typebox", TYPEBOX_STUB);
	await writeStubModule(root, "@earendil-works/pi-coding-agent", "");
	const entryPath = join(root, relativeEntryPath);
	const module = await import(`${pathToFileURL(entryPath).href}?${Date.now()}-${Math.random()}`);
	return {
		module,
		dispose: () => rm(root, { recursive: true, force: true }),
	};
}
