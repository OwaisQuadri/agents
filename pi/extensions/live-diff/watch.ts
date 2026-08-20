import { watch } from "node:fs";

import type { WatcherFactory, WorktreeWatcher } from "./types.ts";

const GIT_DIR = ".git";

/**
 * Decide whether a changed path should trigger a refresh.
 *
 * @param relativePath path relative to the worktree root
 * @param isIgnored answers whether git ignores the path
 * @returns false for git's own bookkeeping and for ignored paths
 */
export function isRefreshWorthy(
	relativePath: string,
	isIgnored: (path: string) => boolean,
): boolean {
	const normalized = relativePath.replaceAll("\\", "/");
	const segments = normalized.split("/");
	if (segments[0] === GIT_DIR) {
		return false;
	}
	return !isIgnored(relativePath);
}

/**
 * Default watcher factory over node:fs.watch with recursive watching.
 *
 * @param root worktree root to watch
 * @param onChange called with each changed path relative to root
 * @returns a watcher, or null when recursive watching is unavailable
 */
export const createWatcher: WatcherFactory = (root, onChange) => {
	let isClosed = false;
	try {
		const handle = watch(root, { recursive: true }, (_event, filename) => {
			if (isClosed || filename === null) {
				return;
			}
			onChange(typeof filename === "string" ? filename : filename.toString());
		});
		handle.on("error", () => {});
		const watcher: WorktreeWatcher = {
			close: () => {
				if (isClosed) {
					return;
				}
				isClosed = true;
				handle.close();
			},
		};
		return watcher;
	} catch {
		return null;
	}
};
