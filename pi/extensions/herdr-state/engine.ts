import type {
	HerdrAgentLocation,
	HerdrPane,
	HerdrRawEvent,
	HerdrRawPane,
	HerdrRawTab,
	HerdrRawWorkspace,
	HerdrRawWorktree,
	HerdrSessionSnapshot,
	HerdrSnapshotResponse,
	HerdrStateEvent,
	HerdrStateModel,
	HerdrTab,
	HerdrWorkspace,
	HerdrWorktree,
} from "./types.ts";

function normalizeWorktree(raw: HerdrRawWorktree | undefined): HerdrWorktree | null {
	if (raw === undefined) {
		return null;
	}
	if (typeof raw.checkout_path !== "string" || raw.checkout_path === "") {
		throw new Error("invalid Herdr worktree: missing checkout_path");
	}
	return { path: raw.checkout_path, branch: null };
}

function normalizeWorkspace(raw: HerdrRawWorkspace): HerdrWorkspace {
	if (typeof raw?.workspace_id !== "string" || raw.workspace_id === "") {
		throw new Error("invalid Herdr workspace: missing workspace_id");
	}
	if (typeof raw.label !== "string") {
		throw new Error(`invalid Herdr workspace ${raw.workspace_id}: missing label`);
	}
	if (typeof raw.focused !== "boolean") {
		throw new Error(`invalid Herdr workspace ${raw.workspace_id}: focused must be boolean`);
	}
	return {
		id: raw.workspace_id,
		label: raw.label,
		worktree: normalizeWorktree(raw.worktree),
		isFocused: raw.focused,
	};
}

function normalizeTab(raw: HerdrRawTab): HerdrTab {
	if (typeof raw?.tab_id !== "string" || raw.tab_id === "") {
		throw new Error("invalid Herdr tab: missing tab_id");
	}
	if (typeof raw.workspace_id !== "string" || raw.workspace_id === "") {
		throw new Error(`invalid Herdr tab ${raw.tab_id}: missing workspace_id`);
	}
	if (typeof raw.label !== "string") {
		throw new Error(`invalid Herdr tab ${raw.tab_id}: missing label`);
	}
	if (typeof raw.focused !== "boolean") {
		throw new Error(`invalid Herdr tab ${raw.tab_id}: focused must be boolean`);
	}
	return {
		id: raw.tab_id,
		workspaceId: raw.workspace_id,
		label: raw.label,
		isFocused: raw.focused,
	};
}

function normalizePane(raw: HerdrRawPane): HerdrPane {
	if (typeof raw?.pane_id !== "string" || raw.pane_id === "") {
		throw new Error("invalid Herdr pane: missing pane_id");
	}
	if (typeof raw.workspace_id !== "string" || raw.workspace_id === "") {
		throw new Error(`invalid Herdr pane ${raw.pane_id}: missing workspace_id`);
	}
	if (typeof raw.tab_id !== "string" || raw.tab_id === "") {
		throw new Error(`invalid Herdr pane ${raw.pane_id}: missing tab_id`);
	}
	if (typeof raw.focused !== "boolean") {
		throw new Error(`invalid Herdr pane ${raw.pane_id}: focused must be boolean`);
	}
	return {
		id: raw.pane_id,
		workspaceId: raw.workspace_id,
		tabId: raw.tab_id,
		label: raw.pane_id,
		cwd: typeof raw.cwd === "string" ? raw.cwd : null,
		isFocused: raw.focused,
	};
}

function isSnapshotShapeValid(snapshot: HerdrSessionSnapshot): boolean {
	return (
		snapshot !== null &&
		typeof snapshot === "object" &&
		Array.isArray(snapshot.workspaces) &&
		Array.isArray(snapshot.tabs) &&
		Array.isArray(snapshot.panes)
	);
}

function requireValidSnapshot(snapshot: HerdrSessionSnapshot): void {
	if (!isSnapshotShapeValid(snapshot)) {
		throw new Error("invalid Herdr session snapshot");
	}
}

/**
 * Normalizes a Herdr socket response into the read-only session model.
 *
 * @param response The Herdr snapshot response to normalize.
 * @returns The normalized session snapshot.
 * @throws Error when the response cannot be normalized.
 */
export function normalizeSnapshot(
	response: HerdrSnapshotResponse,
): HerdrSessionSnapshot {
	if (typeof response?.id !== "string" || response.id === "") {
		throw new Error("invalid Herdr snapshot response: missing id");
	}
	if (response.result?.type !== "session_snapshot") {
		throw new Error("invalid Herdr snapshot response: missing result.type");
	}
	const raw = response.result.snapshot;
	if (
		raw === undefined ||
		raw === null ||
		!Array.isArray(raw.workspaces) ||
		!Array.isArray(raw.tabs) ||
		!Array.isArray(raw.panes)
	) {
		throw new Error(
			"invalid Herdr snapshot response: missing result.snapshot workspaces, tabs, or panes",
		);
	}
	return {
		workspaces: raw.workspaces.map(normalizeWorkspace),
		tabs: raw.tabs.map(normalizeTab),
		panes: raw.panes.map(normalizePane),
		focusedWorkspaceId:
			typeof raw.focused_workspace_id === "string" ? raw.focused_workspace_id : null,
		focusedTabId: typeof raw.focused_tab_id === "string" ? raw.focused_tab_id : null,
		focusedPaneId: typeof raw.focused_pane_id === "string" ? raw.focused_pane_id : null,
	};
}

/**
 * Normalizes one Herdr subscription event for the read-only state model.
 *
 * @param event The raw Herdr event to normalize.
 * @returns A state event, or null when a full snapshot is required.
 * @throws Error when a recognized event is malformed.
 */
export function normalizeEvent(
	event: HerdrRawEvent,
): HerdrStateEvent | null {
	if (event?.type === "workspace_created" || event?.type === "workspace_updated") {
		return { type: "workspace-changed", workspace: normalizeWorkspace(event.workspace) };
	}
	if (event?.type === "tab_created") {
		return { type: "tab-changed", tab: normalizeTab(event.tab) };
	}
	if (event?.type === "pane_created" || event?.type === "pane_updated") {
		return { type: "pane-changed", pane: normalizePane(event.pane) };
	}
	return null;
}

/**
 * Locates this Pi session in a normalized Herdr snapshot.
 *
 * @param snapshot The normalized Herdr session snapshot.
 * @param cwd The Pi process working directory.
 * @param paneId The optional Herdr pane identifier from the Pi process environment.
 * @returns The Pi location, or null when the session has no Herdr location.
 * @throws Error when the location data is invalid.
 */
export function findSelf(
	snapshot: HerdrSessionSnapshot,
	cwd: string,
	paneId: string | undefined,
): HerdrAgentLocation | null {
	requireValidSnapshot(snapshot);
	if (typeof cwd !== "string" || cwd === "") {
		throw new Error("invalid Pi working directory");
	}
	let pane: HerdrPane | undefined;
	if (paneId !== undefined) {
		pane = snapshot.panes.find((candidate) => candidate.id === paneId);
	} else {
		const matches = snapshot.panes.filter((candidate) => candidate.cwd === cwd);
		if (matches.length !== 1) {
			return null;
		}
		[pane] = matches;
	}
	if (pane === undefined) {
		return null;
	}
	return {
		workspaceId: pane.workspaceId,
		tabId: pane.tabId,
		paneId: pane.id,
		isSelf: true,
	};
}

/**
 * Creates the initial read-only Herdr state model.
 *
 * @param snapshot The normalized Herdr session snapshot.
 * @param self The Pi location, or null when it is unavailable.
 * @returns The initial Herdr state model.
 * @throws Error when the snapshot is invalid.
 */
export function createModel(
	snapshot: HerdrSessionSnapshot,
	self: HerdrAgentLocation | null,
): HerdrStateModel {
	requireValidSnapshot(snapshot);
	return {
		snapshot,
		self,
		selectedWorkspaceId: null,
		selectedPaneId: null,
		output: null,
	};
}

function upsertById<T extends { id: string }>(items: T[], item: T): T[] {
	const index = items.findIndex((existing) => existing.id === item.id);
	if (index === -1) {
		return [...items, item];
	}
	const next = [...items];
	next[index] = item;
	return next;
}

function replaceSnapshot(
	model: HerdrStateModel,
	snapshot: HerdrSessionSnapshot,
): HerdrStateModel {
	requireValidSnapshot(snapshot);
	const isSelfPresent =
		model.self !== null && snapshot.panes.some((pane) => pane.id === model.self?.paneId);
	const isSelectedWorkspacePresent =
		model.selectedWorkspaceId !== null &&
		snapshot.workspaces.some((workspace) => workspace.id === model.selectedWorkspaceId);
	const isSelectedPanePresent =
		model.selectedPaneId !== null &&
		snapshot.panes.some((pane) => pane.id === model.selectedPaneId);
	return {
		snapshot,
		self: isSelfPresent ? model.self : null,
		selectedWorkspaceId: isSelectedWorkspacePresent ? model.selectedWorkspaceId : null,
		selectedPaneId: isSelectedPanePresent ? model.selectedPaneId : null,
		output: null,
	};
}

/**
 * Applies one Herdr event to the current state model.
 *
 * @param model The current Herdr state model.
 * @param event The Herdr event to apply.
 * @returns The updated Herdr state model.
 * @throws Error when the event is invalid.
 */
export function applyEvent(
	model: HerdrStateModel,
	event: HerdrStateEvent,
): HerdrStateModel {
	if (model === null || typeof model !== "object") {
		throw new Error("invalid Herdr state model");
	}
	requireValidSnapshot(model.snapshot);
	switch (event?.type) {
		case "snapshot-replaced":
			return replaceSnapshot(model, event.snapshot);
		case "workspace-changed":
			return {
				...model,
				snapshot: {
					...model.snapshot,
					workspaces: upsertById(model.snapshot.workspaces, event.workspace),
				},
			};
		case "tab-changed":
			return {
				...model,
				snapshot: {
					...model.snapshot,
					tabs: upsertById(model.snapshot.tabs, event.tab),
				},
			};
		case "pane-changed":
			return {
				...model,
				snapshot: {
					...model.snapshot,
					panes: upsertById(model.snapshot.panes, event.pane),
				},
			};
		default:
			throw new Error(`invalid Herdr state event: ${JSON.stringify(event)}`);
	}
}
