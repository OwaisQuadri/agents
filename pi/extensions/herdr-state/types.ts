export interface HerdrWorktree {
	path: string;
	branch: string | null;
}

export interface HerdrWorkspace {
	id: string;
	label: string;
	worktree: HerdrWorktree | null;
	isFocused: boolean;
}

export interface HerdrTab {
	id: string;
	workspaceId: string;
	label: string;
	isFocused: boolean;
}

export interface HerdrPane {
	id: string;
	workspaceId: string;
	tabId: string;
	label: string;
	cwd: string | null;
	isFocused: boolean;
}

export interface HerdrAgentLocation {
	workspaceId: string;
	tabId: string;
	paneId: string;
	isSelf: boolean;
}

export interface HerdrSessionSnapshot {
	workspaces: HerdrWorkspace[];
	tabs: HerdrTab[];
	panes: HerdrPane[];
	focusedWorkspaceId: string | null;
	focusedTabId: string | null;
	focusedPaneId: string | null;
}

export interface HerdrPaneOutput {
	paneId: string;
	text: string;
	isTruncated: boolean;
}

export interface HerdrStateModel {
	snapshot: HerdrSessionSnapshot;
	self: HerdrAgentLocation | null;
	selectedWorkspaceId: string | null;
	selectedPaneId: string | null;
	output: HerdrPaneOutput | null;
}

export type HerdrStateEvent =
	| { type: "snapshot-replaced"; snapshot: HerdrSessionSnapshot }
	| { type: "workspace-changed"; workspace: HerdrWorkspace }
	| { type: "tab-changed"; tab: HerdrTab }
	| { type: "pane-changed"; pane: HerdrPane };

export interface HerdrStateFailure {
	code: "unavailable" | "not-found" | "invalid-response";
	message: string;
}

// TODO(AGNT-0066.T08): Implement the live state controller after stash restoration.
export interface HerdrStateController {
	start(cwd: string, paneId: string | undefined): Promise<void>;
	current(): HerdrStateModel | null;
	stop(): void;
}

export interface HerdrRawWorktree {
	checkout_path: string;
	is_linked_worktree: boolean;
	repo_name: string;
}

export interface HerdrRawWorkspace {
	workspace_id: string;
	label: string;
	focused?: unknown;
	worktree?: HerdrRawWorktree;
}

export interface HerdrRawTab {
	tab_id: string;
	workspace_id: string;
	label: string;
	focused?: unknown;
}

export interface HerdrRawPane {
	pane_id: string;
	workspace_id: string;
	tab_id: string;
	cwd?: string;
	focused?: unknown;
}

export interface HerdrRawAgent {
	agent?: string;
	cwd?: string;
	pane_id: string;
	tab_id: string;
	workspace_id: string;
}

export interface HerdrRawSnapshot {
	focused_workspace_id: string | null;
	focused_tab_id: string | null;
	focused_pane_id: string | null;
	workspaces: HerdrRawWorkspace[];
	tabs: HerdrRawTab[];
	panes: HerdrRawPane[];
	agents: HerdrRawAgent[];
}

export interface HerdrSnapshotResult {
	type: "session_snapshot";
	snapshot: HerdrRawSnapshot;
}

export interface HerdrSnapshotResponse {
	id: string;
	result: HerdrSnapshotResult;
}

export type HerdrRawEvent =
	| { type: "workspace_created" | "workspace_updated"; workspace: HerdrRawWorkspace }
	| { type: "workspace_closed"; workspace_id: string }
	| { type: "workspace_focused"; workspace_id: string }
	| { type: "tab_created"; tab: HerdrRawTab }
	| { type: "tab_closed"; tab_id: string; workspace_id: string }
	| { type: "tab_focused"; tab_id: string; workspace_id: string }
	| { type: "pane_created" | "pane_updated"; pane: HerdrRawPane }
	| { type: "pane_closed"; pane_id: string; workspace_id: string }
	| { type: "pane_focused"; pane_id: string; workspace_id: string }
	| { type: "pane_output_changed"; pane_id: string; workspace_id: string }
	| { type: string };

export interface HerdrEventEnvelope {
	event: string;
	data: HerdrRawEvent;
}
