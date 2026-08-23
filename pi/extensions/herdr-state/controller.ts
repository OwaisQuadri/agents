import type { HerdrClient } from "./client.ts";
import { applyEvent, createModel, findSelf, normalizeSnapshot } from "./engine.ts";
import type {
	HerdrSessionSnapshot,
	HerdrStateController as HerdrStateControllerContract,
	HerdrStateEvent,
	HerdrStateFailure,
	HerdrStateModel,
} from "./types.ts";

export type HerdrStateWait = (signal: AbortSignal) => Promise<void>;

interface ControllerRun {
	abortController: AbortController;
	cwd: string;
	paneId: string | undefined;
}

type WaitOutcome<T> =
	| { isStopped: true }
	| { isStopped: false; value: T };

function awaitUntilStopped<T>(
	signal: AbortSignal,
	promise: Promise<T>,
): Promise<WaitOutcome<T>> {
	if (signal.aborted) {
		return Promise.resolve({ isStopped: true });
	}
	return new Promise((resolve, reject) => {
		function stop(): void {
			resolve({ isStopped: true });
		}
		signal.addEventListener("abort", stop, { once: true });
		promise.then(
			(value) => {
				signal.removeEventListener("abort", stop);
				resolve({ isStopped: false, value });
			},
			(error: unknown) => {
				signal.removeEventListener("abort", stop);
				reject(error);
			},
		);
	});
}

function isFailure(
	result: HerdrStateEvent | HerdrStateFailure | object,
): result is HerdrStateFailure {
	return "code" in result;
}

function waitForReconnect(signal: AbortSignal): Promise<void> {
	if (signal.aborted) {
		return Promise.resolve();
	}
	return new Promise((resolve) => {
		const timeout = setTimeout(finish, 100);
		function finish(): void {
			clearTimeout(timeout);
			signal.removeEventListener("abort", finish);
			resolve();
		}
		signal.addEventListener("abort", finish, { once: true });
	});
}

/**
 * Maintains an immutable live Herdr state model from snapshots and events.
 *
 * @param client The read-only Herdr client used for snapshots and events.
 * @param wait The abort-aware delay used before retrying a failed read or dropped stream.
 * @returns A controller whose current model is null until start receives a valid snapshot.
 * @throws Never; client and normalization failures are retried until the controller stops.
 */
export class HerdrStateController implements HerdrStateControllerContract {
	private readonly client: HerdrClient;
	private readonly wait: HerdrStateWait;
	private activeRun: ControllerRun | null = null;
	private model: HerdrStateModel | null = null;

	constructor(client: HerdrClient, wait: HerdrStateWait = waitForReconnect) {
		this.client = client;
		this.wait = wait;
	}

	async start(cwd: string, paneId: string | undefined): Promise<void> {
		this.stop();
		const run: ControllerRun = {
			abortController: new AbortController(),
			cwd,
			paneId,
		};
		this.activeRun = run;
		this.model = null;

		const snapshot = await this.readSnapshot(run);
		if (snapshot === null || !this.isActive(run)) {
			return;
		}
		this.model = this.createCurrentModel(run, snapshot);
		void this.runEventLoop(run);
	}

	current(): HerdrStateModel | null {
		return this.model;
	}

	stop(): void {
		const run = this.activeRun;
		this.activeRun = null;
		run?.abortController.abort();
	}

	private isActive(run: ControllerRun): boolean {
		return this.activeRun === run && !run.abortController.signal.aborted;
	}

	private async pause(run: ControllerRun): Promise<boolean> {
		try {
			const outcome = await awaitUntilStopped(
				run.abortController.signal,
				this.wait(run.abortController.signal),
			);
			return !outcome.isStopped && this.isActive(run);
		} catch {
			return false;
		}
	}

	private async readSnapshot(run: ControllerRun): Promise<HerdrSessionSnapshot | null> {
		while (this.isActive(run)) {
			let response: Awaited<ReturnType<HerdrClient["snapshot"]>>;
			try {
				const outcome = await awaitUntilStopped(
					run.abortController.signal,
					this.client.snapshot(run.abortController.signal),
				);
				if (outcome.isStopped) {
					return null;
				}
				response = outcome.value;
			} catch {
				if (!this.isActive(run) || !(await this.pause(run))) {
					return null;
				}
				continue;
			}
			if (!this.isActive(run)) {
				return null;
			}
			if (!isFailure(response)) {
				try {
					return normalizeSnapshot(response);
				} catch {
					if (!this.isActive(run) || !(await this.pause(run))) {
						return null;
					}
					continue;
				}
			}
			if (!(await this.pause(run))) {
				return null;
			}
		}
		return null;
	}

	private createCurrentModel(
		run: ControllerRun,
		snapshot: HerdrSessionSnapshot,
	): HerdrStateModel {
		return createModel(snapshot, findSelf(snapshot, run.cwd, run.paneId));
	}

	private async recover(run: ControllerRun): Promise<boolean> {
		const snapshot = await this.readSnapshot(run);
		if (snapshot === null || !this.isActive(run)) {
			return false;
		}
		this.model = this.createCurrentModel(run, snapshot);
		return true;
	}

	private apply(run: ControllerRun, event: HerdrStateEvent): void {
		if (!this.isActive(run) || this.model === null) {
			return;
		}
		const model = applyEvent(this.model, event);
		this.model = {
			...model,
			self: findSelf(model.snapshot, run.cwd, run.paneId),
		};
	}

	private async consumeEventStream(run: ControllerRun): Promise<void> {
		// TODO(AGNT-0066.T15): Coalesce consecutive invalid-response recovery work.
		let recoveredInvalidResponse = false;
		for await (const result of this.client.events(run.abortController.signal)) {
			if (!this.isActive(run)) {
				return;
			}
			if (isFailure(result)) {
				if (result.code === "invalid-response" && recoveredInvalidResponse) {
					continue;
				}
				if (!(await this.recover(run))) {
					return;
				}
				recoveredInvalidResponse = result.code === "invalid-response";
				if (result.code === "unavailable") {
					return;
				}
				continue;
			}
			recoveredInvalidResponse = false;
			try {
				this.apply(run, result);
			} catch {
				if (!(await this.recover(run))) {
					return;
				}
			}
		}
	}

	private async runEventLoop(run: ControllerRun): Promise<void> {
		while (this.isActive(run)) {
			try {
				await this.consumeEventStream(run);
			} catch {
				if (!(await this.recover(run))) {
					return;
				}
			}
			if (!this.isActive(run) || !(await this.pause(run))) {
				return;
			}
		}
	}
}
