#!/bin/bash
# gepa-due trigger — fired daily at 3pm by
# workflows/gepa-due/launchd/com.owaisquadri.gepa-due.plist (and once manually to
# test). Accumulation-triggered, not time-triggered, despite the daily cadence: the
# clock only decides WHEN to check, never whether to act. tools/gepa-due (a zero-LLM
# Rust check — see its own doc comment) decides that, every single day, and this
# script only escalates to opening Pi sessions when it prints a non-empty due list.
# On a day nothing crosses the threshold, this exits after the free check — no herdr
# call, no worktree, no Pi invocation, no cost.
#
# Never drives a GUI Terminal (open -a Terminal / osascript) — herdr runs as a
# persistent background daemon reachable over its own socket, so every herdr command
# below is a plain CLI call, mirroring workflows/scheduled-ideation/scripts/trigger.sh's
# own proven mechanism (its own comment documents why the launchd-vs-TCC(Transparency,
# Consent, and Control) unattended-Automation risk does not apply here).
#
# Up to MAX_CONCURRENT due artifacts get their own worktree + live Pi session, run in
# parallel (not one session handed the whole due list serially) — every session is
# scoped to exactly one artifact so its worktree, its kickoff prompt, and its
# post-settle rotation stay independent. Artifacts due beyond the cap are named and
# left for the next fire; never silently dropped.
#
# `logs/` and `votes/` are gitignored, so a fresh git worktree never inherits them —
# git worktrees only share committed history, and hooks/post-checkout explicitly only
# copies untracked NON-ignored files. Each per-artifact worktree gets that artifact's
# real logs/votes copied in by this script before the kickoff prompt fires, so the
# session reads real evidence instead of extrapolating from the checker's own count
# summary (a fabrication bug this exact mechanism ran into and had to be fixed for).
#
# Once a session commits a TUNING.md entry for its artifact (evidence it actually
# reviewed the evidence, mutation or not), this script rotates that artifact's
# logs/usage.jsonl and votes/votes.jsonl in the MAIN checkout to a dated
# `.reviewed-<stamp>` sibling — a verified move, never a delete, per this repo's own
# "never rm before a verified move" rule — so gepa-due stops re-firing on evidence
# that has already been read and recorded, while the reviewed evidence itself stays on
# disk for anyone who wants to look.
#
# Every session is instructed to push its branch and open a PR itself before
# finishing (never merge — that stays a human Decide call) so the review actually
# lands somewhere visible instead of sitting silent in a local worktree; this script
# verifies that happened afterward and WARNs loudly if it didn't.
#
# A usage-only, ZERO-vote due reason gets a DIFFERENT kickoff than a real Reflect: with
# no judge signal on file, a live Reflect pass has nothing to act on and — confirmed
# live, repeatedly, 2026-08-29 — reliably just re-derives "no mutation" from the same
# usage lines every time. So a zero-vote fire dispatches JUDGE_SAMPLE_SIZE fresh-context
# sub-agents (real blind judging, per SKILL.md's judge protocol) against a sample of
# recent usage lines FIRST, to generate real votes for next time, before Reflect runs.
set -uo pipefail

REPO="${GEPA_DUE_REPO:-/Users/owaisquadri/Documents/agents}"
HERDR="${HERDR_BIN:-/opt/homebrew/bin/herdr}"
GEPA_DUE_BASE="${GEPA_DUE_BASE:-main}"
MAX_CONCURRENT="${GEPA_DUE_MAX_CONCURRENT:-3}"
# when an artifact is due on usage_count alone with ZERO votes on file, there is no
# judged critique to Reflect against — every real run so far (2026-08-29) confirmed
# this: 100% of usage-only, zero-vote fires concluded "no mutation" with nothing but a
# restated failure taxonomy already implicit in the incumbent's own contract. The real
# missing input is judge signal, not more prose re-reading the same usage lines, so
# the kickoff below dispatches the judge protocol against this many of the most recent
# current-version usage lines instead of asking for a Reflect essay on zero votes.
JUDGE_SAMPLE_SIZE="${GEPA_DUE_JUDGE_SAMPLE:-5}"
# install.sh builds this and symlinks it onto $HOME/.local/bin (already first on this
# plist's own PATH) — same pattern as every other tools/ checker, per its own "8. the
# rust tools" comment. This launchd job's PATH has no cargo, so it must NOT try to
# build here; command -v finds the symlink once install.sh has run, falling back to
# the raw build path only for a manual run against a checkout install.sh hasn't touched.
GEPA_DUE_BIN="$(command -v gepa-due || echo "$REPO/tools/gepa-due/target/release/gepa-due")"
PRUNE_AFTER_DAYS=7
POLL_TAB_TIMEOUT_S=30
POLL_TAB_INTERVAL_S=2
RUN_STAMP="$(date +%Y-%m-%d-%H%M%S)"
PROMPT_TIMEOUT_MS=600000

log() { printf '[%s] %s\n' "$(date '+%Y-%m-%d %H:%M:%S%z')" "$1"; }

log "gepa-due trigger starting"

# --- step 0: the ONLY step that runs every single day. Zero LLM cost, zero herdr
#     call. Requires install.sh to have already built+symlinked gepa-due (this
#     launchd job's PATH has no cargo, so it cannot build itself) ---
if [ ! -x "$GEPA_DUE_BIN" ]; then
  log "ERROR: gepa-due binary not found at $GEPA_DUE_BIN — run install.sh first (it builds and symlinks every tools/ checker, including this one)"
  exit 1
fi

DUE_JSON="$("$GEPA_DUE_BIN" "$REPO")"
DUE_COUNT="$(printf '%s' "$DUE_JSON" | jq 'length')"

if [ "$DUE_COUNT" -eq 0 ]; then
  log "nothing due — exiting, no Pi invocation"
  exit 0
fi

log "due: $DUE_COUNT artifact(s) — $(printf '%s' "$DUE_JSON" | jq -c '[.[].artifact]')"

# highest-evidence artifacts first when capping — the cap decides WHICH artifacts run
# this fire, never whether the checker's own report gets trusted.
SELECTED_JSON="$(printf '%s' "$DUE_JSON" | jq -c --argjson n "$MAX_CONCURRENT" 'sort_by(-.usage_count, -.vote_count) | .[0:$n]')"
SELECTED_COUNT="$(printf '%s' "$SELECTED_JSON" | jq 'length')"
if [ "$DUE_COUNT" -gt "$MAX_CONCURRENT" ]; then
  DEFERRED_JSON="$(printf '%s' "$DUE_JSON" | jq -c --argjson n "$MAX_CONCURRENT" 'sort_by(-.usage_count, -.vote_count) | .[$n:] | [.[].artifact]')"
  log "concurrency cap $MAX_CONCURRENT < $DUE_COUNT due — running $SELECTED_COUNT this fire, deferred to next fire: $DEFERRED_JSON"
fi

# --- step 1: herdr daemon must be reachable; a harmless no-op call proves it ---
if ! "$HERDR" worktree list --cwd "$REPO" > /dev/null 2>&1; then
  log "ERROR: herdr daemon unreachable (herdr worktree list failed) — is 'herdr server' running?"
  exit 1
fi

# --- step 2: prune gepa-due/* worktrees older than PRUNE_AFTER_DAYS. branch shape is
#     gepa-due/<artifact-slug>/<stamp> — the stamp is always the LAST path segment. ---
CUTOFF_EPOCH=$(( $(date +%s) - PRUNE_AFTER_DAYS * 86400 ))
"$HERDR" worktree list --cwd "$REPO" 2>/dev/null |
  jq -r '.result.worktrees[] | select(.branch // "" | startswith("gepa-due/")) | [.branch, .path] | @tsv' |
  while IFS=$'\t' read -r branch path; do
    stamp="${branch##*/}"
    stamp_date="${stamp:0:10}"
    stamp_epoch="$(date -j -f '%Y-%m-%d' "$stamp_date" +%s 2>/dev/null || echo 0)"
    if [ "$stamp_epoch" -gt 0 ] && [ "$stamp_epoch" -lt "$CUTOFF_EPOCH" ]; then
      log "pruning stale worktree $branch at $path (older than ${PRUNE_AFTER_DAYS}d)"
      git -C "$REPO" worktree remove --force "$path" 2>&1 | while read -r l; do log "  $l"; done || \
        log "WARN: prune failed for $path, leaving it — next run retries"
    fi
  done

# --- step 3: one artifact's full escalation, run in the background per artifact so
#     up to MAX_CONCURRENT run in parallel. Every log line is prefixed with the
#     artifact name since these interleave. ---
handle_artifact() {
  local artifact="$1" usage_count="$2" vote_count="$3" reason="$4"
  local slug="${artifact//\//-}"
  local branch="gepa-due/$slug/$RUN_STAMP"
  local tag="[$artifact]"

  local create_json
  create_json="$("$HERDR" worktree create --cwd "$REPO" --branch "$branch" --base "$GEPA_DUE_BASE" --label gepa-due --focus 2>&1)"
  local workspace_id
  workspace_id="$(printf '%s' "$create_json" | jq -r '.result.workspace.workspace_id // .result.worktree.open_workspace_id // empty' 2>/dev/null)"
  if [ -z "$workspace_id" ]; then
    log "$tag ERROR: could not determine workspace id from herdr worktree create/open output: $create_json"
    return 1
  fi

  local worktree_path
  worktree_path="$("$HERDR" worktree list --cwd "$REPO" 2>/dev/null | jq -r --arg b "$branch" '.result.worktrees[] | select(.branch == $b) | .path' | head -n1)"
  if [ -z "$worktree_path" ]; then
    log "$tag ERROR: worktree created (workspace=$workspace_id) but its filesystem path could not be resolved"
    return 1
  fi
  log "$tag worktree ready: branch=$branch workspace=$workspace_id path=$worktree_path"

  # copy THIS artifact's real evidence into its own worktree — gitignored, never
  # inherited by a fresh worktree otherwise. Copy, not symlink or move: the source
  # (main checkout) must be untouched until rotation confirms real review happened.
  local copied_any=0
  for kind in logs votes; do
    if [ -d "$REPO/$artifact/$kind" ]; then
      mkdir -p "$worktree_path/$artifact"
      cp -R "$REPO/$artifact/$kind" "$worktree_path/$artifact/$kind"
      copied_any=1
    fi
  done
  if [ "$copied_any" -eq 0 ]; then
    log "$tag WARN: no logs/ or votes/ directory found at $REPO/$artifact — proceeding anyway, but the session will have no real evidence to read"
  else
    log "$tag copied real logs/votes into the worktree"
  fi

  local agent_pane="" elapsed=0
  while [ "$elapsed" -lt "$POLL_TAB_TIMEOUT_S" ]; do
    local tabs_json agent_tab
    tabs_json="$("$HERDR" tab list --workspace "$workspace_id" 2>/dev/null || echo '{}')"
    agent_tab="$(printf '%s' "$tabs_json" | jq -r '.result.tabs[]? | select(.label == "agent") | .tab_id' | head -n1)"
    if [ -n "$agent_tab" ]; then
      local panes_json pane_id has_pi
      panes_json="$("$HERDR" pane list --workspace "$workspace_id" 2>/dev/null || echo '{}')"
      for pane_id in $(printf '%s' "$panes_json" | jq -r --arg tab "$agent_tab" '.result.panes[]? | select(.tab_id == $tab) | .pane_id'); do
        has_pi="$("$HERDR" pane process-info --pane "$pane_id" 2>/dev/null | jq -r '[.result.process_info.foreground_processes[]?.argv0] | any(. == "pi")' 2>/dev/null || echo 'false')"
        if [ "$has_pi" = "true" ]; then
          agent_pane="$pane_id"
          break 2
        fi
      done
    fi
    sleep "$POLL_TAB_INTERVAL_S"
    elapsed=$(( elapsed + POLL_TAB_INTERVAL_S ))
  done

  if [ -z "$agent_pane" ]; then
    log "$tag ERROR: agent tab never came up with a live pi foreground process within ${POLL_TAB_TIMEOUT_S}s"
    return 1
  fi
  log "$tag typing kickoff prompt into pane $agent_pane"

  local kickoff
  if [ "$vote_count" -eq 0 ]; then
    kickoff="tools/gepa-due found this artifact due on usage_count alone ($usage_count, reason: $reason) with ZERO votes on file. Its real $artifact/logs/usage.jsonl has already been copied into THIS worktree at that exact path — read it directly. With zero votes there is no judged critique to Reflect against — writing a long Reflect essay re-reading the same usage lines is NOT the right action here (every real gepa-due run so far that hit this exact zero-vote case concluded 'no mutation' with nothing but the incumbent's own contract restated). The right action: dispatch $JUDGE_SAMPLE_SIZE SEPARATE fresh-context sub-agents (the Agent tool, general-purpose type — NOT your own context, blindness is the whole point per SKILL.md's judge protocol section), one per the $JUDGE_SAMPLE_SIZE most recent lines in $artifact/logs/usage.jsonl. Each sub-agent gets ONLY the artifact's own source file and its one assigned usage line — never this prompt, never the other lines, never prior votes — and grades harshly per the judge protocol, submitting via 'python3 skills/ai-author/scripts/submit_vote.py --artifact <name> --grade <grade>' with the vote's first line being 'prompt_version: <exact value from that usage line>'. After all $JUDGE_SAMPLE_SIZE votes land, THEN run Reflect for real with actual judge signal. If it still concludes no mutation (likely, with only $JUDGE_SAMPLE_SIZE votes), commit ONE short paragraph to $artifact/TUNING.md — how many votes you generated and their grades, and why no mutation — NOT a multi-section essay re-deriving the failure taxonomy from usage lines alone. Never ship a mutation without going through the loop's own Decide gate. Whatever you conclude, commit that entry AND push the branch AND open a PR with 'gh pr create --base main' before finishing (do not merge it — that's a human Decide call) — that PR is how this trigger's evidence review actually lands instead of sitting in a local worktree only. This is a scheduled gepa-due run for exactly this one artifact, started at $RUN_STAMP."
  else
    kickoff="tools/gepa-due found this artifact due for a GEPA tuning look: $artifact (usage_count=$usage_count, vote_count=$vote_count, reason: $reason). Its real $artifact/logs/usage.jsonl and $artifact/votes/votes.jsonl have already been copied into THIS worktree at those exact paths — read them directly, don't rely on this summary alone. Read skills/ai-author/SKILL.md's GEPA loop (Reflect/Propose/Test/Decide/Record) and its 'applying frontier data' section, then decide what to do — run a real tuning pass, or record a short 'no mutation, here is why' note (one paragraph if there is genuinely nothing new to report; longer only if the votes actually surface a new pattern worth naming in full). Never ship a mutation without going through the loop's own Decide gate; this prompt is a nudge, not an instruction to skip it. Whatever you conclude, commit a dated entry to $artifact/TUNING.md AND push the branch AND open a PR with 'gh pr create --base main' before finishing (do not merge it — that's a human Decide call) — that commit is how this trigger knows the evidence was actually reviewed and stops re-firing on it, and the PR is how it actually lands for review. This is a scheduled gepa-due run for exactly this one artifact, started at $RUN_STAMP."
  fi
  "$HERDR" pane send-text "$agent_pane" "$kickoff"

  local submit_confirmed=0
  local attempt
  for attempt in 1 2 3; do
    "$HERDR" agent send-keys "$agent_pane" enter >/dev/null 2>&1
    if "$HERDR" agent wait "$agent_pane" --until working --timeout 8000 >/dev/null 2>&1; then
      submit_confirmed=1
      break
    fi
    log "$tag submit attempt $attempt did not reach working within 8s, retrying the enter key"
  done
  if [ "$submit_confirmed" -ne 1 ]; then
    log "$tag ERROR: agent never transitioned to working after 3 submit attempts — giving up"
    return 1
  fi
  log "$tag working confirmed; waiting for it to settle (idle/done/blocked)"
  if ! "$HERDR" agent wait "$agent_pane" --timeout "$PROMPT_TIMEOUT_MS" >/dev/null 2>&1; then
    log "$tag WARN: agent wait did not settle cleanly within ${PROMPT_TIMEOUT_MS}ms — pane is still open and interactive, check it directly"
    return 1
  fi
  log "$tag SUCCESS: session settled — branch=$branch workspace=$workspace_id"

  # verify the session actually did what the kickoff asked (push + open a PR) — the
  # rotation decision below is independent of this (it only needs the LOCAL commit,
  # so a session that committed but forgot to push still gets its evidence rotated,
  # correctly, since the commit itself is the proof of review) but a session that
  # never pushed leaves its record stuck in a local worktree nobody will see, so this
  # is worth a loud WARN rather than a silent gap.
  if git -C "$worktree_path" rev-parse --abbrev-ref --symbolic-full-name '@{u}' >/dev/null 2>&1; then
    local pr_url
    pr_url="$(gh pr view "$branch" --repo "$(git -C "$worktree_path" remote get-url origin | sed -E 's#.*github.com[:/]##; s#\.git$##')" --json url -q .url 2>/dev/null || true)"
    if [ -n "$pr_url" ]; then
      log "$tag confirmed: branch pushed, PR open at $pr_url"
    else
      log "$tag WARN: branch $branch was pushed but no open PR found for it — session may not have run 'gh pr create'"
    fi
  else
    log "$tag WARN: branch $branch was never pushed (no upstream) — its commit(s) are stuck in the local worktree, nobody will see this without manual intervention"
  fi

  # rotation: only once this branch actually committed a TUNING.md entry for this
  # artifact — real evidence the pass happened, not just that the session ran.
  local changed
  changed="$(git -C "$worktree_path" diff --name-only "$GEPA_DUE_BASE" HEAD -- "$artifact/TUNING.md" 2>/dev/null)"
  if [ -z "$changed" ]; then
    log "$tag no $artifact/TUNING.md commit found on $branch — not rotating logs/votes, next fire will see the same evidence"
    return 0
  fi
  log "$tag confirmed $artifact/TUNING.md was committed — rotating reviewed evidence"
  local kind_path dst
  for kind_path in "logs/usage.jsonl" "votes/votes.jsonl"; do
    local src="$REPO/$artifact/$kind_path"
    if [ -f "$src" ]; then
      dst="${src}.reviewed-${RUN_STAMP}"
      mv "$src" "$dst"
      if [ -f "$dst" ] && [ ! -f "$src" ]; then
        log "$tag rotated $kind_path -> $(basename "$dst") (verified move, not a delete)"
      else
        log "$tag WARN: rotate of $kind_path did not verify cleanly (src=$([ -f "$src" ] && echo present || echo gone), dst=$([ -f "$dst" ] && echo present || echo missing)) — leaving as-is"
      fi
    fi
  done
  return 0
}

PIDS=()
for i in $(seq 0 $(( SELECTED_COUNT - 1 ))); do
  entry="$(printf '%s' "$SELECTED_JSON" | jq -c ".[$i]")"
  artifact="$(printf '%s' "$entry" | jq -r '.artifact')"
  usage_count="$(printf '%s' "$entry" | jq -r '.usage_count')"
  vote_count="$(printf '%s' "$entry" | jq -r '.vote_count')"
  reason="$(printf '%s' "$entry" | jq -r '.reason')"
  handle_artifact "$artifact" "$usage_count" "$vote_count" "$reason" &
  PIDS+=("$!")
done

FAILED=0
for pid in "${PIDS[@]}"; do
  wait "$pid" || FAILED=$(( FAILED + 1 ))
done

if [ "$FAILED" -gt 0 ]; then
  log "gepa-due trigger finished with $FAILED/$SELECTED_COUNT artifact session(s) failed — see per-artifact lines above"
  exit 1
fi
log "gepa-due trigger finished — $SELECTED_COUNT/$SELECTED_COUNT artifact session(s) settled cleanly"
