import assert from "node:assert/strict";
import { test } from "node:test";
import { runProcess } from "./process.ts";

const command = (source: string, budget = 1000) => runProcess({ command: process.execPath, args: ["-e", source], input: "fixture", deadline: performance.now() + budget });
test("process collects output only after close", async () => assert.equal(await command('process.stdin.resume(); process.stdin.on("end", () => process.stdout.write("complete"))'), "complete"));
for (const source of ['process.exit(2)', 'process.stdout.write("partial"); setTimeout(() => process.stdout.write("late"), 2000)', 'process.stdout.write("x".repeat(17 * 1024 * 1024))']) {
	test("process fails closed on crash, timeout or oversized output", async () => { await assert.rejects(command(source, 100)); });
}
const structuredCommand = (source: string, budget = 1000, signal?: AbortSignal) => runProcess({ command: process.execPath, args: ["-e", source], input: "", deadline: performance.now() + budget, signal, resultMode: "exit-status" });
for (const exitCode of [0, 1, 2, 7]) {
	test(`structured transport returns decoded output and exit ${exitCode}`, async () => {
		assert.deepEqual(await structuredCommand(`process.stdout.write('complete', () => process.exit(${exitCode}))`), { stdout: "complete", exitCode });
	});
}
for (const source of [
	'process.kill(process.pid, "SIGTERM")',
	'process.stdout.write(Buffer.from([255]))',
	'process.stdout.write("x".repeat(17 * 1024 * 1024))',
	'process.stderr.write("x".repeat(17 * 1024 * 1024))',
]) {
	test(`structured transport rejects unsafe output or signal: ${source}`, async () => { await assert.rejects(structuredCommand(source)); });
}
test("structured transport preserves timeout, cancellation and input bounds", async () => {
	await assert.rejects(structuredCommand('setInterval(() => {}, 1000)', 50), /deadline/);
	const controller = new AbortController();
	const promise = structuredCommand('setInterval(() => {}, 1000)', 1000, controller.signal);
	controller.abort();
	await assert.rejects(promise, /aborted/);
	await assert.rejects(structuredCommand('', 1000, controller.signal), /aborted/);
	await assert.rejects(runProcess({ command: process.execPath, args: [], input: "x".repeat(8 * 1024 * 1024 + 1), deadline: performance.now() + 1000, resultMode: "exit-status" }), /stream limit/);
	await assert.rejects(runProcess({ command: "/missing-required-checker", args: [], input: "", deadline: performance.now() + 1000, resultMode: "exit-status" }), /could not start/);
});
test("default transport still rejects nonzero with complete output", async () => {
	await assert.rejects(command('process.stdin.resume(); process.stdin.on("end", () => process.stdout.write("complete", () => process.exit(1)))'));
});
test("structured transport rejects a failed input stream even with a complete result", async () => {
	await assert.rejects(runProcess({ command: process.execPath, args: ["-e", 'process.stdout.write("complete", () => process.exit(1))'], input: "x".repeat(8 * 1024 * 1024), deadline: performance.now() + 3000, resultMode: "exit-status" }), /input stream failed/);
});
test("missing binary blocks", async () => { await assert.rejects(runProcess({ command: "/missing-required-checker", args: [], input: "", deadline: performance.now() + 1000 })); });
test("abort kills and drains the child", async () => {
	const controller = new AbortController();
	const promise = runProcess({ command: process.execPath, args: ["-e", "setInterval(() => {}, 1000)"], input: "", deadline: performance.now() + 1000, signal: controller.signal });
	controller.abort();
	await assert.rejects(promise);
});
