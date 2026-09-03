#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"

# Default (no --holdout): grades BOTH slices in one pass and writes one merged
# evals/frontier.jsonl record PER TIER TESTED (candidate_id, tier, judge_tier,
# model_ran, both slices' scores, ts) -- per templates/eval-harness.md's
# "frontier.jsonl" section. --holdout keeps a fast, frontier-write-free single-slice
# recheck for a quick look at the holdout slice alone.
#
# EXECUTION ARM (tools/tier-dispatch): this harness used to send the skill's PROSE
# TEXT to a judge and ask whether an agent following it WOULD meet EXPECT -- every
# tier scored identically, since no tier ever actually ran the skill. It now
# dispatches a REAL run of the skill (via tools/tier-dispatch, tier's own primary
# model with same-tier fallback-on-quota) and grades the ARTIFACT that run produced.
# Every available tier is tested every run (exhaustive, per the owner's override of
# the earlier walk-up-from-cheapest design -- see the comment on issue #64), with 3
# repeats per case per tier (REPEATS below) since a single pass/fail is a sample of
# one. A tier whose own fallback chain is fully exhausted on quota is skipped for
# this run entirely -- no frontier line is written for it, never a guessed score.
holdout_only=0
single_tier=""
while [[ "${1:-}" == "--holdout" || "${1:-}" == "--tier" ]]; do
  case "$1" in
    --holdout) holdout_only=1; shift ;;
    --tier) single_tier="${2:?--tier needs a value, e.g. --tier T3}"; shift 2 ;;
  esac
done
skill="${1:-../SKILL.md}"
# ACCEPTED=true|false, set by the caller once GEPA loop step 4 (Decide) has judged this
# run against the incumbent -- run.sh cannot know acceptance at grading time, so it
# defaults to false (nothing ships without an explicit Decide anyway).
accepted="${ACCEPTED:-false}"
# REPEATS: repeat trials per case per tier. 3 is the default per #64/#69's own
# "single pass is a sample of one" language; a smoke run can override this cheaply.
repeats="${REPEATS:-3}"
# CASES_FILE: override for a scoped smoke run without touching the real cases.jsonl.
cases_file="${CASES_FILE:-cases.jsonl}"
tier_dispatch_bin="${TIER_DISPATCH_BIN:-../../../tools/tier-dispatch/target/debug/tier-dispatch}"
tiers_file="${TIERS_FILE:-../../../config/model-tiers.json}"

T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
mkdir -p "$T/fake-artifact/votes"
printf '%s\n' '{"ts":"x","artifact":"fake","grade":"9","vote":"SENTINEL-PRIOR-VOTE"}' \
  > "$T/fake-artifact/votes/votes.jsonl"
out=$(echo "second vote" | python3 ../scripts/submit_vote.py --artifact "$T/fake-artifact" --grade 7)
[[ "$out" == "vote recorded" ]] || { echo "smoke: submit_vote did not confirm" >&2; exit 1; }
if grep -q "SENTINEL-PRIOR-VOTE" <<<"$out"; then
  echo "smoke: submit_vote leaked a prior vote to its caller" >&2; exit 1
fi
[[ "$(wc -l < "$T/fake-artifact/votes/votes.jsonl")" -eq 2 ]] || { echo "smoke: votes not append-only" >&2; exit 1; }
head -1 "$T/fake-artifact/votes/votes.jsonl" | grep -q "SENTINEL-PRIOR-VOTE" || { echo "smoke: prior vote rewritten" >&2; exit 1; }
python3 -c 'import json,sys; json.loads(open(sys.argv[1]).readlines()[1])' "$T/fake-artifact/votes/votes.jsonl" \
  || { echo "smoke: appended vote is not valid JSON" >&2; exit 1; }
if echo "" | python3 ../scripts/submit_vote.py --artifact "$T/fake-artifact" --grade 7 >/dev/null 2>&1; then
  echo "smoke: empty vote not rejected" >&2; exit 1
fi
echo "smoke: scripts pass" >&2

command -v "$tier_dispatch_bin" >/dev/null 2>&1 || [[ -x "$tier_dispatch_bin" ]] || {
  echo "tier-dispatch binary not found or not executable at $tier_dispatch_bin (build with: cd tools/tier-dispatch && cargo build)" >&2
  exit 1
}

python3 - "$skill" "$holdout_only" "$accepted" "$repeats" "$cases_file" "$tier_dispatch_bin" "$tiers_file" "$single_tier" <<'PY'
import hashlib
import json
import os
import statistics
import subprocess
import sys
import tempfile

(skill_path, holdout_only, accepted, repeats, cases_file,
 tier_dispatch_bin, tiers_file, single_tier) = sys.argv[1:9]
holdout_only = holdout_only == "1"
accepted = accepted == "true"
repeats = int(repeats)

skill_dir = os.path.abspath("..")
skill = open(skill_path, encoding="utf-8").read()
rubric = open("rubric.md", encoding="utf-8").read()
all_cases = [json.loads(line) for line in open(cases_file, encoding="utf-8") if line.strip()]

with open(tiers_file, encoding="utf-8") as f:
    tiers_config = json.load(f)
tier_names = sorted(tiers_config["tiers"].keys())  # T1..T5, lexicographic == numeric here
if single_tier:
    # --tier <T3> restricts the sweep to exactly that one tier -- a cheap iteration
    # escape hatch while authoring, NEVER the default: exhaustive-all-tiers stays the
    # no-flag behavior per the owner's override of the earlier walk-up-and-stop design.
    if single_tier not in tier_names:
        sys.exit(f"--tier {single_tier!r} is not a known tier; known tiers: {tier_names}")
    tier_names = [t for t in tier_names if t == single_tier]
    # judge_tier_for still needs the FULL tier order to find "one tier up" correctly --
    # rebuilt separately below rather than derived from this now-narrowed list, so
    # --tier T3 still gets judged by T4 and not treated as though T3 were the top tier.
    full_tier_order = sorted(tiers_config["tiers"].keys())
else:
    full_tier_order = tier_names

def judge_tier_for(tier):
    """One tier up grades the tier under test; the top tier grades itself -- no T6
    exists, and this harness names that limitation rather than hiding it. Always
    resolved against full_tier_order, never the (possibly --tier-narrowed) tier_names
    -- a --tier T3 single-tier run still gets judged by T4, never by itself, just
    because T3 happens to be the only tier in this run's own sweep list."""
    idx = full_tier_order.index(tier)
    return full_tier_order[idx + 1] if idx + 1 < len(full_tier_order) else tier

PASS_THRESHOLD = 5  # rubric.md: 5-8 = "expect met with drift or noise" -- the floor for "met at all"

_temp_prompt_files = []

def write_prompt_file(text):
    """A case whose cases.jsonl entry carries a `files` list gets the skill's own text
    PLUS those files' content appended, exactly as the pre-execution-arm harness did
    when it built the text it judged (a case's EXPECT can rely on that supporting
    material being present -- 3 of this artifact's current cases do). Each such case
    gets its own temp prompt file rather than reusing the shared skill_prompt_file,
    since the appended content differs per case."""
    f = tempfile.NamedTemporaryFile(mode="w", suffix=".md", delete=False, encoding="utf-8")
    f.write(text)
    f.close()
    _temp_prompt_files.append(f.name)
    return f.name

skill_prompt_file = tempfile.NamedTemporaryFile(mode="w", suffix=".md", delete=False, encoding="utf-8")
skill_prompt_file.write(skill)
skill_prompt_file.close()
empty_prompt_file = tempfile.NamedTemporaryFile(mode="w", suffix=".md", delete=False, encoding="utf-8")
empty_prompt_file.write("")
empty_prompt_file.close()

class FatalDispatchError(Exception):
    """Raised on tier-dispatch exit 2 (per tools/tier-dispatch/src/main.rs's own
    documented contract: 'unknown tier or missing required argument') -- a config or
    usage bug that will fail identically on EVERY subsequent call this run, never a
    per-case or per-model condition. Left uncaught deliberately: it aborts the whole
    script before the frontier-write loop at the bottom ever runs, so a bad
    --tiers-file or a typo'd tier name can never silently turn into a frontier.jsonl
    line full of real-looking zero scores (a Critical finding from this change's own
    code review -- exit 2 used to be folded into the same path as a real dispatch
    failure and silently scored 0 for every repeat, every case, every tier)."""

def run_tier_dispatch(tier, system_prompt_path, input_text):
    """Returns (status, artifact_or_None, model_ran_or_None) where status is one of
    'ok', 'exhausted' (tier's fallback chain used up, quota-classified), or
    'hard_failure' (a real, non-quota, per-dispatch failure -- e.g. the pi CLI crashed
    on this specific input). Raises FatalDispatchError directly on exit 2, a config/
    usage error rather than anything about this one dispatch."""
    result = subprocess.run(
        [tier_dispatch_bin, "--tiers-file", tiers_file, "--tier", tier,
         "--system-prompt-file", system_prompt_path, "--input", input_text],
        capture_output=True, text=True,
    )
    if result.returncode == 0:
        model_ran = None
        for line in result.stderr.splitlines():
            if line.startswith("model_ran: "):
                model_ran = line[len("model_ran: "):]
        if model_ran is None:
            print(f"  tier-dispatch on tier {tier} exited 0 but printed no 'model_ran:' line -- treating as a hard failure rather than trusting an unattributed success", file=sys.stderr)
            return "hard_failure", None, None
        return "ok", result.stdout, model_ran
    if result.returncode == 3:
        return "exhausted", None, None
    if result.returncode == 2:
        raise FatalDispatchError(
            f"tier-dispatch config/usage error (exit 2) on tier {tier}: {result.stderr.strip()[:300]} -- "
            "this will fail identically on every remaining call, aborting rather than scoring every repeat 0"
        )
    print(f"  tier-dispatch failed (exit {result.returncode}) on tier {tier}: {result.stderr.strip()[:300]}", file=sys.stderr)
    return "hard_failure", None, None

class TierExhaustedError(Exception):
    """Raised the moment ANY dispatch at a tier -- the artifact under test, OR its judge
    one tier up -- hits a fully-exhausted fallback chain. Caught once per tier, at the
    level that decides whether to write a frontier line, so a tier is skipped for the
    WHOLE run (both slices) the instant either role can't run, never scored as if the
    judge's own unavailability were an artifact quality failure."""
    def __init__(self, tier, reason):
        self.tier = tier
        self.reason = reason

def judge(judge_tier, case, artifact):
    prompt = (
        "Grade one eval case for a skill's ACTUAL OUTPUT from a real dispatched run "
        '(not its prose text). Reply with ONLY a JSON object '
        '{"score": <integer 0-10>, "failure_mode": "<short tag>" or null}.\n\n'
        "RUBRIC:\n" + rubric +
        "\nCASE INPUT:\n" + case["input"] +
        "\n\nEXPECT:\n" + case["expect"] +
        "\n\nTHE SKILL'S ACTUAL OUTPUT ON THIS INPUT:\n" + artifact +
        "\n\nDoes this output meet EXPECT? Grade only what the output actually says or does."
    )
    status, out, _model = run_tier_dispatch(judge_tier, empty_prompt_file.name, prompt)
    if status == "exhausted":
        raise TierExhaustedError(judge_tier, "judge tier's own fallback chain exhausted")
    if status != "ok" or not out:
        return None, "judge-dispatch-failed"
    start, end = out.find("{"), out.rfind("}")
    if start == -1:
        return None, "judge-no-json"
    try:
        verdict = json.loads(out[start:end + 1])
        return verdict["score"], verdict.get("failure_mode")
    except (json.JSONDecodeError, KeyError):
        return None, "judge-bad-json"

def run_tier(tier, cases, slice_name):
    """Runs every case `repeats` times at `tier`. Raises TierExhaustedError if the
    tier's OWN fallback chain, or its judge tier's, was exhausted at any point --
    caught by the caller, which skips the whole tier for this run (both slices), never
    a guessed score for either the artifact-dispatch or the judge going unavailable.
    Returns (per_case_scores, repeat_scores_by_case, models_seen) otherwise, including
    when `cases` is empty (a tier is never considered exhausted on zero cases alone --
    the caller decides tier inclusion from whether BOTH slices came back clean, not
    from this function's own success on an empty slice)."""
    judge_tier = judge_tier_for(tier)
    per_case_scores = []
    repeat_scores_by_case = {}
    models_seen = set()
    ungraded = 0
    for case in cases:
        repeat_scores = []
        skill_text = skill
        for extra in case.get("files", []):
            path = os.path.join(skill_dir, extra)
            skill_text += "\n\n--- " + extra + " ---\n" + open(path, encoding="utf-8").read()
        case_prompt_path = write_prompt_file(skill_text) if case.get("files") else skill_prompt_file.name
        for _ in range(repeats):
            status, artifact, model_ran = run_tier_dispatch(tier, case_prompt_path, case["input"])
            if status == "exhausted":
                raise TierExhaustedError(tier, "artifact tier's own fallback chain exhausted")
            if status == "hard_failure":
                # A real, non-quota dispatch failure on this one repeat -- recorded as
                # ungraded (None), never folded into the numeric mean as if the
                # artifact itself failed the rubric (a Critical finding from this
                # change's own code review: a config typo used to silently score 0
                # exactly like a genuine rubric failure, indistinguishable in the
                # frontier record from the candidate actually being bad).
                repeat_scores.append(None)
                ungraded += 1
                continue
            models_seen.add(model_ran)
            score, failure_mode = judge(judge_tier, case, artifact)
            repeat_scores.append(score)  # None here means the judge itself failed -- also ungraded, not 0
            if score is None:
                ungraded += 1
        repeat_scores_by_case[case["id"]] = repeat_scores
        graded = [s for s in repeat_scores if s is not None]
        per_case_scores.append(statistics.median(graded) if graded else None)
        print(json.dumps({"id": case["id"], "tier": tier, "repeat_scores": repeat_scores,
                           "median": per_case_scores[-1]}))
    graded_case_scores = [s for s in per_case_scores if s is not None]
    if graded_case_scores:
        mean = sum(graded_case_scores) / len(graded_case_scores)
        verdict = "PASS" if mean >= PASS_THRESHOLD else "FAIL"
        print(f"tier {tier}: mean {mean:.2f} over {len(graded_case_scores)} graded cases, {ungraded} ungraded repeats, {verdict} (>= {PASS_THRESHOLD} threshold) ({slice_name} slice)", file=sys.stderr)
    elif per_case_scores:
        print(f"tier {tier}: every case ungraded, {ungraded} ungraded repeats, FAIL (nothing graded) ({slice_name} slice)", file=sys.stderr)
    return per_case_scores, repeat_scores_by_case, sorted(models_seen)

nonholdout_cases = [c for c in all_cases if not bool(c.get("holdout"))]
holdout_cases = [c for c in all_cases if bool(c.get("holdout"))]

def run_tier_both_slices(tier):
    """Tests ONE tier against BOTH slices together, so an empty slice (zero cases,
    trivially 'not exhausted' since run_tier never dispatches anything for it) can
    never mask an exhaustion the OTHER slice hit for the same tier -- a real bug this
    harness had before both slices were tied to one shared exhaustion check: with only
    a nonholdout case in cases.jsonl, an exhausted tier's empty holdout pass used to
    come back trivially clean and still get a frontier line written for it. Returns
    None (skip this tier for the whole run, no frontier line) the moment EITHER slice
    raises TierExhaustedError, else (nonholdout_result, holdout_result)."""
    try:
        nonholdout_result = run_tier(tier, nonholdout_cases, "nonholdout")
        holdout_result = run_tier(tier, holdout_cases, "holdout")
        return nonholdout_result, holdout_result
    except TierExhaustedError as error:
        print(f"  tier {tier} exhausted ({error.reason}, tier {error.tier}) -- skipping tier {tier} for this run (both slices)", file=sys.stderr)
        return None

if holdout_only:
    for tier in tier_names:
        try:
            run_tier(tier, holdout_cases, "holdout")
        except TierExhaustedError as error:
            print(f"  tier {tier} exhausted ({error.reason}, tier {error.tier})", file=sys.stderr)
    sys.exit(0)

nonholdout_per_tier = {}
holdout_per_tier = {}
for tier in tier_names:
    result = run_tier_both_slices(tier)
    if result is not None:
        nonholdout_per_tier[tier], holdout_per_tier[tier] = result

candidate_id = hashlib.sha1(skill.encode("utf-8")).hexdigest()[:8]
tested_against = subprocess.run(
    ["git", "log", "-1", "--format=%h", "--", "..",
     ":(exclude)**/evals/**", ":(exclude)**/TUNING.md", ":(exclude)**/logs/**", ":(exclude)**/votes/**"],
    capture_output=True, text=True, cwd=skill_dir,
).stdout.strip()
ts = subprocess.run(["date", "+%Y-%m-%dT%H:%M:%S%z"], capture_output=True, text=True).stdout.strip()

os.makedirs("frontier", exist_ok=True)
with open(f"frontier/{candidate_id}.md", "w", encoding="utf-8") as f:
    f.write(skill)

tiers_tested = sorted(set(nonholdout_per_tier) | set(holdout_per_tier))
for tier in tiers_tested:
    scores_nonholdout, repeats_nonholdout, models_nonholdout = nonholdout_per_tier.get(tier, ([], {}, []))
    scores_holdout, repeats_holdout, models_holdout = holdout_per_tier.get(tier, ([], {}, []))
    graded_nonholdout = [s for s in scores_nonholdout if s is not None]
    mean_nonholdout = round(sum(graded_nonholdout) / len(graded_nonholdout), 2) if graded_nonholdout else None
    with open("frontier.jsonl", "a", encoding="utf-8") as f:
        f.write(json.dumps({
            "candidate_id": candidate_id,
            "tested_against": tested_against,
            "tier": tier,
            "judge_tier": judge_tier_for(tier),
            "model_ran": sorted(set(models_nonholdout) | set(models_holdout)),
            "scores_nonholdout": scores_nonholdout,
            "scores_holdout": scores_holdout,
            "repeat_scores_nonholdout": repeats_nonholdout,
            "repeat_scores_holdout": repeats_holdout,
            "mean_nonholdout": mean_nonholdout,
            "accepted": accepted,
            "ts": ts,
        }) + "\n")
    print(f"frontier: recorded candidate {candidate_id} tier {tier} (accepted={accepted})", file=sys.stderr)

skipped = sorted(set(tier_names) - set(tiers_tested))
if skipped:
    print(f"frontier: tiers skipped this run (fallback chain exhausted): {skipped}", file=sys.stderr)

os.unlink(skill_prompt_file.name)
os.unlink(empty_prompt_file.name)
for path in _temp_prompt_files:
    os.unlink(path)
PY
