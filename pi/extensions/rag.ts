import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

type SearchMemoryInput = { query: string; k?: number; source_filter?: string };
type SpawnProcess = (command: string, args: string[]) => ChildProcessWithoutNullStreams;
type McpTextContent = { type: "text"; text: string };
type Timeouts = { startupMs: number; requestMs: number };
type Pending = { resolve(value: unknown): void; reject(error: Error): void; cleanup(): void };

const protocolVersion = "2025-11-25";
const defaultTimeouts: Timeouts = { startupMs: 10_000, requestMs: 30_000 };
const maximumStderrLength = 1024;
const maximumUnframedStdoutLength = 8 * 1024 * 1024;

const searchMemoryParameters = {
	type: "object",
	additionalProperties: false,
	required: ["query"],
	properties: {
		query: { type: "string", description: "Search query for the personal knowledge base" },
		k: { type: "integer", minimum: 1, default: 8, description: "Maximum number of results" },
		source_filter: { type: "string", description: "Configured source name to search" },
	},
} as const;

function isRecord(value: unknown): value is Record<string, unknown> {
	return value !== null && typeof value === "object" && !Array.isArray(value);
}

function isTextContent(value: unknown): value is McpTextContent {
	return isRecord(value) && value.type === "text" && typeof value.text === "string";
}

function serverError(stderr: string): Error {
	const diagnostics = stderr.trim();
	return new Error(diagnostics ? `rag server exited: ${diagnostics}` : "rag server exited");
}

class McpSession {
	private readonly pending = new Map<number, Pending>();
	private nextId = 1;
	private stderr = "";
	private stdout = "";
	private isClosed = false;
	private readonly child: ChildProcessWithoutNullStreams;
	private readonly onClosed: () => void;
	private readonly timeouts: Timeouts;

	constructor(spawnProcess: SpawnProcess, onClosed: () => void, timeouts: Timeouts) {
		this.onClosed = onClosed;
		this.timeouts = timeouts;
		this.child = spawnProcess("rag", ["serve"]);
		this.child.stdout.setEncoding("utf8");
		this.child.stderr.setEncoding("utf8");
		this.child.stdout.on("data", this.handleStdout);
		this.child.stdout.on("error", this.handleStdoutError);
		this.child.stderr.on("data", this.handleStderr);
		this.child.stderr.on("error", this.handleStderrError);
		this.child.stdin.on("error", this.handleStdinError);
		this.child.on("error", this.handleChildError);
		this.child.on("exit", this.handleChildExit);
	}

	get isAvailable(): boolean {
		return !this.isClosed;
	}

	async initialize(): Promise<void> {
		const result = await this.request(
			"initialize",
			{ protocolVersion, capabilities: {}, clientInfo: { name: "pi-rag", version: "1" } },
			undefined,
			this.timeouts.startupMs,
			"rag startup timed out",
		);
		if (!isRecord(result) || result.protocolVersion !== protocolVersion) {
			throw new Error("rag server returned an incompatible protocol version");
		}
		this.notify("notifications/initialized");
		const tools = await this.request("tools/list", undefined, undefined, this.timeouts.startupMs, "rag startup timed out");
		if (!isRecord(tools) || !Array.isArray(tools.tools) || !tools.tools.some((tool) => isRecord(tool) && tool.name === "search_memory")) {
			throw new Error("rag server does not provide search_memory");
		}
	}

	async callSearch(params: SearchMemoryInput, signal?: AbortSignal): Promise<{ content: McpTextContent[]; details: { hits: Record<string, unknown>[] } }> {
		const result = await this.request(
			"tools/call",
			{
				name: "search_memory",
				arguments: {
					query: params.query,
					k: params.k ?? 8,
					...(params.source_filter === undefined ? {} : { source_filter: params.source_filter }),
				},
			},
			signal,
			this.timeouts.requestMs,
			"rag request timed out",
		);
		return mapSearchResult(result);
	}

	async close(): Promise<void> {
		if (!this.isClosed) {
			this.stop(new Error("rag server stopped"));
		}
	}

	private request(method: string, params: Record<string, unknown> | undefined, signal: AbortSignal | undefined, timeoutMs: number, timeoutMessage: string): Promise<unknown> {
		if (this.isClosed) {
			return Promise.reject(new Error("rag server is not available"));
		}
		const id = this.nextId++;
		return new Promise((resolve, reject) => {
			const abort = () => rejectRequest(new Error("rag search was cancelled"));
			const timeout = setTimeout(() => rejectRequest(new Error(timeoutMessage)), timeoutMs);
			const cleanup = () => {
				signal?.removeEventListener("abort", abort);
				clearTimeout(timeout);
			};
			const rejectRequest = (error: Error) => {
				if (this.pending.delete(id)) {
					cleanup();
					reject(error);
				}
			};
			this.pending.set(id, { resolve, reject, cleanup });
			signal?.addEventListener("abort", abort, { once: true });
			if (signal?.aborted) {
				abort();
				return;
			}
			try {
				this.child.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", id, method, ...(params === undefined ? {} : { params }) })}\n`);
			} catch (error) {
				rejectRequest(error instanceof Error ? error : new Error(String(error)));
			}
		});
	}

	private notify(method: string): void {
		if (this.isClosed) {
			throw new Error("rag server is not available");
		}
		this.child.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", method })}\n`);
	}

	private handleStdout = (chunk: string): void => {
		this.stdout += chunk;
		for (;;) {
			const newline = this.stdout.indexOf("\n");
			if (newline < 0) {
				if (this.stdout.length > maximumUnframedStdoutLength) {
					this.failProtocol(new Error("rag server exceeded the maximum unframed stdout length"));
				}
				return;
			}
			const line = this.stdout.slice(0, newline);
			this.stdout = this.stdout.slice(newline + 1);
			if (line.trim().length === 0) {
				continue;
			}
			let response: unknown;
			try {
				response = JSON.parse(line);
			} catch {
				this.failProtocol(new Error("rag server returned invalid JSON-RPC output"));
				return;
			}
			if (!isRecord(response) || response.jsonrpc !== "2.0") {
				this.failProtocol(new Error("rag server returned invalid JSON-RPC output"));
				return;
			}
			if (typeof response.method === "string" && !("id" in response)) {
				continue;
			}
			if (typeof response.id !== "number") {
				this.failProtocol(new Error("rag server returned invalid JSON-RPC output"));
				return;
			}
			const pending = this.pending.get(response.id);
			if (pending === undefined) {
				continue;
			}
			this.pending.delete(response.id);
			pending.cleanup();
			if (isRecord(response.error) && typeof response.error.message === "string") {
				pending.reject(new Error(response.error.message));
			} else if ("result" in response) {
				pending.resolve(response.result);
			} else {
				pending.reject(new Error("rag server returned invalid JSON-RPC output"));
			}
		}
	};

	private handleStderr = (chunk: string): void => {
		this.stderr = `${this.stderr}${chunk}`.slice(-maximumStderrLength);
	};

	private handleStdoutError = (): void => {
		this.failProtocol(serverError(this.stderr));
	};

	private handleStderrError = (): void => {
		this.failProtocol(serverError(this.stderr));
	};

	private handleStdinError = (): void => {
		this.failProtocol(serverError(this.stderr));
	};

	private handleChildError = (error: Error): void => {
		const isMissingCommand = (error as NodeJS.ErrnoException).code === "ENOENT";
		this.failProtocol(isMissingCommand ? new Error("rag command was not found") : serverError(this.stderr));
	};

	private handleChildExit = (): void => {
		if (!this.isClosed) {
			this.failProtocol(serverError(this.stderr));
		}
	};

	private failProtocol(error: Error): void {
		if (!this.isClosed) {
			this.stop(error);
		}
	}

	private stop(error: Error): void {
		this.isClosed = true;
		this.onClosed();
		for (const pending of this.pending.values()) {
			pending.cleanup();
			pending.reject(error);
		}
		this.pending.clear();
		this.stdout = "";
		this.child.stdout.off("data", this.handleStdout);
		this.child.stderr.off("data", this.handleStderr);
		this.child.stdin.end();
		if (!this.child.killed) {
			this.child.kill();
		}
	}
}

function mapSearchResult(value: unknown): { content: McpTextContent[]; details: { hits: Record<string, unknown>[] } } {
	if (!isRecord(value)) {
		throw new Error("rag server returned invalid structured output");
	}
	if (value.isError === true) {
		const text = Array.isArray(value.content) ? value.content.find(isTextContent)?.text : undefined;
		throw new Error(text ?? "rag search failed");
	}
	if (!Array.isArray(value.structuredContent) || !value.structuredContent.every(isRecord)) {
		throw new Error("rag server returned invalid structured output");
	}
	const hits = value.structuredContent;
	const content: McpTextContent[] = Array.isArray(value.content) && value.content.every(isTextContent)
		? value.content
		: [{ type: "text", text: JSON.stringify(hits) }];
	return { content, details: { hits } };
}

/**
 * Registers Pi's personal-memory search tool.
 *
 * @param pi - The Pi ExtensionAPI that registers the tool and lifecycle handlers.
 * @returns Nothing.
 * @throws {Error} If Pi rejects tool or lifecycle registration.
 */
export default function ragExtension(pi: ExtensionAPI, spawnProcess: SpawnProcess = spawn, timeouts: Timeouts = defaultTimeouts): void {
	let session: McpSession | undefined;
	let starting: Promise<McpSession> | undefined;
	let isMemorySessionActive = false;

	const startSession = (): Promise<McpSession> => {
		if (session?.isAvailable && starting === undefined) {
			return Promise.resolve(session);
		}
		if (starting !== undefined) {
			return starting;
		}
		let nextSession: McpSession;
		nextSession = new McpSession(spawnProcess, () => {
			if (session === nextSession) {
				session = undefined;
			}
		}, timeouts);
		session = nextSession;
		let initialization!: Promise<McpSession>;
		initialization = (async () => {
			try {
				await nextSession.initialize();
				return nextSession;
			} catch (error) {
				await nextSession.close();
				throw error;
			} finally {
				if (starting === initialization) {
					starting = undefined;
				}
			}
		})();
		starting = initialization;
		return initialization;
	};

	pi.on("session_start", () => {
		isMemorySessionActive = true;
	});
	pi.on("session_shutdown", async () => {
		isMemorySessionActive = false;
		const activeSession = session;
		session = undefined;
		await activeSession?.close();
	});
	pi.registerTool({
		name: "search_memory",
		label: "Search Memory",
		description: "Hybrid semantic and keyword search over personal notes, documents, session transcripts, and agent memory. Returns recent and relevant chunks with source metadata.",
		parameters: searchMemoryParameters,
		async execute(_toolCallId, params: SearchMemoryInput, signal) {
			if (!isMemorySessionActive) {
				throw new Error("rag memory search is only available during an active session");
			}
			return (await startSession()).callSearch(params, signal);
		},
	});
}
