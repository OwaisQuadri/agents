import { test } from "node:test";
import assert from "node:assert/strict";

import {
	applyEvent,
	createModel,
	findSelf,
	normalizeEvent,
	normalizeSnapshot,
} from "./engine.ts";
import type {
	HerdrRawEvent,
	HerdrSnapshotResponse,
	HerdrStateEvent,
} from "./types.ts";
import {
	makeMalformedWorkspaceUpdatedEvent,
	makePaneUpdatedEvent,
	makeSnapshotResponse,
	makeSnapshotResponseMissingId,
	makeSnapshotResponseMissingSnapshot,
	makeSnapshotResponseWithAmbiguousSelfCwd,
	makeSnapshotResponseWithMalformedFocused,
	makeSnapshotResponseWrongType,
	makeUnknownEvent,
	makeWorkspaceUpdatedEvent,
	OTHER_CWD,
	OTHER_PANE_ID,
	OTHER_TAB_ID,
	OTHER_WORKSPACE_ID,
	SELF_CWD,
	SELF_PANE_ID,
	SELF_TAB_ID,
	SELF_WORKSPACE_ID,
} from "./fixtures.ts";

test("TC-24 normalizeSnapshot rejects duplicate workspace identities", () => {
	const response = makeSnapshotResponse({
		focused_workspace_id: null,
		focused_tab_id: null,
		focused_pane_id: null,
		workspaces: [
			{ workspace_id: "dup", label: "one", focused: false },
			{ workspace_id: "dup", label: "two", focused: false },
		],
		tabs: [],
		panes: [],
	});

	assert.throws(() => normalizeSnapshot(response), {
		message: "invalid Herdr snapshot response: duplicate workspace_id",
	});

	response.result.snapshot.workspaces[1]!.workspace_id = "other";
	const snapshot = normalizeSnapshot(response);

	assert.deepEqual(
		snapshot.workspaces.map(({ id, label, isFocused }) => ({ id, label, isFocused })),
		[
			{ id: "dup", label: "one", isFocused: false },
			{ id: "other", label: "two", isFocused: false },
		],
	);
});

const crossRecordVariants: Array<{
	name: string;
	mutate: (response: HerdrSnapshotResponse) => void;
	message: string;
}> = [
	{
		name: "duplicate workspace_id",
		mutate: (response) => {
			response.result.snapshot.workspaces[1]!.workspace_id = SELF_WORKSPACE_ID;
		},
		message: "invalid Herdr snapshot response: duplicate workspace_id",
	},
	{
		name: "duplicate tab_id",
		mutate: (response) => {
			response.result.snapshot.tabs[1]!.tab_id = SELF_TAB_ID;
		},
		message: "invalid Herdr snapshot response: duplicate tab_id",
	},
	{
		name: "duplicate pane_id",
		mutate: (response) => {
			response.result.snapshot.panes[1]!.pane_id = SELF_PANE_ID;
		},
		message: "invalid Herdr snapshot response: duplicate pane_id",
	},
	{
		name: "tab with a missing workspace",
		mutate: (response) => {
			response.result.snapshot.tabs[0]!.workspace_id = "missing-workspace";
		},
		message: "invalid Herdr snapshot response: tab references unknown workspace_id",
	},
	{
		name: "pane with a missing workspace",
		mutate: (response) => {
			response.result.snapshot.panes[0]!.workspace_id = "missing-workspace";
		},
		message: "invalid Herdr snapshot response: pane references unknown workspace_id",
	},
	{
		name: "pane with a missing tab",
		mutate: (response) => {
			response.result.snapshot.panes[0]!.tab_id = "missing-tab";
		},
		message: "invalid Herdr snapshot response: pane references unknown tab_id",
	},
	{
		name: "pane and tab from different workspaces",
		mutate: (response) => {
			response.result.snapshot.panes[0]!.tab_id = OTHER_TAB_ID;
		},
		message: "invalid Herdr snapshot response: pane and tab workspace_id differ",
	},
	{
		name: "missing focused_workspace_id",
		mutate: (response) => {
			response.result.snapshot.focused_workspace_id = "missing-workspace";
		},
		message: "invalid Herdr snapshot response: focused_workspace_id not found",
	},
	{
		name: "missing focused_tab_id",
		mutate: (response) => {
			response.result.snapshot.focused_tab_id = "missing-tab";
		},
		message: "invalid Herdr snapshot response: focused_tab_id not found",
	},
	{
		name: "missing focused_pane_id",
		mutate: (response) => {
			response.result.snapshot.focused_pane_id = "missing-pane";
		},
		message: "invalid Herdr snapshot response: focused_pane_id not found",
	},
	{
		name: "focused IDs from different workspaces",
		mutate: (response) => {
			response.result.snapshot.focused_tab_id = OTHER_TAB_ID;
		},
		message: "invalid Herdr snapshot response: focused IDs do not form one lineage",
	},
];

for (const { name, mutate, message } of crossRecordVariants) {
	test(`TC-28 normalizeSnapshot rejects ${name}`, () => {
		const response = makeSnapshotResponse();
		mutate(response);

		assert.throws(() => normalizeSnapshot(response), { message });
	});
}

test("TC-28 normalizeSnapshot leaves a valid snapshot unchanged", () => {
	assert.deepEqual(normalizeSnapshot(makeSnapshotResponse()), {
		workspaces: [
			{
				id: SELF_WORKSPACE_ID,
				label: "jerusalem",
				worktree: { path: SELF_CWD, branch: null },
				isFocused: true,
			},
			{
				id: OTHER_WORKSPACE_ID,
				label: "edinburgh",
				worktree: { path: OTHER_CWD, branch: null },
				isFocused: false,
			},
		],
		tabs: [
			{
				id: SELF_TAB_ID,
				workspaceId: SELF_WORKSPACE_ID,
				label: "2",
				isFocused: true,
			},
			{
				id: OTHER_TAB_ID,
				workspaceId: OTHER_WORKSPACE_ID,
				label: "2",
				isFocused: false,
			},
		],
		panes: [
			{
				id: SELF_PANE_ID,
				workspaceId: SELF_WORKSPACE_ID,
				tabId: SELF_TAB_ID,
				label: SELF_PANE_ID,
				cwd: SELF_CWD,
				isFocused: true,
			},
			{
				id: OTHER_PANE_ID,
				workspaceId: OTHER_WORKSPACE_ID,
				tabId: OTHER_TAB_ID,
				label: OTHER_PANE_ID,
				cwd: OTHER_CWD,
				isFocused: false,
			},
		],
		focusedWorkspaceId: SELF_WORKSPACE_ID,
		focusedTabId: SELF_TAB_ID,
		focusedPaneId: SELF_PANE_ID,
	});
});

test("TC-01 normalizeSnapshot lists every workspace with its worktree and focus", () => {
	const snapshot = normalizeSnapshot(makeSnapshotResponse());

	assert.equal(snapshot.workspaces.length, 2);
	const byId = new Map(snapshot.workspaces.map((workspace) => [workspace.id, workspace]));
	assert.equal(byId.get(SELF_WORKSPACE_ID)?.label, "jerusalem");
	assert.equal(byId.get(SELF_WORKSPACE_ID)?.worktree?.path, SELF_CWD);
	assert.equal(byId.get(SELF_WORKSPACE_ID)?.isFocused, true);
	assert.equal(byId.get(OTHER_WORKSPACE_ID)?.label, "edinburgh");
	assert.equal(byId.get(OTHER_WORKSPACE_ID)?.isFocused, false);
	assert.equal(snapshot.focusedWorkspaceId, SELF_WORKSPACE_ID);
	assert.equal(snapshot.tabs.length, 2);
	assert.equal(snapshot.panes.length, 2);
});

const malformedFocusedValues = [
	{ name: "string", focused: "true" },
	{ name: "number", focused: 1 },
	{ name: "null", focused: null },
	{ name: "missing", focused: undefined },
] as const;

for (const resourceType of ["workspace", "tab", "pane"] as const) {
	for (const { name, focused } of malformedFocusedValues) {
		test(`normalizeSnapshot rejects a ${name} ${resourceType} focused field`, () => {
			const response = makeSnapshotResponseWithMalformedFocused(resourceType, focused);

			assert.throws(
				() => normalizeSnapshot(response as HerdrSnapshotResponse),
				/focused must be boolean/,
			);
		});
	}
}

test("TC-01 findSelf locates Pi by its injected pane identifier", () => {
	const snapshot = normalizeSnapshot(makeSnapshotResponse());

	const self = findSelf(snapshot, SELF_CWD, SELF_PANE_ID);

	assert.deepEqual(self, {
		workspaceId: SELF_WORKSPACE_ID,
		tabId: SELF_TAB_ID,
		paneId: SELF_PANE_ID,
		isSelf: true,
	});
});

test("findSelf returns null for a stale pane identifier despite one directory match", () => {
	const snapshot = normalizeSnapshot(makeSnapshotResponse());

	assert.equal(findSelf(snapshot, SELF_CWD, "stale-pane"), null);
});

test("findSelf returns the unique directory match with its exact identity", () => {
	const snapshot = normalizeSnapshot(makeSnapshotResponse());

	const self = findSelf(snapshot, OTHER_CWD, undefined);

	assert.deepEqual(self, {
		workspaceId: OTHER_WORKSPACE_ID,
		tabId: OTHER_TAB_ID,
		paneId: OTHER_PANE_ID,
		isSelf: true,
	});
});

test("findSelf returns null for zero directory matches", () => {
	const snapshot = normalizeSnapshot(makeSnapshotResponse());

	assert.equal(findSelf(snapshot, "/no/such/directory", undefined), null);
});

test("findSelf returns null for two directory matches", () => {
	const snapshot = normalizeSnapshot(makeSnapshotResponseWithAmbiguousSelfCwd());

	assert.equal(findSelf(snapshot, SELF_CWD, undefined), null);
});

test("TC-04 createModel handles an absent Pi location", () => {
	const snapshot = normalizeSnapshot(makeSnapshotResponse());

	const model = createModel(snapshot, null);
	assert.equal(model.self, null);
	assert.equal(model.snapshot.workspaces.length, 2);
});

test("TC-01 createModel and applyEvent return new immutable models", () => {
	const snapshot = normalizeSnapshot(makeSnapshotResponse());
	const self = findSelf(snapshot, SELF_CWD, SELF_PANE_ID);
	const model = createModel(snapshot, self);

	const event = normalizeEvent(makeWorkspaceUpdatedEvent());
	assert.notEqual(event, null);
	const nextModel = applyEvent(model, event!);

	assert.notEqual(nextModel, model);
	assert.notEqual(nextModel.snapshot, model.snapshot);
	assert.equal(
		model.snapshot.workspaces.find((workspace) => workspace.id === SELF_WORKSPACE_ID)?.label,
		"jerusalem",
		"the original model is untouched",
	);
	assert.equal(
		nextModel.snapshot.workspaces.find((workspace) => workspace.id === SELF_WORKSPACE_ID)
			?.label,
		"jerusalem (renamed)",
	);
});

test("known tab and pane events update the model in place", () => {
	const snapshot = normalizeSnapshot(makeSnapshotResponse());
	const model = createModel(snapshot, findSelf(snapshot, SELF_CWD, SELF_PANE_ID));

	const paneEvent = normalizeEvent(makePaneUpdatedEvent());
	assert.notEqual(paneEvent, null);
	const nextModel = applyEvent(model, paneEvent!);

	const updatedPane = nextModel.snapshot.panes.find((pane) => pane.id === SELF_PANE_ID);
	assert.equal(updatedPane?.cwd, `${SELF_CWD}/subdir`);
	assert.equal(nextModel.snapshot.panes.length, 2, "the update replaces, not appends");
});

test("TC-05 an unknown event is ignored and a replacement snapshot becomes the displayed state", () => {
	const firstSnapshot = normalizeSnapshot(makeSnapshotResponse());
	const model = createModel(firstSnapshot, findSelf(firstSnapshot, SELF_CWD, SELF_PANE_ID));

	assert.equal(normalizeEvent(makeUnknownEvent()), null);

	const replacementResponse = makeSnapshotResponse({
		focused_tab_id: null,
		focused_pane_id: null,
		workspaces: [
			{
				workspace_id: SELF_WORKSPACE_ID,
				label: "jerusalem after reconnect",
				focused: true,
			},
		],
		tabs: [],
		panes: [],
	});
	const replacementSnapshot = normalizeSnapshot(replacementResponse);
	const recovered = applyEvent(model, {
		type: "snapshot-replaced",
		snapshot: replacementSnapshot,
	});

	assert.equal(model.snapshot.workspaces.length, 2, "the unknown event left the prior model intact");
	assert.equal(recovered.snapshot.workspaces.length, 1);
	assert.equal(recovered.snapshot.workspaces[0]?.label, "jerusalem after reconnect");
	assert.equal(
		recovered.self,
		null,
		"self is cleared once its pane is absent from the replacement snapshot",
	);
});

test("TC-06 normalizeSnapshot throws for a malformed envelope", () => {
	assert.throws(() =>
		normalizeSnapshot(makeSnapshotResponseMissingId() as HerdrSnapshotResponse),
	);
	assert.throws(() =>
		normalizeSnapshot(makeSnapshotResponseMissingSnapshot() as HerdrSnapshotResponse),
	);
	assert.throws(() =>
		normalizeSnapshot(makeSnapshotResponseWrongType() as HerdrSnapshotResponse),
	);
});

test("TC-27 normalizeEvent routes an observed pane_created event to snapshot replacement", () => {
	const event: HerdrRawEvent = {
		type: "pane_created",
		pane: {
			pane_id: "p",
			workspace_id: "missing-w",
			tab_id: "missing-t",
			focused: false,
		},
	};

	assert.equal(normalizeEvent(event), null);
});

test("TC-27 normalizeEvent preserves an observed pane_updated event", () => {
	const event: HerdrRawEvent = {
		type: "pane_updated",
		pane: {
			pane_id: "w5R:p2",
			workspace_id: "w5R",
			tab_id: "w5R:t2",
			focused: false,
		},
	};

	assert.deepEqual(normalizeEvent(event), {
		type: "pane-changed",
		pane: {
			id: "w5R:p2",
			workspaceId: "w5R",
			tabId: "w5R:t2",
			label: "w5R:p2",
			cwd: null,
			isFocused: false,
		},
	});
});

test("normalizeEvent throws for a malformed recognized event and returns null for unknown events", () => {
	assert.throws(() =>
		normalizeEvent(makeMalformedWorkspaceUpdatedEvent() as HerdrRawEvent),
	);
	assert.equal(normalizeEvent(makeUnknownEvent()), null);
});

test("TC-30 tab updates preserve parent lineage and model immutability", () => {
	const snapshot = normalizeSnapshot(makeSnapshotResponse());
	const model = createModel(snapshot, findSelf(snapshot, SELF_CWD, SELF_PANE_ID));
	const priorTab = model.snapshot.tabs.find((tab) => tab.id === SELF_TAB_ID);
	const expectedPriorTab = {
		id: SELF_TAB_ID,
		workspaceId: SELF_WORKSPACE_ID,
		label: "2",
		isFocused: true,
	};

	assert.throws(
		() =>
			applyEvent(model, {
				type: "tab-changed",
				tab: {
					id: SELF_TAB_ID,
					workspaceId: "missing-workspace",
					label: "invalid",
					isFocused: false,
				},
			}),
		{
			message: "invalid Herdr tab event: tab references unknown workspace_id",
		},
	);

	const validTab = { ...expectedPriorTab, label: "renamed" };
	const nextModel = applyEvent(model, { type: "tab-changed", tab: validTab });

	assert.notEqual(nextModel, model);
	assert.notEqual(nextModel.snapshot, model.snapshot);
	assert.deepEqual(
		nextModel.snapshot.tabs.find((tab) => tab.id === SELF_TAB_ID),
		validTab,
	);
	assert.deepEqual(
		model.snapshot.tabs.find((tab) => tab.id === SELF_TAB_ID),
		expectedPriorTab,
	);
	assert.deepEqual(priorTab, expectedPriorTab);
});

test("TC-30 pane updates preserve parent lineage and model immutability", () => {
	const snapshot = normalizeSnapshot(makeSnapshotResponse());
	const model = createModel(snapshot, findSelf(snapshot, SELF_CWD, SELF_PANE_ID));
	const priorPane = model.snapshot.panes.find((pane) => pane.id === SELF_PANE_ID);
	const expectedPriorPane = {
		id: SELF_PANE_ID,
		workspaceId: SELF_WORKSPACE_ID,
		tabId: SELF_TAB_ID,
		label: SELF_PANE_ID,
		cwd: SELF_CWD,
		isFocused: true,
	};
	const invalidCases: Array<{ event: HerdrStateEvent; message: string }> = [
		{
			event: {
				type: "pane-changed",
				pane: {
					id: "p-missing-w",
					workspaceId: "missing-w",
					tabId: SELF_TAB_ID,
					label: "p-missing-w",
					cwd: null,
					isFocused: false,
				},
			},
			message: "invalid Herdr pane event: pane references unknown workspace_id",
		},
		{
			event: {
				type: "pane-changed",
				pane: {
					id: "p-missing-t",
					workspaceId: SELF_WORKSPACE_ID,
					tabId: "missing-t",
					label: "p-missing-t",
					cwd: null,
					isFocused: false,
				},
			},
			message: "invalid Herdr pane event: pane references unknown tab_id",
		},
		{
			event: {
				type: "pane-changed",
				pane: {
					id: "p-cross",
					workspaceId: SELF_WORKSPACE_ID,
					tabId: OTHER_TAB_ID,
					label: "p-cross",
					cwd: null,
					isFocused: false,
				},
			},
			message: "invalid Herdr pane event: pane and tab workspace_id differ",
		},
	];

	for (const { event, message } of invalidCases) {
		assert.throws(() => applyEvent(model, event), { message });
	}

	const validPane = { ...expectedPriorPane, isFocused: false };
	const nextModel = applyEvent(model, { type: "pane-changed", pane: validPane });

	assert.notEqual(nextModel, model);
	assert.notEqual(nextModel.snapshot, model.snapshot);
	assert.deepEqual(
		nextModel.snapshot.panes.find((pane) => pane.id === SELF_PANE_ID),
		validPane,
	);
	assert.deepEqual(
		model.snapshot.panes.find((pane) => pane.id === SELF_PANE_ID),
		expectedPriorPane,
	);
	assert.deepEqual(priorPane, expectedPriorPane);
});

test("createModel and applyEvent throw for malformed input", () => {
	assert.throws(() => createModel(null as never, null));
	assert.throws(() => createModel({} as never, null));

	const snapshot = normalizeSnapshot(makeSnapshotResponse());
	const model = createModel(snapshot, null);
	assert.throws(() => applyEvent(model, { type: "not-a-real-event" } as never));
});
