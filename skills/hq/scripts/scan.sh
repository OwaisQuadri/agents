#!/bin/bash
set -euo pipefail
exec /usr/bin/python3 - "$@" <<'PY'
import glob
import json
import os
import re
import subprocess
import sys
from datetime import datetime

HOME = os.path.expanduser("~")
STATE_DIR = os.environ.get("HQ_STATE", os.path.join(HOME, ".claude", "hq"))
STUCK_SECONDS = 2 * 60 * 60
FAILED_PATTERN = re.compile(r"failed|error", re.IGNORECASE)
SEED_LABELS = ["com.owaisquadri.ollama", "homebrew.mxcl.postgresql@18"]


def now_iso():
    return datetime.now().astimezone().strftime("%Y-%m-%dT%H:%M:%S%z")


def mtime_iso(path):
    return datetime.fromtimestamp(os.path.getmtime(path)).astimezone().strftime("%Y-%m-%dT%H:%M:%S%z")


def parse_iso(text):
    return datetime.strptime(text, "%Y-%m-%dT%H:%M:%S%z")


def read_json(path, fallback):
    try:
        with open(path) as f:
            return json.load(f)
    except (OSError, ValueError):
        return fallback


def git_line(path, *args):
    try:
        result = subprocess.run(
            ["/usr/bin/git", "-C", path, *args],
            capture_output=True, text=True, timeout=10,
        )
    except subprocess.TimeoutExpired:
        return None
    if result.returncode != 0:
        return None
    return result.stdout.strip()


def probe_sessions():
    sessions = []
    for path in sorted(glob.glob(os.path.join(HOME, ".claude", "sessions", "*.json"))):
        entry = read_json(path, None)
        if not entry or "pid" not in entry:
            continue
        try:
            os.kill(entry["pid"], 0)
        except (OSError, ProcessLookupError):
            continue
        sessions.append({
            "pid": entry["pid"],
            "sessionId": entry.get("sessionId"),
            "cwd": entry.get("cwd"),
            "name": entry.get("name"),
            "startedAt": entry.get("startedAt"),
        })
    return sessions


def probe_workspaces():
    workspaces = []
    root = os.path.join(HOME, "conductor", "workspaces")
    if not os.path.isdir(root):
        return workspaces
    for repo in sorted(os.listdir(root)):
        repo_dir = os.path.join(root, repo)
        if not os.path.isdir(repo_dir) or os.path.islink(repo_dir):
            continue
        for workspace in sorted(os.listdir(repo_dir)):
            path = os.path.join(repo_dir, workspace)
            if not os.path.isdir(path) or os.path.islink(path):
                continue
            slug = path.replace("/", "--")
            touched_source = os.path.join(HOME, ".conductor", "projects", slug)
            if not os.path.exists(touched_source):
                touched_source = path
            workspaces.append({
                "repo": repo,
                "workspace": workspace,
                "path": path,
                "branch": git_line(path, "rev-parse", "--abbrev-ref", "HEAD"),
                "headSha": git_line(path, "rev-parse", "--short", "HEAD"),
                "lastTouchedAt": mtime_iso(touched_source),
            })
    return workspaces


def probe_jobs():
    jobs = []
    for state_path in sorted(glob.glob(os.path.join(HOME, ".claude", "jobs", "*", "state.json"))):
        entry = read_json(state_path, None)
        if entry is None:
            continue
        timeline_path = os.path.join(os.path.dirname(state_path), "timeline.jsonl")
        jobs.append({
            "id": os.path.basename(os.path.dirname(state_path)),
            "state": entry.get("state"),
            "detail": entry.get("detail"),
            "stateMtime": mtime_iso(state_path),
            "timelineMtime": mtime_iso(timeline_path) if os.path.exists(timeline_path) else None,
        })
    return jobs


def probe_daemon():
    roster = read_json(os.path.join(HOME, ".claude", "daemon", "roster.json"), {})
    return len(roster.get("workers", {}) or {})


def probe_launchd():
    entries = []
    labels_path = os.path.join(STATE_DIR, "watched-jobs.txt")
    if not os.path.exists(labels_path):
        return entries
    with open(labels_path) as f:
        labels = [line.strip() for line in f if line.strip()]
    for label in labels:
        try:
            result = subprocess.run(
                ["/bin/launchctl", "list", label],
                capture_output=True, text=True, timeout=10,
            )
        except subprocess.TimeoutExpired:
            result = None
        if result is None or result.returncode != 0:
            entries.append({"label": label, "isRunning": False, "pid": None, "lastExit": None})
            continue
        pid_match = re.search(r'"PID"\s*=\s*(\d+)', result.stdout)
        exit_match = re.search(r'"LastExitStatus"\s*=\s*(\d+)', result.stdout)
        entries.append({
            "label": label,
            "isRunning": pid_match is not None,
            "pid": int(pid_match.group(1)) if pid_match else None,
            "lastExit": int(exit_match.group(1)) if exit_match else None,
        })
    return entries


def build_snapshot():
    return {
        "at": now_iso(),
        "sessions": probe_sessions(),
        "workspaces": probe_workspaces(),
        "jobs": probe_jobs(),
        "daemonWorkerCount": probe_daemon(),
        "launchd": probe_launchd(),
    }


def is_stuck(job, snapshot_at):
    if job.get("state") != "running" or not job.get("timelineMtime"):
        return False
    age = parse_iso(snapshot_at) - parse_iso(job["timelineMtime"])
    return age.total_seconds() > STUCK_SECONDS


def classify(prev, curr):
    routine = []
    anomalies = []
    at = curr["at"]

    def change(kind, subject, before, after, detail):
        return {"at": at, "kind": kind, "subject": subject, "before": before, "after": after, "detail": detail}

    prev_sessions = {s["sessionId"]: s for s in prev.get("sessions", [])}
    curr_sessions = {s["sessionId"]: s for s in curr.get("sessions", [])}
    for sid, s in curr_sessions.items():
        if sid not in prev_sessions:
            routine.append(change("session_started", s.get("name") or sid, None, s.get("cwd"), "agent session started"))
    for sid, s in prev_sessions.items():
        if sid not in curr_sessions:
            routine.append(change("session_ended", s.get("name") or sid, s.get("cwd"), None, "agent session ended"))

    prev_workspaces = {w["path"]: w for w in prev.get("workspaces", [])}
    curr_workspaces = {w["path"]: w for w in curr.get("workspaces", [])}
    for path, w in curr_workspaces.items():
        old = prev_workspaces.get(path)
        subject = f"{w['repo']}/{w['workspace']}"
        if old is None:
            routine.append(change("workspace_updated", subject, None, w.get("headSha"), "new workspace"))
        elif old.get("headSha") != w.get("headSha"):
            routine.append(change("workspace_updated", subject, old.get("headSha"), w.get("headSha"), f"head moved on {w.get('branch')}"))
        elif old.get("lastTouchedAt") != w.get("lastTouchedAt"):
            routine.append(change("workspace_updated", subject, old.get("lastTouchedAt"), w.get("lastTouchedAt"), "workspace touched"))
    for path, w in prev_workspaces.items():
        if path not in curr_workspaces:
            routine.append(change("workspace_updated", f"{w['repo']}/{w['workspace']}", w.get("headSha"), None, "workspace removed"))

    prev_jobs = {j["id"]: j for j in prev.get("jobs", [])}
    curr_jobs = {j["id"]: j for j in curr.get("jobs", [])}
    for jid, j in curr_jobs.items():
        old = prev_jobs.get(jid)
        old_state = old.get("state") if old else None
        if old_state != j.get("state"):
            entry = change("job_state_changed", jid, old_state, j.get("state"), j.get("detail") or "")
            if j.get("state") and FAILED_PATTERN.search(j["state"]):
                anomalies.append(entry)
            else:
                routine.append(entry)
        was_stuck = old is not None and is_stuck(old, prev.get("at", at))
        if is_stuck(j, at) and not was_stuck:
            anomalies.append(change("job_stuck", jid, old.get("timelineMtime") if old else None, j.get("timelineMtime"), "running with no timeline progress for over 2h"))

    prev_launchd = {e["label"]: e for e in prev.get("launchd", [])}
    curr_launchd = {e["label"]: e for e in curr.get("launchd", [])}
    for label, e in curr_launchd.items():
        old = prev_launchd.get(label)
        if old is None:
            continue
        if old.get("isRunning") and not e.get("isRunning"):
            anomalies.append(change("launchd_down", label, old.get("pid"), None, "launchd job no longer running"))
        elif old.get("lastExit") in (0, None) and e.get("lastExit") not in (0, None):
            anomalies.append(change("launchd_flapped", label, old.get("lastExit"), e.get("lastExit"), "launchd job exited nonzero"))
        elif not old.get("isRunning") and e.get("isRunning"):
            routine.append(change("job_state_changed", label, "down", "running", "launchd job back up"))

    if prev.get("daemonWorkerCount", 0) != curr.get("daemonWorkerCount", 0):
        routine.append(change("job_state_changed", "daemon", prev.get("daemonWorkerCount", 0), curr.get("daemonWorkerCount", 0), "daemon worker count changed"))

    return {"routine": routine, "anomalies": anomalies}


def update_registry(sessions):
    registry_path = os.path.join(STATE_DIR, "registry.json")
    registry = read_json(registry_path, {"updatedAt": None, "agents": []})
    agents = {a["sessionId"]: a for a in registry.get("agents", []) if a.get("sessionId")}
    stamp = now_iso()
    live_ids = set()
    workspace_root = os.path.join(HOME, "conductor", "workspaces") + os.sep
    for s in sessions:
        sid = s.get("sessionId")
        if not sid:
            continue
        live_ids.add(sid)
        cwd = s.get("cwd") or ""
        repo = workspace = None
        if cwd.startswith(workspace_root):
            parts = cwd[len(workspace_root):].split(os.sep)
            if len(parts) >= 2:
                repo, workspace = parts[0], parts[1]
        name = s.get("name") or workspace or os.path.basename(cwd) or sid
        slug = "-" + cwd.replace("/", "-").lstrip("-")
        found = glob.glob(os.path.join(HOME, ".claude", "projects", "*", f"{sid}.jsonl"))
        if not found:
            siblings = glob.glob(os.path.join(HOME, ".claude", "projects", slug, "*.jsonl"))
            if siblings:
                found = [max(siblings, key=os.path.getmtime)]
        agents[sid] = {
            "name": name,
            "repo": repo,
            "workspace": workspace,
            "cwd": cwd,
            "sessionId": sid,
            "pid": s.get("pid"),
            "isLive": True,
            "transcript": found[0] if found else os.path.join(HOME, ".claude", "projects", slug, f"{sid}.jsonl"),
            "lastSeenAt": stamp,
        }
    for sid, a in agents.items():
        if sid not in live_ids:
            a["isLive"] = False
    with open(registry_path, "w") as f:
        json.dump({"updatedAt": stamp, "agents": sorted(agents.values(), key=lambda a: a["name"])}, f, indent=1)


def update_state(**fields):
    state_path = os.path.join(STATE_DIR, "state.json")
    state = read_json(state_path, {})
    state.update(fields)
    with open(state_path, "w") as f:
        json.dump(state, f, indent=1)


def run_classify_mode(prev_arg, curr_arg):
    with open(curr_arg) as f:
        curr = json.load(f)
    if prev_arg == "-":
        return
    with open(prev_arg) as f:
        prev = json.load(f)
    result = classify(prev, curr)
    if result["routine"] or result["anomalies"]:
        print(json.dumps(result, indent=1))


def run_scan():
    os.makedirs(os.path.join(STATE_DIR, "snapshots"), exist_ok=True)
    os.makedirs(os.path.join(STATE_DIR, "gates", "resolved"), exist_ok=True)
    labels_path = os.path.join(STATE_DIR, "watched-jobs.txt")
    if not os.path.exists(labels_path):
        with open(labels_path, "w") as f:
            f.write("\n".join(SEED_LABELS) + "\n")

    snapshot = build_snapshot()
    update_registry(snapshot["sessions"])

    latest_path = os.path.join(STATE_DIR, "snapshots", "latest.json")
    previous_path = os.path.join(STATE_DIR, "snapshots", "previous.json")
    prev = None
    if os.path.exists(latest_path):
        prev = read_json(latest_path, None)
        os.replace(latest_path, previous_path)
        if not os.path.exists(previous_path):
            print("snapshot rotation failed", file=sys.stderr)
            sys.exit(1)
    with open(latest_path, "w") as f:
        json.dump(snapshot, f, indent=1)

    update_state(lastScanAt=snapshot["at"])
    if prev is None:
        return

    result = classify(prev, snapshot)
    if result["routine"]:
        with open(os.path.join(STATE_DIR, "activity.jsonl"), "a") as f:
            for entry in result["routine"]:
                f.write(json.dumps(entry) + "\n")
    if result["anomalies"]:
        with open(os.path.join(STATE_DIR, "delta.json"), "w") as f:
            json.dump({"at": snapshot["at"], "changes": result["anomalies"]}, f, indent=1)
        print("delta")


args = sys.argv[1:]
if args and args[0] == "--classify":
    if len(args) != 3:
        print("usage: scan.sh --classify <prev.json|-> <curr.json>", file=sys.stderr)
        sys.exit(2)
    run_classify_mode(args[1], args[2])
else:
    run_scan()
PY
