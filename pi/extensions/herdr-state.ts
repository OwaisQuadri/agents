import type { ExtensionAPI, ExtensionCommandContext } from "@earendil-works/pi-coding-agent";
import { once } from "node:events";
import { homedir } from "node:os";
import { join } from "node:path";
import { createConnection } from "node:net";

import { HerdrCommandClient, type HerdrClient, type HerdrCommandResult, type HerdrTransport } from "./herdr-state/client.ts";
import { HerdrStateController as LiveHerdrStateController } from "./herdr-state/controller.ts";
import { findSelf, normalizeSnapshot } from "./herdr-state/engine.ts";
import type {
	HerdrAgentLocation,
	HerdrPane,
	HerdrSessionSnapshot,
	HerdrStateController,
	HerdrStateFailure,
	HerdrTab,
	HerdrWorkspace,
} from "./herdr-state/types.ts";

const COMMAND_NAME = "herdr-state";
const HERDR_BINARY = "herdr";
const EVENT_SUBSCRIPTION_ID = "pi-herdr-state-events";
const GENERAL_EVENT_FILTERS = [
	"workspace.created",
	"workspace.updated",
	"workspace.metadata_updated",
	"workspace.renamed",
	"workspace.moved",
	"workspace.reordered",
	"workspace.closed",
	"workspace.focused",
	"worktree.created",
	"worktree.opened",
	"worktree.removed",
	"tab.created",
	"tab.closed",
	"tab.focused",
	"tab.renamed",
	"tab.moved",
	"pane.created",
	"pane.closed",
	"pane.updated",
	"pane.focused",
	"pane.moved",
	"pane.exited",
	"pane.agent_detected",
	"layout.updated",
] as const;
const DEFAULT_PANE_LINE_LIMIT = 200;
const SELF_MARKER = " <- Pi is here";
const USAGE = "Usage: /herdr-state [workspace <workspace-id> | pane <pane-id> [line-limit]]";

interface LoadedState {
	snapshot: HerdrSessionSnapshot;
	self: HerdrAgentLocation | null;
}

type ParsedArguments =
	| { kind: "global" }
	| { kind: "workspace"; workspaceId: string }
	| { kind: "pane"; paneId: string; lineLimit: number };

function errorMessage(error: unknown): string {
	return error instanceof Error ? error.message : String(error);
}

function isFailure(value: unknown): value is HerdrStateFailure {
	return (
		value !== null &&
		typeof value === "object" &&
		typeof (value as { code?: unknown }).code === "string" &&
		typeof (value as { message?: unknown }).message === "string"
	);
}

function formatFailure(failure: HerdrStateFailure): string {
	return `Herdr state is not available (${failure.code}): ${failure.message}`;
}

/**
 * Reads the Herdr pane identifier injected into the Pi process environment.
 *
 * @returns The injected pane identifier, or undefined when it is absent.
 */
function selfPaneIdFromEnvironment(): string | undefined {
	const value = process.env.HERDR_PANE_ID;
	return value === undefined || value === "" ? undefined : value;
}

/**
 * Loads and normalizes the live Herdr session snapshot, and locates Pi
 * within it.
 *
 * @param client The read-only Herdr client to query.
 * @param cwd The Pi process working directory.
 * @returns The normalized snapshot and Pi's location, or a classified failure.
 */
async function loadState(client: HerdrClient, cwd: string): Promise<LoadedState | HerdrStateFailure> {
	const response = await client.snapshot();
	if (isFailure(response)) {
		return response;
	}
	let snapshot: HerdrSessionSnapshot;
	try {
		snapshot = normalizeSnapshot(response);
	} catch (error) {
		return { code: "invalid-response", message: errorMessage(error) };
	}
	return { snapshot, self: findSelf(snapshot, cwd, selfPaneIdFromEnvironment()) };
}

function formatWorkspaceLine(workspace: HerdrWorkspace, self: HerdrAgentLocation | null): string {
	const worktree = workspace.worktree !== null ? workspace.worktree.path : "(no worktree)";
	const focus = workspace.isFocused ? " [focused]" : "";
	const marker = self !== null && self.workspaceId === workspace.id ? SELF_MARKER : "";
	return `  ${workspace.id}  ${workspace.label}  ${worktree}${focus}${marker}`;
}

function formatTabLine(tab: HerdrTab, self: HerdrAgentLocation | null): string {
	const focus = tab.isFocused ? " [focused]" : "";
	const marker = self !== null && self.tabId === tab.id ? SELF_MARKER : "";
	return `  ${tab.id}  ${tab.label}${focus}${marker}`;
}

function formatPaneLine(pane: HerdrPane, self: HerdrAgentLocation | null): string {
	const focus = pane.isFocused ? " [focused]" : "";
	const cwd = pane.cwd ?? "(unknown cwd)";
	const marker = self !== null && self.paneId === pane.id ? SELF_MARKER : "";
	return `  ${pane.id}  ${cwd}${focus}${marker}`;
}

/**
 * Renders every Herdr workspace and marks Pi's own workspace, tab, and pane.
 *
 * @param snapshot The normalized Herdr session snapshot.
 * @param self Pi's location, or null when it is absent from the snapshot.
 * @returns The rendered global state text.
 */
function renderGlobalState(snapshot: HerdrSessionSnapshot, self: HerdrAgentLocation | null): string {
	const lines = ["Herdr workspaces:"];
	if (snapshot.workspaces.length === 0) {
		lines.push("  (no open workspaces)");
	} else {
		for (const workspace of snapshot.workspaces) {
			lines.push(formatWorkspaceLine(workspace, self));
		}
	}
	lines.push("");
	lines.push(
		self === null
			? "Pi location: not found in the current Herdr session."
			: `Pi location: workspace ${self.workspaceId}, tab ${self.tabId}, pane ${self.paneId}.`,
	);
	return lines.join("\n");
}

/**
 * Renders one workspace's tabs and panes, scoped to that workspace only.
 *
 * @param snapshot The normalized Herdr session snapshot.
 * @param self Pi's location, or null when it is absent from the snapshot.
 * @param workspaceId The requested workspace identifier.
 * @returns The rendered workspace detail text, or an explicit absent-workspace message.
 */
function renderWorkspaceDetail(
	snapshot: HerdrSessionSnapshot,
	self: HerdrAgentLocation | null,
	workspaceId: string,
): string {
	const workspace = snapshot.workspaces.find((candidate) => candidate.id === workspaceId);
	if (workspace === undefined) {
		return `Workspace ${workspaceId} is absent from the current Herdr session.`;
	}
	const lines = [formatWorkspaceLine(workspace, self), "", "Tabs:"];
	const tabs = snapshot.tabs.filter((tab) => tab.workspaceId === workspaceId);
	if (tabs.length === 0) {
		lines.push("  (no open tabs)");
	} else {
		for (const tab of tabs) {
			lines.push(formatTabLine(tab, self));
		}
	}
	lines.push("", "Panes:");
	const panes = snapshot.panes.filter((pane) => pane.workspaceId === workspaceId);
	if (panes.length === 0) {
		lines.push("  (no open panes)");
	} else {
		for (const pane of panes) {
			lines.push(formatPaneLine(pane, self));
		}
	}
	return lines.join("\n");
}

/**
 * Renders one pane's bounded recent output, marking Pi's own pane when it
 * matches. Pane text is always shown as literal bounded data; it is never
 * interpreted as a command or a workspace label.
 *
 * @param client The read-only Herdr client to read pane output from.
 * @param snapshot The normalized Herdr session snapshot.
 * @param self Pi's location, or null when it is absent from the snapshot.
 * @param paneId The requested pane identifier.
 * @param lineLimit The positive bounded maximum number of recent lines to show.
 * @returns The rendered pane detail text, or an explicit failure message.
 */
async function renderPaneDetail(
	client: HerdrClient,
	snapshot: HerdrSessionSnapshot,
	self: HerdrAgentLocation | null,
	paneId: string,
	lineLimit: number,
): Promise<string> {
	const output = await client.readPane(paneId, lineLimit);
	if (isFailure(output)) {
		return formatFailure(output);
	}
	const pane = snapshot.panes.find((candidate) => candidate.id === paneId);
	const lines =
		pane === undefined
			? [`Pane ${paneId} is absent from the current workspace listing.`]
			: [formatPaneLine(pane, self)];
	lines.push("");
	lines.push(
		output.isTruncated
			? `Output (bounded to the last ${lineLimit} lines; earlier output was truncated):`
			: "Output:",
	);
	lines.push(output.text.replace(/[\u0000-\u0008\u000b-\u001f\u007f-\u009f\u061c\u200e\u200f\u202a-\u202e\u2066-\u2069]/g, ""));
	return lines.join("\n");
}

/**
 * Parses the `/herdr-state` command arguments into a global, workspace, or
 * pane detail request.
 *
 * @param args The raw command argument string.
 * @returns The parsed request.
 * @throws Error when the arguments do not match a supported request shape.
 */
function parseArguments(args: string): ParsedArguments {
	const tokens = args.trim().split(/\s+/).filter((token) => token.length > 0);
	if (tokens.length === 0) {
		return { kind: "global" };
	}
	if (tokens[0] === "workspace") {
		if (tokens.length !== 2) {
			throw new Error(`Herdr state workspace detail requires exactly one workspace identifier. ${USAGE}`);
		}
		return { kind: "workspace", workspaceId: tokens[1] as string };
	}
	if (tokens[0] === "pane") {
		if (tokens.length < 2 || tokens.length > 3) {
			throw new Error(`Herdr state pane detail requires a pane identifier and an optional line limit. ${USAGE}`);
		}
		let lineLimit = DEFAULT_PANE_LINE_LIMIT;
		if (tokens.length === 3) {
			const limitToken = tokens[2] as string;
			const parsedLimit = Number(limitToken);
			if (!/^[0-9]+$/.test(limitToken) || parsedLimit < 1 || parsedLimit > 10_000) {
				throw new Error(`Herdr state pane line limit must be an integer from 1 through 10,000. ${USAGE}`);
			}
			lineLimit = parsedLimit;
		}
		return { kind: "pane", paneId: tokens[1] as string, lineLimit };
	}
	throw new Error(`Herdr state received an unrecognized request. ${USAGE}`);
}

/**
 * Registers Pi's read-only Herdr state command against an injected client.
 *
 * With no arguments the command lists every open Herdr workspace and marks
 * Pi's own workspace, tab, and pane. `workspace <id>` scopes the result to
 * one workspace's tabs and panes. `pane <id> [line-limit]` reads that
 * pane's bounded recent output. The command never writes to Herdr.
 *
 * @param pi The Pi extension application programming interface.
 * @param client The read-only Herdr client to query for state and pane output.
 * @param controller The live state controller whose cached model takes priority, when supplied.
 * @returns Nothing.
 * @throws Never during registration; the registered handler reports Herdr failures through its rendered result instead of throwing them.
 */
export function registerHerdrStateCommand(
	pi: ExtensionAPI,
	client: HerdrClient,
	controller?: HerdrStateController,
): void {
	pi.registerCommand(COMMAND_NAME, {
		description: "List every open Herdr workspace, mark Pi's location, and show workspace or pane detail",
		handler: async (args: string, ctx: ExtensionCommandContext) => {
			const parsed = parseArguments(args);

			const current = controller?.current() ?? null;
			const loaded = current === null
				? await loadState(client, ctx.cwd)
				: { snapshot: current.snapshot, self: current.self };
			if (isFailure(loaded)) {
				ctx.ui.notify(formatFailure(loaded));
				return;
			}
			const { snapshot, self } = loaded;

			if (parsed.kind === "global") {
				ctx.ui.notify(renderGlobalState(snapshot, self));
				return;
			}
			if (parsed.kind === "workspace") {
				ctx.ui.notify(renderWorkspaceDetail(snapshot, self, parsed.workspaceId));
				return;
			}
			ctx.ui.notify(
				await renderPaneDetail(
					client,
					snapshot,
					self,
					parsed.paneId,
					parsed.lineLimit,
				),
			);
		},
	});
}

function eventSubscriptionRequest(): string {
	return `${JSON.stringify({
		id: EVENT_SUBSCRIPTION_ID,
		method: "events.subscribe",
		params: {
			subscriptions: GENERAL_EVENT_FILTERS.map((type) => ({ type })),
		},
	})}\n`;
}

async function* subscribeToHerdrEvents(signal?: AbortSignal): AsyncIterable<string> {
	if (signal?.aborted) {
		return;
	}
	const socketPath = process.env.HERDR_SOCKET_PATH ?? join(homedir(), ".config", "herdr", "herdr.sock");
	const socket = createConnection(socketPath);
	const closeForAbort = () => socket.destroy();
	signal?.addEventListener("abort", closeForAbort, { once: true });
	try {
		await once(socket, "connect");
		if (signal?.aborted) {
			return;
		}
		socket.write(eventSubscriptionRequest());
		socket.setEncoding("utf8");
		let buffered = "";
		for await (const chunk of socket) {
			buffered += String(chunk);
			let newlineIndex = buffered.indexOf("\n");
			while (newlineIndex !== -1) {
				yield buffered.slice(0, newlineIndex);
				buffered = buffered.slice(newlineIndex + 1);
				newlineIndex = buffered.indexOf("\n");
			}
		}
		if (buffered !== "") {
			yield buffered;
		}
	} catch (error) {
		if (!signal?.aborted) {
			throw error;
		}
	} finally {
		signal?.removeEventListener("abort", closeForAbort);
		socket.destroy();
	}
}

/**
 * Builds a cancellable read-only Herdr transport. It runs snapshot and pane
 * reads through Pi and opens one filtered event subscription on the Herdr
 * local socket.
 *
 * @param pi The Pi extension application programming interface.
 * @returns The read-only Herdr transport.
 * @throws Event iteration throws when the socket cannot connect or fails unexpectedly.
 */
export function createTransport(pi: ExtensionAPI): HerdrTransport {
	return {
		runCommand: async (
			commandArguments: string[],
			signal?: AbortSignal,
		): Promise<HerdrCommandResult> => {
			const result = await pi.exec(
				HERDR_BINARY,
				commandArguments,
				signal === undefined ? undefined : { signal },
			);
			return { code: result.code, stdout: result.stdout, stderr: result.stderr };
		},
		subscribeEvents: subscribeToHerdrEvents,
	};
}

/**
 * Registers Pi's read-only Herdr state command.
 *
 * @param pi The Pi extension application programming interface.
 * @param client The read-only Herdr client, or the default command client.
 * @param controller The state controller, or a controller owned by this extension.
 * @returns Nothing.
 * @throws Never; Herdr access failures are reported through the command's rendered result.
 */
export default function herdrState(
	pi: ExtensionAPI,
	client: HerdrClient = new HerdrCommandClient(createTransport(pi)),
	controller: HerdrStateController = new LiveHerdrStateController(client),
): void {
	pi.on("session_start", (_event, ctx) => {
		void controller.start(ctx.cwd, selfPaneIdFromEnvironment());
	});
	pi.on("session_shutdown", () => {
		controller.stop();
	});
	registerHerdrStateCommand(pi, client, controller);
}
