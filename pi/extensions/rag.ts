import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

type SearchMemoryInput = {
	query: string;
	k?: number;
	source_filter?: string;
};

type CommandResult = {
	stdout: string;
	stderr: string;
	code: number;
	isKilled: boolean;
};

const searchMemoryParameters = {
	type: "object",
	additionalProperties: false,
	required: ["query"],
	properties: {
		query: {
			type: "string",
			description: "Search query for the personal knowledge base",
		},
		k: {
			type: "integer",
			minimum: 1,
			default: 8,
			description: "Maximum number of results",
		},
		source_filter: {
			type: "string",
			description: "Configured source name to search",
		},
	},
} as const;

function parseSearchResults(stdout: string): Record<string, unknown>[] {
	const lines = stdout.split("\n").filter((line) => line.trim().length > 0);

	try {
		return lines.map((line) => {
			const value: unknown = JSON.parse(line);
			if (value === null || typeof value !== "object" || Array.isArray(value)) {
				throw new Error("result is not an object");
			}
			return value as Record<string, unknown>;
		});
	} catch {
		throw new Error("rag search returned invalid JSON output");
	}
}

function commandArguments(params: SearchMemoryInput): string[] {
	const args = ["search", params.query, "--k", String(params.k ?? 8)];
	if (params.source_filter !== undefined) {
		args.push("--source", params.source_filter);
	}
	args.push("--json");
	return args;
}

function commandFailure(result: CommandResult): Error {
	const stderr = result.stderr.trim();
	if (result.code === 1 && stderr.length === 0) {
		return new Error("rag command was not found");
	}
	return new Error(stderr || `rag search failed with exit code ${result.code}`);
}

function isMissingCommand(error: unknown): boolean {
	return error instanceof Error && "code" in error && error.code === "ENOENT";
}

/**
 * Registers Pi's personal-memory search tool.
 *
 * @param pi - Pi extension application programming interface used to register and execute the tool.
 * @returns Nothing.
 * @throws An error if Pi rejects tool registration.
 */
export default function ragExtension(pi: ExtensionAPI): void {
	pi.registerTool({
		name: "search_memory",
		label: "Search Memory",
		description:
			"Hybrid semantic and keyword search over personal notes, documents, session transcripts, and agent memory. Returns recent and relevant chunks with source metadata.",
		parameters: searchMemoryParameters,
		async execute(_toolCallId, params: SearchMemoryInput, signal) {
			let result: CommandResult;
			try {
				result = await pi.exec("rag", commandArguments(params), { signal });
			} catch (error) {
				if (isMissingCommand(error)) {
					throw new Error("rag command was not found");
				}
				throw error;
			}

			if (result.isKilled || signal?.aborted) {
				throw new Error("rag search was cancelled");
			}
			if (result.code !== 0) {
				throw commandFailure(result);
			}

			const hits = parseSearchResults(result.stdout);
			return {
				content: [{ type: "text", text: JSON.stringify(hits) }],
				details: { hits },
			};
		},
	});
}
