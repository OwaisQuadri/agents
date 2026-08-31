import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { mkdir, mkdtemp, readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve, sep } from "node:path";

export type CommandResult = {
	stdout: string;
	stderr: string;
	code: number;
	isKilled: boolean;
};

const BINARY = "transcript-directed-video-processor";

// Each video_analyze call gets its own scratch directory under this root, and
// returns that directory's path as work_dir for a later video_review call to
// reuse. Kept under the OS temp root rather than the caller's project
// directory: the underlying videos/frames/captions this tool downloads have
// nothing to do with the repository being worked on.
const SCRATCH_ROOT = join(tmpdir(), "tdvp-sessions");

const analyzeParameters = {
	type: "object",
	additionalProperties: false,
	properties: {
		url: {
			type: "string",
			description: "A YouTube video URL to analyze. Mutually exclusive with input; exactly one is required.",
		},
		input: {
			type: "string",
			description:
				"Absolute path to a local video file that has a matching same-named .srt or .vtt sidecar caption file next to it. Mutually exclusive with url; exactly one is required. Local transcription (no sidecar caption file) is not supported yet.",
		},
	},
} as const;

const reviewParameters = {
	type: "object",
	additionalProperties: false,
	required: ["work_dir", "moments", "model"],
	properties: {
		work_dir: {
			type: "string",
			description: "The work_dir value returned by a prior video_analyze call for this same video.",
		},
		moments: {
			type: "array",
			items: { type: "integer", minimum: 0 },
			minItems: 1,
			description: "Moment indices to review, taken from video_analyze's returned moments array, e.g. [0, 5, 12].",
		},
		model: {
			type: "string",
			description:
				"A genai model-name string naming the configured vision-capable model to review each extracted frame, e.g. gpt-5.1, claude-sonnet-4-5, gemini-2.5-pro. Requires the matching provider API key set in the environment (OPENAI_API_KEY, ANTHROPIC_API_KEY, or GEMINI_API_KEY).",
		},
		clip: {
			type: "boolean",
			default: false,
			description:
				"Also extract a short clip alongside the still frame, for archival evidence. The vision model always reviews the still frame, never the clip.",
		},
	},
} as const;

function isMissingCommand(error: unknown): boolean {
	return error instanceof Error && "code" in error && (error as { code?: unknown }).code === "ENOENT";
}

function commandFailure(label: string, result: CommandResult): Error {
	const stderr = result.stderr.trim();
	return new Error(stderr || `${label} failed with exit code ${result.code}`);
}

async function runBinary(
	exec: ExtensionAPI["exec"],
	args: string[],
	cwd: string,
	signal: AbortSignal | undefined,
): Promise<void> {
	const label = `${BINARY} ${args[0]}`;
	let result: CommandResult;
	try {
		result = await exec(BINARY, args, { cwd, signal });
	} catch (error) {
		if (isMissingCommand(error)) {
			throw new Error(`${BINARY} was not found on PATH — build and install it first (tools/${BINARY}/, see its README)`);
		}
		throw error;
	}
	if (result.isKilled || signal?.aborted) {
		throw new Error(`${label} was cancelled`);
	}
	if (result.code !== 0) {
		throw commandFailure(label, result);
	}
}

// work_dir is a scratch directory this extension creates under SCRATCH_ROOT and
// hands back to the caller — video_review passes it straight through as the
// spawned process's cwd, with `--dir .`. The binary's own path-escape guard
// (main.rs's resolve_contained_dir_under) only protects a --out/--dir from
// escaping ITS OWN cwd; since this extension sets that cwd to work_dir itself,
// an arbitrary caller-supplied work_dir would defeat that guard entirely rather
// than being caught by it. Checked here instead, before it's ever used as a cwd.
function assertManagedWorkDir(workDir: string): void {
	const resolved = resolve(workDir);
	const root = resolve(SCRATCH_ROOT);
	if (resolved !== root && !resolved.startsWith(root + sep)) {
		throw new Error(`work_dir must be a directory returned by video_analyze, got: ${workDir}`);
	}
}

async function readJsonFile(path: string): Promise<unknown> {
	const raw = await readFile(path, "utf8");
	return JSON.parse(raw) as unknown;
}

/**
 * Registers Pi's transcript-directed video analysis tools: video_analyze (fetch
 * and segment a transcript into candidate moments) and video_review (extract
 * frames for named moments and review them with a configured vision-capable
 * model).
 *
 * @param pi - Pi extension application programming interface used to register and execute the tools.
 * @returns Nothing.
 * @throws An error if Pi rejects tool registration.
 */
export default function transcriptDirectedVideoProcessorExtension(pi: ExtensionAPI): void {
	pi.registerTool({
		name: "video_analyze",
		label: "Analyze Video Transcript",
		description:
			"Fetches a YouTube video's captions (or a local video's sidecar .srt/.vtt) and segments the transcript into candidate chapter-like moments with timestamps. Text-only — no vision model call, no video download. Returns a work_dir to pass into video_review for visual evidence on selected moments.",
		parameters: analyzeParameters,
		async execute(_toolCallId, params: { url?: string; input?: string }, signal) {
			if (Boolean(params.url) === Boolean(params.input)) {
				throw new Error("pass exactly one of url or input");
			}
			await mkdir(SCRATCH_ROOT, { recursive: true });
			const workDir = await mkdtemp(join(SCRATCH_ROOT, "session-"));

			const source = (params.url ?? params.input) as string;
			await runBinary(pi.exec, ["analyze", params.url ? "--url" : "--input", source, "--out", "."], workDir, signal);

			const chapters = await readJsonFile(join(workDir, "chapters.json"));
			return {
				content: [{ type: "text", text: JSON.stringify({ work_dir: workDir, ...(chapters as object) }) }],
				details: { work_dir: workDir, chapters },
			};
		},
	});

	pi.registerTool({
		name: "video_review",
		label: "Review Video Moments",
		description:
			"Extracts a frame (and optionally a short clip) for each named moment index from a prior video_analyze call, and sends each frame to a configured vision-capable model for visual review. Requires the matching provider API key in the environment. A single review call downloads the full source video once and reuses it for every named moment — expect this to take minutes on a long video the first time it's called for a given work_dir.",
		parameters: reviewParameters,
		async execute(
			_toolCallId,
			params: { work_dir: string; moments: number[]; model: string; clip?: boolean },
			signal,
		) {
			assertManagedWorkDir(params.work_dir);
			const args = ["review", "--dir", ".", "--moments", params.moments.join(","), "--model", params.model];
			if (params.clip) {
				args.push("--clip", "yes");
			}
			await runBinary(pi.exec, args, params.work_dir, signal);

			const evidence = await readJsonFile(join(params.work_dir, "evidence.json"));
			return {
				content: [{ type: "text", text: JSON.stringify(evidence) }],
				details: { evidence },
			};
		},
	});
}
