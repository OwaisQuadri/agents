import { spawn } from "node:child_process";
import { checkDeadline, ValidationCancelled, ValidationExpired } from "./direct.ts";

type ProcessOptions = { command: string; args: string[]; input: string; deadline: number; signal?: AbortSignal; cwd?: string; env?: NodeJS.ProcessEnv };
export type ProcessResult = { stdout: string; exitCode: number };

export function runProcess(options: ProcessOptions & { resultMode: "exit-status" }): Promise<ProcessResult>;
export function runProcess(options: ProcessOptions): Promise<string>;
export async function runProcess(options: ProcessOptions & { resultMode?: "exit-status" }): Promise<string | ProcessResult> {
	checkDeadline(options.deadline, options.signal);
	const input = Buffer.from(options.input);
	if (input.length > 8 * 1024 * 1024) throw new Error("Checker request exceeds the supported stream limit.");
	return new Promise((resolve, reject) => {
		const child = spawn(options.command, options.args, { cwd: options.cwd, env: options.env, stdio: [input.length ? "pipe" : "ignore", "pipe", "pipe"], detached: true });
		let failure: Error | undefined;
		const chunks: Buffer[] = [];
		let size = 0;
		let errorSize = 0;
		const kill = () => {
			if (child.pid) {
				try { process.kill(-child.pid, "SIGKILL"); } catch { child.kill("SIGKILL"); }
			}
		};
		const fail = (message: string | Error) => {
			failure ??= typeof message === "string" ? new Error(message) : message;
			kill();
		};
		const abort = () => fail(new ValidationCancelled());
		const timer = setTimeout(() => fail(new ValidationExpired()), Math.max(1, options.deadline - performance.now()));
		options.signal?.addEventListener("abort", abort, { once: true });
		if (options.signal?.aborted) abort();
		child.stdout!.on("data", (chunk: Buffer) => {
			size += chunk.length;
			if (size > 16 * 1024 * 1024) fail("Checker output exceeds the supported stream limit.");
			else if (!failure) chunks.push(chunk);
		});
		child.stderr!.on("data", (chunk: Buffer) => {
			errorSize += chunk.length;
			if (errorSize > 16 * 1024 * 1024) fail("Checker error stream exceeds the supported limit.");
		});
		child.stdout!.on("error", () => fail("Checker output stream failed."));
		child.stderr!.on("error", () => fail("Checker error stream failed."));
		child.stdin?.on("error", () => fail("Checker input stream failed."));
		child.on("error", () => fail("Required checker could not start; build or repair it before retrying."));
		child.on("close", (code, signal) => {
			clearTimeout(timer);
			options.signal?.removeEventListener("abort", abort);
			kill();
			try { checkDeadline(options.deadline, options.signal); } catch (error) { return reject(error); }
			if (failure || code === null || (code !== 0 && options.resultMode !== "exit-status") || signal) return reject(failure ?? new Error("Required checker failed."));
			try {
				const stdout = new TextDecoder("utf-8", { fatal: true, ignoreBOM: true }).decode(Buffer.concat(chunks));
				resolve(options.resultMode === "exit-status" ? { stdout, exitCode: code } : stdout);
			}
			catch { reject(new Error("Checker returned invalid UTF-8.")); }
		});
		child.stdin?.end(input);
	});
}
