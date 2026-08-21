import type {
	HerdrAgentLocation,
	HerdrRawEvent,
	HerdrSessionSnapshot,
	HerdrSnapshotResponse,
	HerdrStateEvent,
	HerdrStateModel,
} from "./types.ts";

/**
 * Normalizes a Herdr socket response into the read-only session model.
 *
 * @param response The Herdr snapshot response to normalize.
 * @returns The normalized session snapshot.
 * @throws Error when the response cannot be normalized.
 */
export function normalizeSnapshot(
	_response: HerdrSnapshotResponse,
): HerdrSessionSnapshot {
	throw new Error("unimplemented");
}

/**
 * Normalizes one Herdr subscription event for the read-only state model.
 *
 * @param event The raw Herdr event to normalize.
 * @returns A state event, or null when a full snapshot is required.
 * @throws Error when a recognized event is malformed.
 */
export function normalizeEvent(
	_event: HerdrRawEvent,
): HerdrStateEvent | null {
	throw new Error("unimplemented");
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
	_snapshot: HerdrSessionSnapshot,
	_cwd: string,
	_paneId: string | undefined,
): HerdrAgentLocation | null {
	throw new Error("unimplemented");
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
	_snapshot: HerdrSessionSnapshot,
	_self: HerdrAgentLocation | null,
): HerdrStateModel {
	throw new Error("unimplemented");
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
	_model: HerdrStateModel,
	_event: HerdrStateEvent,
): HerdrStateModel {
	throw new Error("unimplemented");
}
