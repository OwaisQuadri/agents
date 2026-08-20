import type { WatcherFactory } from "./types.ts";

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
	// TODO(AGNT-0015.T17): reject .git and its descendants, reject ignored paths.
	throw new Error("unimplemented");
}

/**
 * Default watcher factory over node:fs.watch with recursive watching.
 *
 * @param root worktree root to watch
 * @param onChange called with each changed path relative to root
 * @returns a watcher, or null when recursive watching is unavailable
 */
export const createWatcher: WatcherFactory = (root, onChange) => {
	// TODO(AGNT-0015.T17): fs.watch(root,{recursive:true}); never throw, return
	// null when unavailable.
	throw new Error("unimplemented");
};
