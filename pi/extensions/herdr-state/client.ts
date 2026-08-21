// TODO(AGNT-0066.T01): Implement only read-only snapshot, event, and pane-output access.
import type {
	HerdrPaneOutput,
	HerdrSnapshotResponse,
	HerdrStateEvent,
	HerdrStateFailure,
} from "./types.ts";

export interface HerdrClient {
	snapshot(): Promise<HerdrSnapshotResponse | HerdrStateFailure>;
	events(): AsyncIterable<HerdrStateEvent | HerdrStateFailure>;
	readPane(
		paneId: string,
		lineLimit: number,
	): Promise<HerdrPaneOutput | HerdrStateFailure>;
}
