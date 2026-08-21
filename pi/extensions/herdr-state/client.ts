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
