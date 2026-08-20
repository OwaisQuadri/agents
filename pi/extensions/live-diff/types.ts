export type DiffMode = "request" | "overall";

export type FileChangeKind =
	| "added"
	| "modified"
	| "deleted"
	| "renamed"
	| "binary";

export interface FileChange {
	path: string;
	renamedFrom: string | null;
	kind: FileChangeKind;
	additions: number;
	deletions: number;
	isBinary: boolean;
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
	| "toggle-mode"
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
}
