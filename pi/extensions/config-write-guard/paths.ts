import { existsSync, realpathSync } from "node:fs";
import { homedir } from "node:os";
import { basename, dirname, isAbsolute, relative, resolve, sep } from "node:path";

const protectedRelativePaths = [
	[".agents", "skills"],
	[".config", "herdr", "config.toml"],
	[".config", "simslim"],
	[".pi", "agent", "agents"],
	[".pi", "agent", "extensions"],
	[".pi", "agent", "keybindings.json"],
	[".pi", "agent", "settings.json"],
] as const;

/**
 * Returns the managed destination paths beneath a home directory.
 *
 * @param home The home directory that contains the agent destinations.
 * @returns Absolute paths that agents must not edit directly.
 * @throws Never.
 */
export function protectedConfigRoots(home = homedir()): string[] {
	return protectedRelativePaths.map((segments) => resolve(home, ...segments));
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
