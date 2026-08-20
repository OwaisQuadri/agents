export type DiffMode = "request" | "overall";

export type FileChangeKind =
	| "added"
	| "modified"
	| "deleted"
	| "renamed"
	| "binary";

export type ChangeOrigin = "committed" | "uncommitted" | "both";

export interface FileChange {
	path: string;
	renamedFrom: string | null;
	kind: FileChangeKind;
	additions: number;
	deletions: number;
	isBinary: boolean;
	origin: ChangeOrigin;
}

export interface DiffStats {
	files: FileChange[];
	additions: number;
	deletions: number;
	isTruncated: boolean;
}

export interface Snapshot {
	treeSha: string;
	baselineSha: string;
	captureTs: number;
}

export interface Hunk {
	header: string;
	lines: HunkLine[];
}

export interface HunkLine {
	origin: " " | "+" | "-";
	text: string;
}

export interface OverlayRow {
	change: FileChange;
	isFolded: boolean;
	hunks: Hunk[] | null;
}

export interface OverlayModel {
	mode: DiffMode;
	rows: OverlayRow[];
	cursor: number;
	isLoadingPatch: boolean;
}

export type OverlayKey =
	| "up"
	| "down"
	| "fold"
	| "mode-left"
	| "mode-right"
	| "open"
	| "close";

export type OverlayEffect =
	| { kind: "open-in-nvim"; path: string }
	| { kind: "load-patch"; path: string; mode: DiffMode }
	| { kind: "close" };

export interface OverlayStep {
	model: OverlayModel;
	effect: OverlayEffect | null;
}

export interface LiveDiffState {
	requestSnapshot: Snapshot | null;
	overallBaselineSha: string | null;
	requestStats: DiffStats | null;
	overallStats: DiffStats | null;
	refreshTimer: ReturnType<typeof setTimeout> | null;
	isRefreshing: boolean;
	watcher: WorktreeWatcher | null;
	watchTimer: ReturnType<typeof setTimeout> | null;
}

export type RowTone =
	| "header"
	| "path"
	| "added"
	| "removed"
	| "binary"
	| "hunkHeader"
	| "hunkAdd"
	| "hunkRemove"
	| "hunkContext"
	| "hint"
	| "truncation"
	| "originCommitted"
	| "originUncommitted";

export interface RenderSpan {
	text: string;
	tone: RowTone;
}

export interface RenderRow {
	spans: RenderSpan[];
	isSelected: boolean;
}

export interface WorktreeWatcher {
	close(): void;
}

export type WatcherFactory = (
	root: string,
	onChange: (relativePath: string) => void,
) => WorktreeWatcher | null;
