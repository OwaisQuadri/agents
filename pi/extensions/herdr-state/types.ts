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
