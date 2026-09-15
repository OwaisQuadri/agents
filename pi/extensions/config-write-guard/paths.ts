import { existsSync, realpathSync } from "node:fs";
import { homedir } from "node:os";
import { basename, dirname, isAbsolute, relative, resolve, sep } from "node:path";

const protectedRelativePaths = [
	[".agents", "mcp.json"],
	[".agents", "mcp", "mcp.json"],
	[".agents", "skills"],
	[".config", "herdr", "config.toml"],
	[".config", "mcp", "mcp.json"],
	[".config", "simslim"],
	[".pi", "agent", "AGENTS.md"],
	[".pi", "agent", "agents"],
	[".pi", "agent", "extensions"],
	[".pi", "agent", "keybindings.json"],
	[".pi", "agent", "mcp.json"],
	[".pi", "agent", "models.json"],
	[".pi", "agent", "pi-transcribe.json"],
	[".pi", "agent", "plannotator.json"],
	[".pi", "agent", "settings.json"],
	[".pi", "agent", "themes", "owais.json"],
	[".pi", "extensions", "managed-config-guard"],
	[".pi", "extensions", "managed-config-guard.test.ts"],
	[".pi", "extensions", "managed-config-guard.ts"],
] as const;

/**
 * Returns the managed destination paths beneath a home directory.
 *
 * @param home The home directory that contains the agent destinations.
 * @returns Absolute paths that agents must not edit directly.
 * @throws Never.
 */
export function protectedConfigRoots(home = homedir()): string[] {
	const roots = protectedRelativePaths.map((segments) => resolve(home, ...segments));
	const customAgentConfig = resolve(piAgentDirectory(home), "mcp.json");
	if (!roots.includes(customAgentConfig)) roots.push(customAgentConfig);
	return roots;
}

/**
 * Returns the active Pi agent directory.
 *
 * @param home The home directory used to expand a configured tilde path.
 * @returns The absolute Pi agent directory.
 * @throws Never.
 */
export function piAgentDirectory(home = homedir()): string {
	const configured = process.env.PI_CODING_AGENT_DIR?.trim();
	if (!configured) return resolve(home, ".pi", "agent");
	if (configured === "~") return resolve(home);
	if (configured.startsWith("~/")) return resolve(home, configured.slice(2));
	return resolve(configured);
}

function isInside(root: string, candidate: string): boolean {
	const relativePath = relative(root, candidate);
	return relativePath === "" || (!relativePath.startsWith(`..${sep}`) && relativePath !== ".." && !isAbsolute(relativePath));
}

function canonicalPotentialPath(path: string, cwd: string): string {
	let existing = resolve(cwd, path);
	const missing: string[] = [];
	while (!existsSync(existing)) {
		const parent = dirname(existing);
		if (parent === existing) break;
		missing.push(basename(existing));
		existing = parent;
	}
	const canonicalExisting = existsSync(existing) ? realpathSync(existing) : existing;
	return resolve(canonicalExisting, ...missing.reverse());
}

export function isPathInsideRoot(path: string, root: string, cwd = process.cwd()): boolean {
	return isInside(canonicalPotentialPath(root, cwd), canonicalPotentialPath(path, cwd));
}

/**
 * Checks whether a path names a managed agent destination.
 *
 * @param path The path supplied to a Pi file tool.
 * @param home The home directory that contains the agent destinations.
 * @returns True when the resolved path is inside a protected destination.
 * @throws Never.
 */
export function isProtectedConfigPath(path: string, home = homedir()): boolean {
	const candidate = resolve(path);
	return protectedConfigRoots(home).some((root) => isInside(root, candidate) || isPathInsideRoot(path, root));
}
