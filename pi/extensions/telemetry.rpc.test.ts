import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { constants } from "node:fs";
import { access, lstat, mkdir, mkdtemp, readFile, readdir, readlink, realpath, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { delimiter, dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import { StringDecoder } from "node:string_decoder";
import { fileURLToPath } from "node:url";
import { test } from "node:test";

type JsonRecord = Record<string, unknown>;

type Waiter = {
	label: string;
	predicate: (event: JsonRecord) => boolean;
	resolve: (event: JsonRecord) => void;
	reject: (error: Error) => void;
	timeout: ReturnType<typeof setTimeout>;
};

const RPC_REQUEST_TIMEOUT_MS = 10_000;
const RPC_PROCESS_TIMEOUT_MS = 20_000;
const MAX_STDOUT_LINES = 100;
const MAX_STDERR_LINES = 20;
const MAX_CAPTURE_BYTES = 128 * 1024;

function isRecord(value: unknown): value is JsonRecord {
	return typeof value === "object" && value !== null && !Array.isArray(value);
}

async function resolvePiExecutable(): Promise<string> {
	const executableName = process.platform === "win32" ? "pi.cmd" : "pi";

	for (const pathDirectory of (process.env.PATH ?? "").split(delimiter)) {
		if (pathDirectory.length === 0) {
			continue;
		}

		const candidate = resolve(pathDirectory, executableName);
		try {
			await access(candidate, constants.X_OK);
			return await realpath(candidate);
		} catch {
			continue;
		}
	}

	throw new Error("Pi executable was not found on the parent PATH");
}

function isPathBeneath(path: string, root: string): boolean {
	const relativePath = relative(resolve(root), resolve(path));
	return relativePath.length > 0 && relativePath !== ".." && !relativePath.startsWith(`..${sep}`) && !isAbsolute(relativePath);
}

async function snapshotNamespace(root: string): Promise<Map<string, string>> {
	const snapshot = new Map<string, string>();

	async function walk(directory: string): Promise<void> {
		const entries = await readdir(directory, { withFileTypes: true });
		entries.sort((left, right) => left.name.localeCompare(right.name));

		for (const entry of entries) {
			const path = join(directory, entry.name);
			const stats = await lstat(path);

			if (stats.isDirectory()) {
				snapshot.set(path, "directory");
				await walk(path);
				continue;
			}

			if (stats.isFile()) {
				snapshot.set(path, `file:${(await readFile(path)).toString("base64")}`);
				continue;
			}

			if (stats.isSymbolicLink()) {
				snapshot.set(path, `symbolic-link:${await readlink(path)}`);
				continue;
			}

			snapshot.set(path, `other:${stats.mode}:${stats.size}`);
		}
	}

	await walk(root);
	return snapshot;
}

function subtreeSnapshot(snapshot: Map<string, string>, root: string): Map<string, string> {
	return new Map([...snapshot].filter(([path]) => path === root || isPathBeneath(path, root)));
}

function changedPaths(before: Map<string, string>, after: Map<string, string>): string[] {
	const paths = new Set([...before.keys(), ...after.keys()]);
	return [...paths].filter((path) => before.get(path) !== after.get(path));
}

test("telemetry RPC mode drives status, filtered output, feedback, and clean shutdown", async (t) => {
	const namespaceDirectory = await mkdtemp(join(tmpdir(), "telemetry-rpc-"));
	const allowedRoot = join(namespaceDirectory, "allowed");
	const outsideControlDirectory = join(namespaceDirectory, "outside-control");
	const writablePaths = {
		cwd: join(allowedRoot, "workspace"),
		HOME: join(allowedRoot, "home"),
		PI_CODING_AGENT_DIR: join(allowedRoot, "agent"),
		TMPDIR: join(allowedRoot, "tmp"),
		XDG_CACHE_HOME: join(allowedRoot, "cache"),
		XDG_CONFIG_HOME: join(allowedRoot, "config"),
		XDG_DATA_HOME: join(allowedRoot, "data"),
	} as const;

	await Promise.all([
		...Object.values(writablePaths).map(async (path) => await mkdir(path, { recursive: true })),
		mkdir(outsideControlDirectory, { recursive: true }),
	]);
	await writeFile(join(outsideControlDirectory, "sentinel.bin"), Buffer.from([0x00, 0xff, 0x54, 0x31, 0x34, 0x0a]));

	const piExecutable = await resolvePiExecutable();
	const childEnvironment = {
		HOME: writablePaths.HOME,
		NO_COLOR: "1",
		PATH: dirname(process.execPath),
		PI_CODING_AGENT_DIR: writablePaths.PI_CODING_AGENT_DIR,
		TERM: "dumb",
		TMPDIR: writablePaths.TMPDIR,
		XDG_CACHE_HOME: writablePaths.XDG_CACHE_HOME,
		XDG_CONFIG_HOME: writablePaths.XDG_CONFIG_HOME,
		XDG_DATA_HOME: writablePaths.XDG_DATA_HOME,
	};
	const allowlistedEnvironmentKeys = [
		"HOME",
		"NO_COLOR",
		"PATH",
		"PI_CODING_AGENT_DIR",
		"TERM",
		"TMPDIR",
		"XDG_CACHE_HOME",
		"XDG_CONFIG_HOME",
		"XDG_DATA_HOME",
	];
	assert.deepEqual(Object.keys(childEnvironment).sort(), allowlistedEnvironmentKeys);
	assert.ok(Object.keys(childEnvironment).every((key) => !/(credential|token|key|secret|auth|proxy)/i.test(key)));
	assert.ok(Object.values(writablePaths).every((path) => isPathBeneath(path, allowedRoot)));

	const agentDirectory = writablePaths.PI_CODING_AGENT_DIR;

	const packageName = "@earendil-works/pi-coding-agent";
	const packageVersion = "0.84.2";
	const telemetryExtensionPath = fileURLToPath(new URL("./telemetry.ts", import.meta.url));
	const telemetryStorePath = join(agentDirectory, "telemetry.jsonl");
	const seedRecords = [
		{
			recordType: "run",
			runId: "run-2",
			parentRunId: null,
			packageName,
			packageVersion,
			agentName: "agent-a",
			startedAt: "2026-08-17T02:25:00.000Z",
			settledAt: "2026-08-17T02:25:20.000Z",
			durationMs: 20000,
			status: "succeeded",
			tokens: {
				input: 15,
				output: 30,
				cacheRead: null,
				cacheWrite: null,
			},
			costUsd: 0.25,
		},
		{
			recordType: "run",
			runId: "run-1",
			parentRunId: null,
			packageName,
			packageVersion,
			agentName: "agent-a",
			startedAt: "2026-08-17T02:24:00.000Z",
			settledAt: "2026-08-17T02:24:10.000Z",
			durationMs: 10000,
			status: "succeeded",
			tokens: {
				input: 10,
				output: 20,
				cacheRead: null,
				cacheWrite: null,
			},
			costUsd: 0.25,
		},
	] as const;

	await writeFile(telemetryStorePath, `${seedRecords.map((record) => JSON.stringify(record)).join("\n")}\n`, "utf8");
	const namespaceBeforeSpawn = await snapshotNamespace(namespaceDirectory);

	const child = spawn(piExecutable, ["--mode", "rpc", "--no-session", "--extension", telemetryExtensionPath], {
		cwd: writablePaths.cwd,
		env: childEnvironment,
		stdio: ["pipe", "pipe", "pipe"],
	});

	const stdoutEvents: JsonRecord[] = [];
	const uiRequests: JsonRecord[] = [];
	const stderrLines: string[] = [];
	const waiters = new Set<Waiter>();
	const stdoutDecoder = new StringDecoder("utf8");
	const stderrDecoder = new StringDecoder("utf8");
	let stdoutBuffer = "";
	let stderrBuffer = "";
	let stdoutBytes = 0;
	let stderrBytes = 0;
	let isStdoutFinished = false;
	let isStderrFinished = false;
	let isClosed = false;
	let failure: Error | null = null;
	let closeResolve: (result: { code: number | null; signal: string | null }) => void = () => undefined;
	const closePromise = new Promise<{ code: number | null; signal: string | null }>((resolveClose) => {
		closeResolve = resolveClose;
	});

	function rejectWaiters(error: Error): void {
		for (const waiter of waiters) {
			clearTimeout(waiter.timeout);
			waiter.reject(error);
		}
		waiters.clear();
	}

	function fail(error: Error): void {
		if (failure !== null) {
			return;
		}

		failure = error;
		rejectWaiters(error);
		child.kill("SIGKILL");
	}

	function pushEvent(event: JsonRecord): void {
		if (failure !== null) {
			return;
		}

		stdoutEvents.push(event);
		if (event.type === "extension_ui_request") {
			uiRequests.push(event);
		}

		if (stdoutEvents.length > MAX_STDOUT_LINES) {
			fail(new Error(`stdout exceeded ${MAX_STDOUT_LINES} JSON lines`));
			return;
		}

		for (const waiter of [...waiters]) {
			if (!waiter.predicate(event)) {
				continue;
			}

			clearTimeout(waiter.timeout);
			waiters.delete(waiter);
			waiter.resolve(event);
		}
	}

	function drainStdoutBuffer(isFinal: boolean): void {
		while (failure === null) {
			const newlineIndex = stdoutBuffer.indexOf("\n");
			if (newlineIndex === -1 && (!isFinal || stdoutBuffer.length === 0)) {
				return;
			}

			let line: string;
			if (newlineIndex === -1) {
				line = stdoutBuffer;
				stdoutBuffer = "";
			} else {
				line = stdoutBuffer.slice(0, newlineIndex);
				stdoutBuffer = stdoutBuffer.slice(newlineIndex + 1);
			}

			if (line.endsWith("\r")) {
				line = line.slice(0, -1);
			}
			if (line.length === 0) {
				continue;
			}

			let parsed: unknown;
			try {
				parsed = JSON.parse(line);
			} catch (error) {
				fail(new Error(`stdout is not valid JSON: ${error instanceof Error ? error.message : String(error)}`));
				return;
			}

			if (!isRecord(parsed)) {
				fail(new Error("stdout line is not a JSON object"));
				return;
			}

			pushEvent(parsed);
		}
	}

	function drainStderrBuffer(isFinal: boolean): void {
		while (failure === null) {
			const newlineIndex = stderrBuffer.indexOf("\n");
			if (newlineIndex === -1 && (!isFinal || stderrBuffer.length === 0)) {
				return;
			}

			let line: string;
			if (newlineIndex === -1) {
				line = stderrBuffer;
				stderrBuffer = "";
			} else {
				line = stderrBuffer.slice(0, newlineIndex);
				stderrBuffer = stderrBuffer.slice(newlineIndex + 1);
			}

			if (line.endsWith("\r")) {
				line = line.slice(0, -1);
			}
			if (line.length === 0) {
				continue;
			}

			stderrLines.push(line);
			if (stderrLines.length > MAX_STDERR_LINES) {
				fail(new Error(`stderr exceeded ${MAX_STDERR_LINES} lines`));
				return;
			}
		}
	}

	function pushStdoutChunk(chunk: Buffer | string): void {
		if (failure !== null) {
			return;
		}

		stdoutBytes += typeof chunk === "string" ? Buffer.byteLength(chunk, "utf8") : chunk.length;
		if (stdoutBytes > MAX_CAPTURE_BYTES) {
			fail(new Error(`stdout exceeded ${MAX_CAPTURE_BYTES} bytes`));
			return;
		}

		stdoutBuffer += typeof chunk === "string" ? chunk : stdoutDecoder.write(chunk);
		drainStdoutBuffer(false);
	}

	function pushStderrChunk(chunk: Buffer | string): void {
		if (failure !== null) {
			return;
		}

		stderrBytes += typeof chunk === "string" ? Buffer.byteLength(chunk, "utf8") : chunk.length;
		if (stderrBytes > MAX_CAPTURE_BYTES) {
			fail(new Error(`stderr exceeded ${MAX_CAPTURE_BYTES} bytes`));
			return;
		}

		stderrBuffer += typeof chunk === "string" ? chunk : stderrDecoder.write(chunk);
		drainStderrBuffer(false);
	}

	function finishStdout(): void {
		if (isStdoutFinished) {
			return;
		}

		isStdoutFinished = true;
		stdoutBuffer += stdoutDecoder.end();
		drainStdoutBuffer(true);
	}

	function finishStderr(): void {
		if (isStderrFinished) {
			return;
		}

		isStderrFinished = true;
		stderrBuffer += stderrDecoder.end();
		drainStderrBuffer(true);
	}

	function waitForEvent(predicate: (event: JsonRecord) => boolean, label: string, timeoutMs: number): Promise<JsonRecord> {
		if (failure !== null) {
			return Promise.reject(failure);
		}

		for (const event of stdoutEvents) {
			if (predicate(event)) {
				return Promise.resolve(event);
			}
		}

		if (isClosed) {
			return Promise.reject(new Error(`Child exited before ${label}`));
		}

		return new Promise<JsonRecord>((resolve, reject) => {
			let waiter!: Waiter;
			const timeout = setTimeout(() => {
				fail(new Error(`Timed out waiting for ${label}`));
			}, timeoutMs);

			waiter = {
				label,
				predicate,
				resolve,
				reject,
				timeout,
			};
			waiters.add(waiter);
		});
	}

	async function waitForResponse(id: string): Promise<JsonRecord> {
		return await waitForEvent(
			(event) => event.type === "response" && event.id === id,
			`response ${id}`,
			RPC_REQUEST_TIMEOUT_MS,
		);
	}

	let requestIndex = 0;
	async function send(command: JsonRecord): Promise<JsonRecord> {
		if (failure !== null) {
			throw failure;
		}

		const id = `request-${++requestIndex}`;
		const responsePromise = waitForResponse(id);
		child.stdin.write(`${JSON.stringify({ id, ...command })}\n`);
		return await responsePromise;
	}

	function withTimeout<T>(promise: Promise<T>, timeoutMs: number, label: string): Promise<T> {
		return new Promise<T>((resolve, reject) => {
			const timeout = setTimeout(() => {
				const error = new Error(`Timed out waiting for ${label}`);
				fail(error);
				reject(error);
			}, timeoutMs);

			promise
				.then(resolve, reject)
				.finally(() => {
					clearTimeout(timeout);
				});
		});
	}

	child.stdout.on("data", (chunk) => {
		pushStdoutChunk(chunk);
	});
	child.stdout.once("end", finishStdout);
	child.stderr.on("data", (chunk) => {
		pushStderrChunk(chunk);
	});
	child.stderr.once("end", finishStderr);
	child.once("error", (error) => {
		const failureError = error instanceof Error ? error : new Error(String(error));
		fail(failureError);
		isClosed = true;
		closeResolve({ code: null, signal: null });
	});
	child.once("close", (code, signal) => {
		finishStdout();
		finishStderr();
		isClosed = true;
		const closeCode = code ?? null;
		const closeSignal = signal ?? null;
		if (failure !== null) {
			rejectWaiters(failure);
		} else if (closeCode !== 0 || closeSignal !== null) {
			rejectWaiters(new Error(`Child exited with code ${String(closeCode)}${closeSignal === null ? "" : ` and signal ${closeSignal}`}`));
		} else if (waiters.size > 0) {
			rejectWaiters(new Error("Child exited before expected output arrived"));
		}
		closeResolve({ code: closeCode, signal: closeSignal });
	});

	t.after(async () => {
		if (!isClosed) {
			child.kill("SIGKILL");
		}
		await withTimeout(closePromise, RPC_PROCESS_TIMEOUT_MS, "child cleanup").catch(() => undefined);
	});

	const startupStatusEvent = await waitForEvent(
		(event) => event.type === "extension_ui_request" && event.method === "setStatus" && event.statusKey === "telemetry",
		"startup telemetry status",
		RPC_REQUEST_TIMEOUT_MS,
	);
	assert.equal(startupStatusEvent.type, "extension_ui_request");
	assert.equal(uiRequests.length, 1);
	assert.equal(uiRequests[0]?.method, "setStatus");
	assert.equal(uiRequests[0]?.statusKey, "telemetry");
	assert.equal(uiRequests[0]?.statusText, undefined);

	const getCommandsResponse = await send({ type: "get_commands" });
	assert.equal(getCommandsResponse.type, "response");
	assert.equal(getCommandsResponse.command, "get_commands");
	assert.equal(getCommandsResponse.success, true);

	const commandsResponseData = getCommandsResponse.data as { commands: Array<JsonRecord> };
	const commandsByName = new Map(commandsResponseData.commands.map((command) => [String(command.name), command]));
	for (const commandName of ["telemetry-status", "telemetry-runs", "telemetry-feedback"]) {
		const command = commandsByName.get(commandName);
		assert.ok(command, `missing command ${commandName}`);
		assert.equal(command?.source, "extension");
		assert.equal((command?.sourceInfo as JsonRecord | undefined)?.path, telemetryExtensionPath);
	}

	const statusCountBefore = uiRequests.length;
	const statusResponse = await send({ type: "prompt", message: "/telemetry-status" });
	assert.equal(statusResponse.type, "response");
	assert.equal(statusResponse.command, "prompt");
	assert.equal(statusResponse.success, true);
	assert.equal(uiRequests.length, statusCountBefore + 1);
	const statusEvent = uiRequests[statusCountBefore] as JsonRecord;
	assert.equal(statusEvent.method, "notify");
	assert.equal(statusEvent.message, "active: 0 failed: 0");

	const feedbackCountBefore = uiRequests.length;
	const feedbackAcceptedResponse = await send({ type: "prompt", message: "/telemetry-feedback run-2 accepted" });
	assert.equal(feedbackAcceptedResponse.type, "response");
	assert.equal(feedbackAcceptedResponse.command, "prompt");
	assert.equal(feedbackAcceptedResponse.success, true);
	assert.equal(uiRequests.length, feedbackCountBefore);

	const feedbackCorrectedResponse = await send({ type: "prompt", message: "/telemetry-feedback run-1 corrected" });
	assert.equal(feedbackCorrectedResponse.type, "response");
	assert.equal(feedbackCorrectedResponse.command, "prompt");
	assert.equal(feedbackCorrectedResponse.success, true);
	assert.equal(uiRequests.length, feedbackCountBefore);

	const runOutputCountBefore = uiRequests.length;
	const runFilter = {
		packageName,
		packageVersion,
		agentName: "agent-a",
		status: "succeeded",
		minimumDurationMs: 10000,
		maximumCostUsd: 0.25,
	};
	const runsResponse = await send({ type: "prompt", message: `/telemetry-runs ${JSON.stringify(runFilter)}` });
	assert.equal(runsResponse.type, "response");
	assert.equal(runsResponse.command, "prompt");
	assert.equal(runsResponse.success, true);
	assert.equal(uiRequests.length, runOutputCountBefore + 1);
	const notifyEvent = uiRequests[runOutputCountBefore] as JsonRecord;
	assert.equal(notifyEvent.method, "notify");

	const projectedRuns = JSON.parse(String(notifyEvent.message)) as Array<JsonRecord>;
	assert.deepEqual(projectedRuns.map((record) => record.runId), ["run-2", "run-1"]);
	assert.deepEqual(
		Object.keys(projectedRuns[0] as JsonRecord),
		["recordType", "runId", "parentRunId", "packageName", "packageVersion", "agentName", "startedAt", "settledAt", "durationMs", "status", "tokens", "costUsd"],
	);
	assert.deepEqual(Object.keys((projectedRuns[0]?.tokens as JsonRecord) ?? {}), ["input", "output", "cacheRead", "cacheWrite"]);

	child.stdin.end();
	const exitResult = await withTimeout(closePromise, RPC_PROCESS_TIMEOUT_MS, "child exit");
	if (failure !== null) {
		throw failure;
	}
	assert.equal(exitResult.code, 0);
	assert.equal(exitResult.signal, null);

	assert.equal(stderrLines.length, 0);
	assert.ok(stdoutEvents.every((event) => event.type === "response" || event.type === "extension_ui_request"));

	const telemetryFile = await readFile(telemetryStorePath, "utf8");
	const telemetryRecords = telemetryFile.trim().split(/\r?\n/).map((line) => JSON.parse(line) as JsonRecord);
	assert.equal(telemetryRecords.length, 4);
	assert.equal(telemetryRecords[0]?.runId, "run-2");
	assert.equal(telemetryRecords[1]?.runId, "run-1");
	assert.equal(telemetryRecords[2]?.recordType, "feedback");
	assert.equal(telemetryRecords[2]?.runId, "run-2");
	assert.equal(telemetryRecords[2]?.value, "accepted");
	assert.equal(telemetryRecords[3]?.recordType, "feedback");
	assert.equal(telemetryRecords[3]?.runId, "run-1");
	assert.equal(telemetryRecords[3]?.value, "corrected");
	assert.equal(Number.isFinite(Date.parse(String(telemetryRecords[2]?.createdAt))), true);
	assert.equal(Number.isFinite(Date.parse(String(telemetryRecords[3]?.createdAt))), true);

	const persistedTelemetryPaths = [telemetryStorePath];
	assert.ok(persistedTelemetryPaths.every((path) => isPathBeneath(path, allowedRoot)));
	assert.ok(Object.values(writablePaths).every((path) => isPathBeneath(path, allowedRoot)));

	const namespaceAfterExit = await snapshotNamespace(namespaceDirectory);
	assert.deepEqual(
		subtreeSnapshot(namespaceAfterExit, outsideControlDirectory),
		subtreeSnapshot(namespaceBeforeSpawn, outsideControlDirectory),
	);
	const changedNamespacePaths = changedPaths(namespaceBeforeSpawn, namespaceAfterExit);
	assert.ok(
		changedNamespacePaths.every((path) => isPathBeneath(path, allowedRoot)),
		`new or changed path escaped the allowed root: ${changedNamespacePaths.join(", ")}`,
	);
});
