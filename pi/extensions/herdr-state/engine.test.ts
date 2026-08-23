import { test } from "node:test";
import assert from "node:assert/strict";

import {
	applyEvent,
	createModel,
	findSelf,
	normalizeEvent,
	normalizeSnapshot,
} from "./engine.ts";
import type { HerdrRawEvent, HerdrSnapshotResponse } from "./types.ts";
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
		workspaces: [
			{ workspace_id: "dup", label: "one", focused: false },
			{ workspace_id: "dup", label: "two", focused: false },
		],
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

// TODO(AGNT-0066.T16): Prove pane_created requests replacement and pane_updated stays incremental.
test("normalizeEvent throws for a malformed recognized event and returns null for unknown events", () => {
	assert.throws(() =>
		normalizeEvent(makeMalformedWorkspaceUpdatedEvent() as HerdrRawEvent),
	);
	assert.equal(normalizeEvent(makeUnknownEvent()), null);
});

test("createModel and applyEvent throw for malformed input", () => {
	assert.throws(() => createModel(null as never, null));
	assert.throws(() => createModel({} as never, null));

	const snapshot = normalizeSnapshot(makeSnapshotResponse());
	const model = createModel(snapshot, null);
	assert.throws(() => applyEvent(model, { type: "not-a-real-event" } as never));
});
