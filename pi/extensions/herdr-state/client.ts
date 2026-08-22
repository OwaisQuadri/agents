import { normalizeEvent } from "./engine.ts";
import type {
	HerdrPaneOutput,
	HerdrRawEvent,
	HerdrSnapshotResponse,
	HerdrStateEvent,
	HerdrStateFailure,
} from "./types.ts";

export interface HerdrClient {
	snapshot(signal?: AbortSignal): Promise<HerdrSnapshotResponse | HerdrStateFailure>;
	events(signal?: AbortSignal): AsyncIterable<HerdrStateEvent | HerdrStateFailure>;
	readPane(
		paneId: string,
		lineLimit: number,
	): Promise<HerdrPaneOutput | HerdrStateFailure>;
}

export interface HerdrCommandResult {
	code: number;
	stdout: string;
	stderr: string;
}

/**
 * Runs one read-only `herdr` command and reports its exit code and captured
 * output. The concrete binary path or socket connection is the caller's
 * concern; `HerdrCommandClient` never selects a write or input subcommand.
 *
 * @param args The read-only Herdr command line arguments, without the `herdr` binary name.
 * @param signal The optional cancellation signal for the command.
 * @returns The command's exit code, standard output, and standard error.
 * @throws Error when the command cannot be started.
 */
export type HerdrCommandRunner = (
	args: string[],
	signal?: AbortSignal,
) => Promise<HerdrCommandResult>;

/**
 * Opens the Herdr live event subscription (over its local socket or an
 * equivalent long-running read-only command) and yields one raw JSON line
 * per received event, until the connection ends or fails.
 *
 * @param signal The optional cancellation signal for the subscription.
 * @returns An asynchronous iterable of raw JSON event lines.
 * @throws Error when the subscription cannot be started or the connection fails.
 */
export type HerdrEventSubscriber = (signal?: AbortSignal) => AsyncIterable<string>;

export interface HerdrTransport {
	runCommand: HerdrCommandRunner;
	subscribeEvents: HerdrEventSubscriber;
}

function errorMessage(error: unknown): string {
	return error instanceof Error ? error.message : String(error);
}

function unavailable(message: string): HerdrStateFailure {
	return { code: "unavailable", message };
}

function notFound(message: string): HerdrStateFailure {
	return { code: "not-found", message };
}

function invalidResponse(message: string): HerdrStateFailure {
	return { code: "invalid-response", message };
}

function invalidEventResponse(message: string): HerdrStateFailure {
	return invalidResponse(`${message}; replace the snapshot`);
}

function isObject(value: unknown): value is Record<string, unknown> {
	return value !== null && typeof value === "object" && !Array.isArray(value);
}

function isSubscriptionAcknowledgement(value: unknown): boolean {
	if (!isObject(value) || Object.keys(value).length !== 2) {
		return false;
	}
	const result = value.result;
	return (
		value.id === "pi-herdr-state-events" &&
		isObject(result) &&
		Object.keys(result).length === 1 &&
		result.type === "subscription_started"
	);
}

function isValidSnapshotEnvelope(value: unknown): value is HerdrSnapshotResponse {
	if (value === null || typeof value !== "object") {
		return false;
	}
	const envelope = value as { id?: unknown; result?: unknown };
	if (typeof envelope.id !== "string" || envelope.id === "") {
		return false;
	}
	if (envelope.result === null || typeof envelope.result !== "object") {
		return false;
	}
	const result = envelope.result as { type?: unknown; snapshot?: unknown };
	if (result.type !== "session_snapshot") {
		return false;
	}
	return result.snapshot !== null && typeof result.snapshot === "object";
}

function parseCommandError(stderr: string): { code: string; message: string } | null {
	try {
		const parsed = JSON.parse(stderr) as { error?: { code?: unknown; message?: unknown } };
		if (
			typeof parsed.error?.code === "string" &&
			typeof parsed.error?.message === "string"
		) {
			return { code: parsed.error.code, message: parsed.error.message };
		}
	} catch {
		// stderr was not a JSON error envelope; fall through to the generic failure below.
	}
	return null;
}

function splitLines(text: string): string[] {
	const withoutTrailingNewline = text.replace(/\n$/, "");
	return withoutTrailingNewline === "" ? [] : withoutTrailingNewline.split("\n");
}

/**
 * Read-only Herdr client backed by an injected command runner and event
 * subscriber. It reads the live session snapshot, subscribes to normalized
 * state events, and reads bounded recent pane output. It never runs a
 * Herdr write or input command.
 */
export class HerdrCommandClient implements HerdrClient {
	private readonly transport: HerdrTransport;

	constructor(transport: HerdrTransport) {
		this.transport = transport;
	}

	/**
	 * Reads the live Herdr session snapshot envelope.
	 *
	 * @returns The `{ id, result: { type: "session_snapshot", snapshot } }` envelope, or a classified failure.
	 * @throws The abort reason when the supplied signal is aborted; other failures are returned.
	 */
	async snapshot(signal?: AbortSignal): Promise<HerdrSnapshotResponse | HerdrStateFailure> {
		signal?.throwIfAborted();
		let result: HerdrCommandResult;
		try {
			result = await this.transport.runCommand(["api", "snapshot"], signal);
		} catch (error) {
			if (signal?.aborted) {
				signal.throwIfAborted();
			}
			return unavailable(`Herdr snapshot command failed to run: ${errorMessage(error)}`);
		}
		signal?.throwIfAborted();
		if (result.code !== 0) {
			return unavailable(
				`Herdr snapshot command exited with code ${result.code}: ${result.stderr.trim()}`,
			);
		}
		let parsed: unknown;
		try {
			parsed = JSON.parse(result.stdout);
		} catch (error) {
			return invalidResponse(
				`Herdr snapshot output is not valid JSON: ${errorMessage(error)}`,
			);
		}
		if (!isValidSnapshotEnvelope(parsed)) {
			return invalidResponse(
				"Herdr snapshot response is missing the required id, result.type, or result.snapshot fields",
			);
		}
		return parsed;
	}

	/**
	 * Subscribes to normalized Herdr state events.
	 *
	 * @returns An asynchronous iterable of normalized events, or classified failures, until the connection ends.
	 * @throws Never; failures are yielded, and intentional abort ends the stream.
	 */
	async *events(signal?: AbortSignal): AsyncIterable<HerdrStateEvent | HerdrStateFailure> {
		if (signal?.aborted) {
			return;
		}
		let lines: AsyncIterable<string>;
		try {
			lines = this.transport.subscribeEvents(signal);
		} catch (error) {
			if (signal?.aborted) {
				return;
			}
			yield unavailable(`Herdr event subscription failed to start: ${errorMessage(error)}`);
			return;
		}
		let isAcknowledged = false;
		try {
			for await (const line of lines) {
				let raw: unknown;
				try {
					raw = JSON.parse(line);
				} catch (error) {
					yield invalidEventResponse(
						isAcknowledged
							? `Herdr event line is not valid JSON: ${errorMessage(error)}`
							: `Herdr subscription acknowledgement is not valid JSON: ${errorMessage(error)}`,
					);
					if (!isAcknowledged) {
						return;
					}
					continue;
				}
				if (!isAcknowledged) {
					if (!isSubscriptionAcknowledgement(raw)) {
						yield invalidEventResponse(
							"Herdr subscription acknowledgement does not match pi-herdr-state-events subscription_started",
						);
						return;
					}
					isAcknowledged = true;
					continue;
				}
				if (!isObject(raw)) {
					yield invalidEventResponse("Herdr event line did not decode to an object");
					continue;
				}
				const event = raw.event;
				if (
					event === "pane.output_matched" ||
					event === "pane.agent_status_changed" ||
					event === "pane.scroll_changed"
				) {
					yield invalidEventResponse(`Herdr specialized event ${event} is not accepted`);
					continue;
				}
				const data = raw.data;
				if (typeof event !== "string" || event === "" || !isObject(data)) {
					yield invalidEventResponse("Herdr event envelope is malformed");
					continue;
				}
				if (typeof data.type !== "string" || data.type === "") {
					yield invalidEventResponse("Herdr event data is malformed");
					continue;
				}
				if (event !== data.type) {
					yield invalidEventResponse(
						`Herdr event envelope ${event} does not match data.type ${data.type}`,
					);
					continue;
				}
				let normalized: HerdrStateEvent | null;
				try {
					normalized = normalizeEvent(data as HerdrRawEvent);
				} catch (error) {
					yield invalidEventResponse(`Herdr event is malformed: ${errorMessage(error)}`);
					continue;
				}
				if (normalized === null) {
					yield invalidEventResponse("Herdr event type is unknown");
					continue;
				}
				yield normalized;
			}
			if (!isAcknowledged && !signal?.aborted) {
				yield invalidEventResponse("Herdr subscription ended before its acknowledgement");
			}
		} catch (error) {
			if (signal?.aborted) {
				return;
			}
			yield unavailable(`Herdr event subscription ended: ${errorMessage(error)}`);
		}
	}

	/**
	 * Reads bounded recent output from one Herdr pane.
	 *
	 * @param paneId The Herdr pane identifier to read.
	 * @param lineLimit The positive maximum number of recent lines to return.
	 * @returns The bounded recent pane output, or a classified failure.
	 * @throws Never; failures are returned, not thrown.
	 */
	async readPane(
		paneId: string,
		lineLimit: number,
	): Promise<HerdrPaneOutput | HerdrStateFailure> {
		if (typeof paneId !== "string" || paneId === "") {
			return invalidResponse("Herdr pane read requires a non-empty pane identifier");
		}
		if (!Number.isInteger(lineLimit) || lineLimit <= 0) {
			return invalidResponse("Herdr pane read requires a positive bounded line limit");
		}
		let result: HerdrCommandResult;
		try {
			// Requesting one extra line over the limit turns "the source has more
			// than lineLimit lines" into an observable signal: herdr's `--lines`
			// caps output silently and reports no truncation flag of its own.
			result = await this.transport.runCommand([
				"pane",
				"read",
				paneId,
				"--source",
				"recent",
				"--lines",
				String(lineLimit + 1),
			]);
		} catch (error) {
			return unavailable(`Herdr pane read command failed to run: ${errorMessage(error)}`);
		}
		if (result.code !== 0) {
			const parsedError = parseCommandError(result.stderr);
			if (parsedError?.code === "pane_not_found") {
				return notFound(`Herdr pane ${paneId} was not found: ${parsedError.message}`);
			}
			return unavailable(
				`Herdr pane read command exited with code ${result.code}: ${result.stderr.trim()}`,
			);
		}
		const lines = splitLines(result.stdout);
		const isTruncated = lines.length > lineLimit;
		const boundedLines = isTruncated ? lines.slice(lines.length - lineLimit) : lines;
		return { paneId, text: boundedLines.join("\n"), isTruncated };
	}
}
