import { appendFileSync, closeSync, fstatSync, mkdirSync, openSync, readSync } from "node:fs";
import { join } from "node:path";
import { execFileSync } from "node:child_process";

const MAX_LOG_BYTES = 16 * 1024 * 1024;

function checkJsonSize(value, limit) {
  let remaining = limit;
  const consume = (count) => {
    remaining -= count;
    if (remaining < 0) throw new Error("Event log byte limit exceeded");
  };
  const string = (text) => {
    consume(2 + Buffer.byteLength(text));
    const escapes = /["\\\u0000-\u001f\ud800-\udfff]/gu;
    let match;
    while ((match = escapes.exec(text)) !== null) {
      const char = match[0];
      const code = char.charCodeAt(0);
      consume(code >= 0xd800 ? 3 : char === '"' || char === "\\" || "\b\f\n\r\t".includes(char) ? 1 : 5);
    }
  };
  const visit = (item, depth) => {
    if (depth > 64) throw new Error("Event nesting limit exceeded");
    if (typeof item === "string") string(item);
    else if (item === null || typeof item === "boolean" || typeof item === "number") consume(JSON.stringify(item).length);
    else if (Array.isArray(item)) {
      consume(2);
      for (let index = 0; index < item.length; index++) {
        if (index) consume(1);
        visit(item[index] === undefined ? null : item[index], depth + 1);
      }
    } else if (typeof item === "object" && Object.getPrototypeOf(item) === Object.prototype) {
      consume(2);
      let count = 0;
      for (const key in item) {
        if (!Object.hasOwn(item, key) || item[key] === undefined) continue;
        if (count++) consume(1);
        string(key);
        consume(1);
        visit(item[key], depth + 1);
      }
    } else throw new Error("Unsupported event value");
  };
  visit(value, 0);
}

export function appendEvent(path, event, limit = MAX_LOG_BYTES) {
  const fd = openSync(path, "a");
  try {
    checkJsonSize(event, limit - fstatSync(fd).size - 1);
    const line = `${JSON.stringify(event)}\n`;
    if (fstatSync(fd).size + Buffer.byteLength(line) > limit) throw new Error("Event log byte limit exceeded");
    appendFileSync(fd, line);
  } finally { closeSync(fd); }
}

export function copySession(from, to, limit = MAX_LOG_BYTES) {
  const source = openSync(from, "r");
  try {
    if (fstatSync(source).size > limit) throw new Error("Child session byte limit exceeded");
    const target = openSync(to, "wx");
    try {
      const buffer = Buffer.alloc(8192);
      let total = 0;
      for (;;) {
        const count = readSync(source, buffer, 0, buffer.length, null);
        if (!count) break;
        total += count;
        if (total > limit) throw new Error("Child session byte limit exceeded");
        appendFileSync(target, buffer.subarray(0, count));
      }
    } finally { closeSync(target); }
  } finally { closeSync(source); }
}

export default function recorder(pi) {
  const key = Symbol.for("skill-eval:root-recorder");
  const directory = process.env.SKILL_EVAL_EXECUTION_DIR;
  let isRoot = false;
  let sequence = 0;
  const workers = new Set();
  const settled = new Set();
  let isStopped = false;
  const fail = () => {
    if (isStopped) return;
    isStopped = true;
    try { appendEvent(join(directory, "events.jsonl"), { sequence: sequence++, type: "recorder_error", message: "Recording failed or exceeded evidence limits" }); } catch {}
  };
  const emit = (event) => {
    if (isStopped) return;
    try { appendEvent(join(directory, "events.jsonl"), { sequence: sequence++, ...event }); } catch { fail(); }
  };
  const capture = () => JSON.parse(execFileSync(process.env.SKILL_EVAL_EXECUTABLE, ["--execution-snapshot", join(directory, "fixture")], { encoding: "utf8", timeout: 10000, maxBuffer: 16 * 1024 * 1024 }));
  const guarded = (action) => {
    if (isStopped) return;
    try { action(); } catch { fail(); }
  };
  const manager = () => globalThis[Symbol.for("pi-subagents:manager")];
  const settle = (id) => {
    if (isStopped || settled.has(id)) return;
    const record = manager()?.getRecord(id);
    if (!record || !record.sessionFile) throw new Error(`Missing child session for ${id}`);
    const sessionCopy = join(directory, "children", `${settled.size}.jsonl`);
    copySession(record.sessionFile, sessionCopy);
    emit({ type: "worker_settled", id, toolCallId: record.toolCallId, status: record.status, sessionFile: record.sessionFile, sessionCopy });
    settled.add(id);
  };
  pi.on("session_start", (_event, ctx) => {
    if (globalThis[key]) return;
    globalThis[key] = true;
    isRoot = true;
    mkdirSync(join(directory, "children"), { recursive: true });
    emit({ type: "recorder_started", sessionId: ctx.sessionManager.getSessionId() });
  });
  for (const name of ["tool_execution_start", "tool_execution_end"]) {
    pi.on(name, (event) => {
      if (isRoot) guarded(() => emit({ ...event, snapshot: capture() }));
    });
  }
  pi.events.on("subagents:started", (event) => {
    if (!isRoot) return;
    guarded(() => {
      if (workers.has(event.id)) throw new Error("Resumed worker is unsupported");
      workers.add(event.id);
      emit({ type: "worker_started", id: event.id, workerType: event.type, description: event.description, snapshot: capture() });
    });
  });
  for (const name of ["subagents:completed", "subagents:failed"]) {
    pi.events.on(name, (event) => {
      if (isRoot) guarded(() => settle(event.id));
    });
  }
  pi.on("session_shutdown", async () => {
    if (!isRoot || isStopped) return;
    try {
      await manager()?.waitForAll();
      for (const id of workers) settle(id);
      emit({ type: "recorder_complete", workers: [...workers] });
    } catch {
      fail();
    }
  });
}
