import test from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync, statSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { execFileSync } from "node:child_process";
import { EventEmitter } from "node:events";
import recorder, { appendEvent, copySession } from "./recorder.mjs";

test("native root hooks retain top-level identities and child sessions", async () => {
  assert.ok(process.env.SKILL_EVAL_EXECUTABLE, "set SKILL_EVAL_EXECUTABLE to the built runner");
  const root = mkdtempSync(join(tmpdir(), "execution-recorder-test-"));
  const previousDirectory = process.env.SKILL_EVAL_EXECUTION_DIR;
  const managerKey = Symbol.for("pi-subagents:manager");
  const ownerKey = Symbol.for("skill-eval:root-recorder");
  const previousManager = globalThis[managerKey];
  const previousOwner = globalThis[ownerKey];
  try {
    process.env.SKILL_EVAL_EXECUTION_DIR = root;
    delete globalThis[ownerKey];
    const fixture = join(root, "fixture");
    mkdirSync(fixture);
    const git = (...args) => execFileSync("git", ["-C", fixture, "-c", "user.name=eval", "-c", "user.email=eval@local", "-c", "core.hooksPath=/dev/null", "-c", "commit.gpgsign=false", ...args]);
    git("init", "-q", "-b", "main");
    writeFileSync(join(fixture, "cost.mjs"), "export const value = 1;\n");
    git("add", ".");
    git("commit", "-qm", "fixture");
    const sessionFile = join(root, "native-child.jsonl");
    writeFileSync(sessionFile, '{"type":"session","id":"child-session"}\n');
    globalThis[managerKey] = { getRecord: (id) => id === "child" ? { sessionFile, status: "completed" } : undefined, waitForAll: async () => {} };
    const hooks = new Map();
    const bus = new EventEmitter();
    recorder({ on: (name, handler) => hooks.set(name, handler), events: bus });
    hooks.get("session_start")({}, { sessionManager: { getSessionId: () => "root-session" } });
    const childHooks = new Map();
    recorder({ on: (name, handler) => childHooks.set(name, handler), events: bus });
    childHooks.get("session_start")({}, {});
    childHooks.get("tool_execution_start")({ type: "tool_execution_start", toolName: "read", toolCallId: "child-only" });
    hooks.get("tool_execution_start")({ type: "tool_execution_start", toolName: "Agent", toolCallId: "root-call", args: { inherit_context: false } });
    bus.emit("subagents:started", { id: "child", type: "tester", description: "test" });
    bus.emit("subagents:completed", { id: "child" });
    hooks.get("tool_execution_end")({ type: "tool_execution_end", toolName: "Agent", toolCallId: "root-call", result: { details: { agentId: "child" } }, isError: false });
    await hooks.get("session_shutdown")();
    const events = readFileSync(join(root, "events.jsonl"), "utf8").trim().split("\n").map(JSON.parse);
    assert.deepEqual(events.map((event) => event.type), ["recorder_started", "tool_execution_start", "worker_started", "worker_settled", "tool_execution_end", "recorder_complete"]);
    assert.deepEqual(events.map((event) => event.sequence), [0, 1, 2, 3, 4, 5]);
    assert.equal(events[2].workerType, "tester");
    assert.equal(events[4].result.details.agentId, events[2].id);
    assert.deepEqual(events[1].snapshot.files, events[2].snapshot.files);
    assert.equal(readFileSync(events[3].sessionCopy, "utf8"), readFileSync(sessionFile, "utf8"));
  } finally {
    if (previousDirectory === undefined) delete process.env.SKILL_EVAL_EXECUTION_DIR;
    else process.env.SKILL_EVAL_EXECUTION_DIR = previousDirectory;
    globalThis[managerKey] = previousManager;
    globalThis[ownerKey] = previousOwner;
    rmSync(root, { recursive: true, force: true });
  }
});

test("event append checks exact encoded and cumulative byte boundaries", () => {
  const root = mkdtempSync(join(tmpdir(), "execution-recorder-test-"));
  try {
    for (const [index, event] of [
      { type: "tool_execution_end", result: { text: "source", isError: false }, missing: undefined },
      { text: "é😀\u0000\n\r\t\b\f\ud800\\\"", values: [null, undefined, 1, false] },
    ].entries()) {
      const path = join(root, `${index}.jsonl`);
      const expected = `${JSON.stringify(event)}\n`;
      const bytes = Buffer.byteLength(expected);
      assert.throws(() => appendEvent(path, event, bytes - 1), /limit exceeded/);
      assert.equal(statSync(path).size, 0);
      appendEvent(path, event, bytes);
      assert.equal(readFileSync(path, "utf8"), expected);
      assert.throws(() => appendEvent(path, event, 2 * bytes - 1), /limit exceeded/);
      assert.equal(statSync(path).size, bytes);
      appendEvent(path, event, 2 * bytes);
      assert.equal(statSync(path).size, 2 * bytes);
    }
    const path = join(root, "large.jsonl");
    const limit = 16 * 1024 * 1024;
    const event = { text: "x".repeat(limit - Buffer.byteLength(`${JSON.stringify({ text: "" })}\n`)) };
    const start = performance.now();
    appendEvent(path, event);
    console.log(`event append bytes=${limit} elapsed_ms=${(performance.now() - start).toFixed(2)}`);
    assert.equal(statSync(path).size, limit);
    assert.throws(() => appendEvent(join(root, "overflow.jsonl"), { text: event.text + "x" }), /limit exceeded/);
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test("child copy accepts the ceiling and rejects overflow before creating a copy", () => {
  const root = mkdtempSync(join(tmpdir(), "execution-recorder-test-"));
  try {
    const source = join(root, "session.jsonl");
    writeFileSync(source, "abcd");
    copySession(source, join(root, "boundary"), 4);
    assert.equal(readFileSync(join(root, "boundary"), "utf8"), "abcd");
    assert.throws(() => copySession(source, join(root, "overflow"), 3), /limit exceeded/);
    assert.equal(existsSync(join(root, "overflow")), false);
    const limit = 16 * 1024 * 1024;
    writeFileSync(source, Buffer.alloc(limit));
    const start = performance.now();
    copySession(source, join(root, "large"));
    console.log(`child copy bytes=${limit} elapsed_ms=${(performance.now() - start).toFixed(2)}`);
    assert.deepEqual(readFileSync(join(root, "large")), readFileSync(source));
    writeFileSync(source, Buffer.alloc(limit + 1));
    assert.throws(() => copySession(source, join(root, "too-large")), /limit exceeded/);
    assert.equal(existsSync(join(root, "too-large")), false);
  } finally { rmSync(root, { recursive: true, force: true }); }
});

for (const scenario of ["event-overflow", "child-overflow", "full-log"]) {
  test(`${scenario} stops recording without recursive errors or completion`, async () => {
    const root = mkdtempSync(join(tmpdir(), "execution-recorder-test-"));
    const previousDirectory = process.env.SKILL_EVAL_EXECUTION_DIR;
    const managerKey = Symbol.for("pi-subagents:manager");
    const ownerKey = Symbol.for("skill-eval:root-recorder");
    const previousManager = globalThis[managerKey];
    const previousOwner = globalThis[ownerKey];
    try {
      process.env.SKILL_EVAL_EXECUTION_DIR = root;
      delete globalThis[ownerKey];
      const path = join(root, "events.jsonl");
      const limit = 16 * 1024 * 1024;
      if (scenario === "full-log") writeFileSync(path, " ".repeat(limit));
      const sessionFile = join(root, "native.jsonl");
      writeFileSync(sessionFile, Buffer.alloc(limit + 1));
      globalThis[managerKey] = { getRecord: () => ({ sessionFile, status: "completed" }), waitForAll: async () => {} };
      const hooks = new Map();
      const bus = new EventEmitter();
      recorder({ on: (name, handler) => hooks.set(name, handler), events: bus });
      hooks.get("session_start")({}, { sessionManager: { getSessionId: () => scenario === "event-overflow" ? "x".repeat(limit) : "root" } });
      bus.emit("subagents:completed", { id: "child" });
      const stoppedSize = statSync(path).size;
      hooks.get("tool_execution_start")({ type: "tool_execution_start", toolName: "read", toolCallId: "never-recorded" });
      bus.emit("subagents:failed", { id: "child" });
      await hooks.get("session_shutdown")();
      assert.equal(statSync(path).size, stoppedSize);
      assert.ok(stoppedSize <= limit);
      assert.equal(existsSync(join(root, "children", "0.jsonl")), false);
      if (scenario !== "full-log") {
        const events = readFileSync(path, "utf8").trim().split("\n").map(JSON.parse);
        assert.equal(events.filter((event) => event.type === "recorder_error").length, 1);
        assert.equal(events.some((event) => event.type === "recorder_complete"), false);
      }
    } finally {
      if (previousDirectory === undefined) delete process.env.SKILL_EVAL_EXECUTION_DIR;
      else process.env.SKILL_EVAL_EXECUTION_DIR = previousDirectory;
      globalThis[managerKey] = previousManager;
      globalThis[ownerKey] = previousOwner;
      rmSync(root, { recursive: true, force: true });
    }
  });
}
