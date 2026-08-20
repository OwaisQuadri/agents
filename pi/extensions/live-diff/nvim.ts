import type { Exec } from "./engine.ts";

const HERDR_CANDIDATES = ["herdr", "/opt/homebrew/bin/herdr"];

interface HerdrWorkspace {
	workspace_id: string;
	worktree?: { checkout_path: string };
}

interface HerdrTab {
	tab_id: string;
	workspace_id: string;
	label: string;
}

interface HerdrPane {
	pane_id: string;
	tab_id: string;
}

interface HerdrSnapshot {
	workspaces: HerdrWorkspace[];
	tabs: HerdrTab[];
	panes: HerdrPane[];
}

async function runHerdr(
	exec: Exec,
	herdrBin: string,
	args: string[],
): Promise<string | null> {
	try {
		const result = await exec(herdrBin, args);
		if (result.code !== 0) return null;
		return result.stdout;
	} catch {
		return null;
	}
}

async function resolveHerdr(exec: Exec): Promise<{ bin: string; snapshotJson: string } | null> {
	for (const bin of HERDR_CANDIDATES) {
		const out = await runHerdr(exec, bin, ["api", "snapshot"]);
		if (out !== null) return { bin, snapshotJson: out };
	}
	return null;
}

function parseSnapshot(json: string): HerdrSnapshot | null {
	try {
		const parsed = JSON.parse(json);
		const snapshot = parsed?.result?.snapshot;
		if (!snapshot?.workspaces || !snapshot?.tabs || !snapshot?.panes) return null;
		return snapshot as HerdrSnapshot;
	} catch {
		return null;
	}
}

function findWorkspaceId(snapshot: HerdrSnapshot, cwd: string): string | null {
	let bestId: string | null = null;
	let bestLength = -1;
	for (const workspace of snapshot.workspaces) {
		const checkoutPath = workspace.worktree?.checkout_path;
		if (!checkoutPath) continue;
		const isMatch = cwd === checkoutPath || cwd.startsWith(checkoutPath + "/");
		if (isMatch && checkoutPath.length > bestLength) {
			bestId = workspace.workspace_id;
			bestLength = checkoutPath.length;
		}
	}
	return bestId;
}

async function findNvimPaneId(
	exec: Exec,
	herdrBin: string,
	panes: HerdrPane[],
): Promise<string | null> {
	for (const pane of panes) {
		const out = await runHerdr(exec, herdrBin, [
			"pane",
			"process-info",
			"--pane",
			pane.pane_id,
		]);
		if (out === null) continue;
		try {
			const info = JSON.parse(out)?.result?.process_info;
			const processes: { name?: string }[] = info?.foreground_processes ?? [];
			if (processes.some((process) => process.name === "nvim")) {
				return pane.pane_id;
			}
		} catch {
			continue;
		}
	}
	return null;
}

/**
 * Focus the herdr editor tab of the workspace owning cwd and open the file
 * in its nvim pane.
 *
 * @param exec command runner
 * @param cwd repository worktree root
 * @param path file path relative to cwd
 * @returns true when a workspace, editor tab, and nvim pane were all found
 *   and the open sequence was sent; false otherwise (never throws)
 */
export async function openInNvim(
	exec: Exec,
	cwd: string,
	path: string,
): Promise<boolean> {
	try {
		const resolved = await resolveHerdr(exec);
		if (resolved === null) return false;
		const { bin, snapshotJson } = resolved;

		const snapshot = parseSnapshot(snapshotJson);
		if (snapshot === null) return false;

		const workspaceId = findWorkspaceId(snapshot, cwd);
		if (workspaceId === null) return false;

		const editorTab = snapshot.tabs.find(
			(tab) => tab.workspace_id === workspaceId && tab.label === "editor",
		);
		if (!editorTab) return false;

		const tabPanes = snapshot.panes.filter(
			(pane) => pane.tab_id === editorTab.tab_id,
		);
		const nvimPaneId = await findNvimPaneId(exec, bin, tabPanes);
		if (nvimPaneId === null) return false;

		const focusOut = await runHerdr(exec, bin, ["tab", "focus", editorTab.tab_id]);
		if (focusOut === null) return false;

		const absolutePath = path.startsWith("/") ? path : cwd + "/" + path;
		if (/[\n\r\t]/.test(absolutePath)) return false;
		const escapedPath = absolutePath.replace(
			/[\\ |%#]/g,
			(char) => "\\" + char,
		);
		const escOut = await runHerdr(exec, bin, ["pane", "send-keys", nvimPaneId, "esc"]);
		if (escOut === null) return false;
		const runOut = await runHerdr(exec, bin, [
			"pane",
			"run",
			nvimPaneId,
			":e " + escapedPath,
		]);
		return runOut !== null;
	} catch {
		return false;
	}
}
