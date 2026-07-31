#!/usr/bin/env python3
"""Append-only blind vote submission for ai-author judges.

Reads the open-ended vote text from stdin and appends one JSON line to the
artifact's votes/votes.jsonl. Blindness by construction: the file is opened in
append mode only; this script never reads, prints, or returns existing votes.

Usage:
    echo "<vote text>" | submit_vote.py --artifact <name-or-path> --grade <letter-or-score>
"""
import argparse
import json
import sys
import time
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[3]  # scripts/ -> ai-author/ -> skills/ -> repo
SEARCH_DIRS = ("skills", "agents", "workflows")


def artifact_dir(name: str) -> Path:
    p = Path(name).expanduser()
    if p.is_dir():
        return p
    for d in SEARCH_DIRS:
        candidate = REPO_ROOT / d / name
        if candidate.is_dir():
            return candidate
    sys.exit(f"artifact not found: {name} (looked in {', '.join(SEARCH_DIRS)}/ under {REPO_ROOT})")


def main() -> None:
    ap = argparse.ArgumentParser(description="Submit one blind judge vote (append-only).")
    ap.add_argument("--artifact", required=True, help="artifact name or path to its directory")
    ap.add_argument("--grade", required=True, help="letter grade or 0-10 score")
    args = ap.parse_args()

    vote = sys.stdin.read().strip()
    if not vote:
        sys.exit("empty vote: pipe the open-ended vote text via stdin")

    votes = artifact_dir(args.artifact) / "votes"
    votes.mkdir(parents=True, exist_ok=True)
    line = json.dumps({
        "ts": time.strftime("%Y-%m-%dT%H:%M:%S%z", time.localtime()),
        "artifact": args.artifact,
        "grade": args.grade,
        "vote": vote,
    })
    with open(votes / "votes.jsonl", "a", encoding="utf-8") as f:
        f.write(line + "\n")
    print("vote recorded")


if __name__ == "__main__":
    main()
