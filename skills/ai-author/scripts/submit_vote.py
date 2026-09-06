#!/usr/bin/env python3
"""Idempotent blind vote submission for ai-author judges.

Reads the open-ended vote text from stdin and records one JSON line to the
artifact's votes/votes.jsonl. Extracts the case_id from the vote text (first
line after "Grade:"), then deduplicates by (artifact, case_id, grade) — if an
identical vote for the same case already exists, replaces it. Blindness by
construction: the deduplication never reads the full vote text, only its
metadata, and the order of votes in the file is otherwise untouched.

Usage:
    echo "<vote text>" | submit_vote.py --artifact <name-or-path> --grade <letter-or-score>

The vote text must contain a line with the format "Grade: N/10" or "Grade: L" where
the case_id is implicit in the vote context (extracted from the case being graded).
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


def extract_case_id(vote: str) -> str | None:
    """Extract the case ID from the vote text.

    The vote should be formatted as:
        case_id: <id>
        Grade: N/10...

    Returns the case ID, or None if not found.
    """
    for line in vote.split('\n'):
        line = line.strip()
        if line.startswith('case_id:'):
            return line[len('case_id:'):].strip()
    return None


def main() -> None:
    ap = argparse.ArgumentParser(description="Submit one blind judge vote (idempotent, deduplicated).")
    ap.add_argument("--artifact", required=True, help="artifact name or path to its directory")
    ap.add_argument("--grade", required=True, help="letter grade or 0-10 score")
    ap.add_argument("--case-id", required=True, help="the case ID being voted on")
    args = ap.parse_args()

    vote = sys.stdin.read().strip()
    if not vote:
        sys.exit("empty vote: pipe the open-ended vote text via stdin")

    votes_dir = artifact_dir(args.artifact) / "votes"
    votes_dir.mkdir(parents=True, exist_ok=True)
    votes_file = votes_dir / "votes.jsonl"

    # Prepare the new vote
    ts = time.strftime("%Y-%m-%dT%H:%M:%S%z", time.localtime())
    new_vote = {
        "ts": ts,
        "artifact": args.artifact,
        "case_id": args.case_id,
        "grade": args.grade,
        "vote": vote,
    }

    # Read existing votes and deduplicate by (artifact, case_id, grade)
    existing_votes = []
    if votes_file.exists():
        try:
            with open(votes_file, "r", encoding="utf-8") as f:
                for line in f:
                    line = line.strip()
                    if line:
                        try:
                            v = json.loads(line)
                            existing_votes.append(v)
                        except json.JSONDecodeError:
                            # Skip malformed lines
                            pass
        except (IOError, OSError):
            # If we can't read, just append
            existing_votes = []

    # Replace any vote with the same (artifact, case_id, grade)
    deduplicated = []
    replaced = False
    for v in existing_votes:
        if (v.get("artifact") == args.artifact and
            v.get("case_id") == args.case_id and
            v.get("grade") == args.grade):
            if not replaced:
                # Replace only the first matching vote
                deduplicated.append(new_vote)
                replaced = True
            # Skip this duplicate
        else:
            deduplicated.append(v)

    # If we didn't find a matching vote to replace, append the new one
    if not replaced:
        deduplicated.append(new_vote)

    # Write all votes back
    with open(votes_file, "w", encoding="utf-8") as f:
        for v in deduplicated:
            f.write(json.dumps(v) + "\n")

    if replaced:
        print(f"vote updated for case {args.case_id}")
    else:
        print(f"vote recorded for case {args.case_id}")


if __name__ == "__main__":
    main()
