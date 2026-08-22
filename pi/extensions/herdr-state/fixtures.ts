import type {
	HerdrRawEvent,
	HerdrRawPane,
	HerdrRawSnapshot,
	HerdrRawTab,
	HerdrRawWorkspace,
	HerdrSnapshotResponse,
} from "./types.ts";

export const SELF_WORKSPACE_ID = "w5R";
export const SELF_TAB_ID = "w5R:t2";
export const SELF_PANE_ID = "w5R:p2";
export const SELF_CWD = "/Users/pi/workspaces/jerusalem";

export const OTHER_WORKSPACE_ID = "w5S";
export const OTHER_TAB_ID = "w5S:t2";
export const OTHER_PANE_ID = "w5S:p2";
export const OTHER_CWD = "/Users/pi/workspaces/edinburgh";

/**
 * Builds one raw Herdr workspace record, matching the `herdr api snapshot` schema.
 *
 * @param overrides Fields to override on the default fixture workspace.
 * @returns The raw workspace record.
 */
export function makeRawWorkspace(
	overrides: Partial<HerdrRawWorkspace> = {},
): HerdrRawWorkspace {
	return {
		workspace_id: SELF_WORKSPACE_ID,
		label: "jerusalem",
		focused: true,
		worktree: {
			checkout_path: SELF_CWD,
			is_linked_worktree: true,
			repo_name: "Pillars",
		},
		...overrides,
	};
}

/**
 * Builds one raw Herdr tab record, matching the `herdr api snapshot` schema.
 *
 * @param overrides Fields to override on the default fixture tab.
 * @returns The raw tab record.
 */
export function makeRawTab(overrides: Partial<HerdrRawTab> = {}): HerdrRawTab {
	return {
		tab_id: SELF_TAB_ID,
		workspace_id: SELF_WORKSPACE_ID,
		label: "2",
		focused: true,
		...overrides,
	};
}

/**
 * Builds one raw Herdr pane record, matching the `herdr api snapshot` schema.
 *
 * @param overrides Fields to override on the default fixture pane.
 * @returns The raw pane record.
 */
export function makeRawPane(overrides: Partial<HerdrRawPane> = {}): HerdrRawPane {
	return {
		pane_id: SELF_PANE_ID,
		workspace_id: SELF_WORKSPACE_ID,
		tab_id: SELF_TAB_ID,
		cwd: SELF_CWD,
		focused: true,
		...overrides,
	};
}

/**
 * Builds a raw Herdr session snapshot with two workspaces: one holding Pi's
 * own pane at `SELF_CWD`/`SELF_PANE_ID`, and one unrelated workspace.
 *
 * @param overrides Fields to override on the default fixture snapshot.
 * @returns The raw session snapshot.
 */
export function makeRawSnapshot(
	overrides: Partial<HerdrRawSnapshot> = {},
): HerdrRawSnapshot {
	return {
		focused_workspace_id: SELF_WORKSPACE_ID,
		focused_tab_id: SELF_TAB_ID,
		focused_pane_id: SELF_PANE_ID,
		workspaces: [
			makeRawWorkspace(),
			makeRawWorkspace({
				workspace_id: OTHER_WORKSPACE_ID,
				label: "edinburgh",
				focused: false,
				worktree: {
					checkout_path: OTHER_CWD,
					is_linked_worktree: true,
					repo_name: "Pillars",
				},
			}),
		],
		tabs: [
			makeRawTab(),
			makeRawTab({
				tab_id: OTHER_TAB_ID,
				workspace_id: OTHER_WORKSPACE_ID,
				focused: false,
			}),
		],
		panes: [
			makeRawPane(),
			makeRawPane({
				pane_id: OTHER_PANE_ID,
				workspace_id: OTHER_WORKSPACE_ID,
				tab_id: OTHER_TAB_ID,
				cwd: OTHER_CWD,
				focused: false,
			}),
		],
		agents: [],
		...overrides,
	};
}

/**
 * Builds a full `{ id, result: { type, snapshot } }` Herdr snapshot envelope.
 *
 * @param overrides Fields to override on the default fixture snapshot.
 * @returns The Herdr snapshot response.
 */
export function makeSnapshotResponse(
	overrides: Partial<HerdrRawSnapshot> = {},
): HerdrSnapshotResponse {
	return {
		id: "cli:api:snapshot",
		result: {
			type: "session_snapshot",
			snapshot: makeRawSnapshot(overrides),
		},
	};
}

/**
 * Builds a Herdr snapshot response missing the required envelope `id`.
 *
 * @returns A malformed value, typed loosely to simulate untrusted socket input.
 */
export function makeSnapshotResponseMissingId(): unknown {
	return {
		result: { type: "session_snapshot", snapshot: makeRawSnapshot() },
	};
}

/**
 * Builds a Herdr snapshot response missing `result.snapshot`.
 *
 * @returns A malformed value, typed loosely to simulate untrusted socket input.
 */
export function makeSnapshotResponseMissingSnapshot(): unknown {
	return { id: "cli:api:snapshot", result: { type: "session_snapshot" } };
}

/**
 * Builds a Herdr snapshot response with the wrong envelope `result.type`.
 *
 * @returns A malformed value, typed loosely to simulate untrusted socket input.
 */
export function makeSnapshotResponseWrongType(): unknown {
	return {
		id: "cli:api:snapshot",
		result: { type: "not_a_snapshot", snapshot: makeRawSnapshot() },
	};
}

/**
 * Builds a raw `workspace_updated` event carrying a full workspace record.
 *
 * @param overrides Fields to override on the default fixture workspace.
 * @returns The raw workspace-updated event.
 */
export function makeWorkspaceUpdatedEvent(
	overrides: Partial<HerdrRawWorkspace> = {},
): HerdrRawEvent {
	return {
		type: "workspace_updated",
		workspace: makeRawWorkspace({ label: "jerusalem (renamed)", ...overrides }),
	};
}

/**
 * Builds a raw `pane_updated` event carrying a full pane record.
 *
 * @param overrides Fields to override on the default fixture pane.
 * @returns The raw pane-updated event.
 */
export function makePaneUpdatedEvent(overrides: Partial<HerdrRawPane> = {}): HerdrRawEvent {
	return {
		type: "pane_updated",
		pane: makeRawPane({ cwd: `${SELF_CWD}/subdir`, ...overrides }),
	};
}

/**
 * Builds a raw event outside the recognized workspace, tab, and pane
 * create/update shapes, requiring a full snapshot replacement.
 *
 * @returns The unrecognized raw event.
 */
export function makeUnknownEvent(): HerdrRawEvent {
	return { type: "workspace_reordered" };
}

/**
 * Builds a raw `workspace_updated` event missing its required `workspace`
 * field, simulating a malformed recognized event from the socket.
 *
 * @returns A malformed value, typed loosely to simulate untrusted socket input.
 */
export function makeMalformedWorkspaceUpdatedEvent(): unknown {
	return { type: "workspace_updated" };
}
