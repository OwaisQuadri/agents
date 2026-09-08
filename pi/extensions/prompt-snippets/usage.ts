import { constants } from "node:fs";
import { lstat, mkdir, open } from "node:fs/promises";
import { homedir } from "node:os";
import { dirname, join, resolve } from "node:path";

interface SnippetUsageRecord {
	version: 1;
	timestamp: string;
	sessionId: string;
	snippetIds: string[];
}

function usagePath(): string {
	const configuredDirectory = process.env.PI_CODING_AGENT_DIR?.trim();
	const root = configuredDirectory && configuredDirectory.length > 0 ? configuredDirectory : join(homedir(), ".pi", "agent");
	return join(resolve(root), "prompt-snippets-usage.jsonl");
}

function isNoSuchFileError(error: unknown): boolean {
	return error instanceof Error && "code" in error && (error as { code?: string }).code === "ENOENT";
}

async function isSymbolicLink(path: string): Promise<boolean> {
	try {
		return (await lstat(path)).isSymbolicLink();
	} catch (error) {
		if (isNoSuchFileError(error)) return false;
		throw error;
	}
}

async function hasUnsafeSymbolicLink(path: string): Promise<boolean> {
	const root = dirname(path);
	return (await Promise.all([dirname(root), root].map(isSymbolicLink))).some(Boolean);
}

/**
 * Appends content-free prompt-snippet usage for one transformed message.
 *
 * @param sessionId - The stable local Pi session identifier.
 * @param snippetIds - Active snippet file identifiers in injection order.
 * @returns A promise that resolves even if local storage fails.
 * @throws Never; storage failures are intentionally non-blocking.
 */
export async function recordSnippetUsage(sessionId: string, snippetIds: string[]): Promise<void> {
	if (snippetIds.length === 0) return;

	const record: SnippetUsageRecord = {
		version: 1,
		timestamp: new Date().toISOString(),
		sessionId,
		snippetIds: [...snippetIds],
	};

	try {
		const path = usagePath();
		const root = dirname(path);
		if (await hasUnsafeSymbolicLink(path)) return;
		await mkdir(root, { recursive: true });
		if (await hasUnsafeSymbolicLink(path)) return;
		const file = await open(path, constants.O_APPEND | constants.O_CREAT | constants.O_WRONLY | constants.O_NOFOLLOW, 0o600);
		try {
			await file.writeFile(`${JSON.stringify(record)}\n`, "utf8");
		} finally {
			await file.close();
		}
	} catch {}
}
