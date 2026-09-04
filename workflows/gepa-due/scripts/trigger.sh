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
# post-settle state entry stay independent. Artifacts due beyond the cap are named and
# left for the next fire; never silently dropped.
#
# There is no more per-artifact logs/usage.jsonl to copy in — usage evidence is no
# longer self-reported at all (see skills/ai-author/SKILL.md's "usage evidence"
# section): tools/gepa-due and the dispatched session both read the SAME real Pi
# transcripts directly from ~/.pi/agent/sessions/, a machine-global location every
# worktree on this machine can already see without anything being copied. Only
# votes/votes.jsonl is still gitignored-per-artifact and still needs copying in —
# `git worktree` only carries committed history, and hooks/post-checkout explicitly
# only copies untracked NON-ignored files.
#
# "Since last tune" is now a TIME cutoff (max of the artifact's own last-modification
# commit and workflows/gepa-due/state/reviewed.jsonl's reviewed_through for it), not a
# prompt_version hash match — a transcript hit carries no prompt_version field to
# match against. tools/gepa-due computes that cutoff and reports it as cutoff_iso per
# due artifact; this script passes it straight into the kickoff prompt so the
# dispatched session (running in a FRESH worktree that cannot see the gitignored state
# file the cutoff came from) never has to — and never could — re-derive it itself.
#
# Dedup against a still-open PR: before selecting which due artifacts to run this
# fire, this script reads workflows/gepa-due/state/reviewed.jsonl (main checkout,
# gitignored, written only by this script) for each due artifact's MOST RECENT prior
# dispatch, and skips it if that dispatch's PR is still open — no sense opening a
# second worktree to review the same artifact while a prior review sits unmerged.
#
# Every fire whose session reaches a VERDICT (settles: idle/done/blocked) appends one
# line to workflows/gepa-due/state/reviewed.jsonl — gated on REACHING a verdict, never
# on what that verdict was (a real mutation, a no-mutation note, or nothing committed
# at all are all a reviewed conclusion). A session that times out or gets stuck never
# reaches a verdict and gets NO entry — it stays due for tomorrow, same evidence,
# same cutoff. Append-only: never edits or deletes a prior line, mirroring this
# repo's own "never rm before a verified move" spirit — nothing here is destructive.
#
# A usage-only, ZERO-vote due reason gets a DIFFERENT kickoff than a real Reflect: with
# no judge signal on file, a live Reflect pass has nothing to act on and — confirmed
# live, repeatedly, 2026-08-29 — reliably just re-derives "no mutation" from the same
# usage lines every time. So a zero-vote fire dispatches JUDGE_SAMPLE_SIZE fresh-context
# sub-agents (real blind judging, per SKILL.md's judge protocol) against a sample of
# recent REAL transcript hits FIRST, to generate real votes for next time, before
# Reflect runs.
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
# real transcript hits instead of asking for a Reflect essay on zero votes.
JUDGE_SAMPLE_SIZE="${GEPA_DUE_JUDGE_SAMPLE:-5}"
# install.sh builds this and symlinks it onto $HOME/.local/bin (already first on this
# plist's own PATH) — same pattern as every other tools/ checker, per its own "8. the
# rust tools" comment. This launchd job's PATH has no cargo, so it must NOT try to
# build here; command -v finds the symlink once install.sh has run, falling back to
# the raw build path only for a manual run against a checkout install.sh hasn't touched.
GEPA_DUE_BIN="$(command -v gepa-due || echo "$REPO/tools/gepa-due/target/release/gepa-due")"
STATE_FILE="$REPO/workflows/gepa-due/state/reviewed.jsonl"
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

# --- step 0.5: drop any due artifact whose most recent prior dispatch still has an
#     open PR — no dedup value in reviewing it again before that review lands. Reads
#     directly from the STATE_FILE in the main checkout (never copied, never needed
#     to be — this step runs here, not inside a worktree). ---
GH_REPO_SLUG=""
if git -C "$REPO" remote get-url origin >/dev/null 2>&1; then
  GH_REPO_SLUG="$(git -C "$REPO" remote get-url origin | sed -E 's#.*github.com[:/]##; s#\.git$##')"
fi
if [ -f "$STATE_FILE" ] && [ -n "$GH_REPO_SLUG" ]; then
  LATEST_BY_ARTIFACT="$(jq -c -s 'group_by(.artifact) | map(sort_by(.dispatched_at) | last)' "$STATE_FILE" 2>/dev/null || echo '[]')"
  SKIP_ARTIFACTS=()
  while IFS=$'\t' read -r artifact pr_number; do
    [ -z "$artifact" ] && continue
    if [ -z "$pr_number" ] || [ "$pr_number" = "null" ]; then
      continue
    fi
    state="$(gh pr view "$pr_number" --repo "$GH_REPO_SLUG" --json state -q .state 2>/dev/null || echo '')"
    if [ "$state" = "OPEN" ]; then
      log "[$artifact] skipping this fire — prior dispatch PR #$pr_number is still open"
      SKIP_ARTIFACTS+=("$artifact")
    fi
  done < <(printf '%s' "$LATEST_BY_ARTIFACT" | jq -r '.[] | [.artifact, (.pr_number // "")] | @tsv')

  if [ "${#SKIP_ARTIFACTS[@]}" -gt 0 ]; then
    SKIP_JSON="$(printf '%s\n' "${SKIP_ARTIFACTS[@]}" | jq -R . | jq -s .)"
    DUE_JSON="$(printf '%s' "$DUE_JSON" | jq -c --argjson skip "$SKIP_JSON" '[.[] | select(([.artifact] | inside($skip)) | not)]')"
    DUE_COUNT="$(printf '%s' "$DUE_JSON" | jq 'length')"
  fi
fi

if [ "$DUE_COUNT" -eq 0 ]; then
  log "nothing left to dispatch after open-PR dedup — exiting, no Pi invocation"
  exit 0
fi

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
  local artifact="$1" usage_count="$2" vote_count="$3" reason="$4" cutoff_iso="$5"
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
  local base_commit baseline_diff_hash baseline_untracked
  base_commit="$(git -C "$worktree_path" rev-parse HEAD)" || return 1
  baseline_diff_hash="$(git -C "$worktree_path" diff --binary HEAD | git hash-object --stdin)"
  baseline_untracked="$(git -C "$worktree_path" ls-files --others --exclude-standard | LC_ALL=C sort)"

  # copy THIS artifact's real vote history into its own worktree — gitignored, never
  # inherited by a fresh worktree otherwise. Usage evidence needs no copying: real Pi
  # transcripts under ~/.pi/agent/sessions/ are a machine-global path this worktree
  # already sees, same as every other worktree on this machine.
  if [ -d "$REPO/$artifact/votes" ]; then
    mkdir -p "$worktree_path/$artifact"
    cp -R "$REPO/$artifact/votes" "$worktree_path/$artifact/votes"
    log "$tag copied real votes/ into the worktree"
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
    kickoff="tools/gepa-due found this artifact due on usage_count alone ($usage_count real transcript hits since $cutoff_iso, reason: $reason) with ZERO votes on file. There is no logs/usage.jsonl to read — usage evidence is real Pi session transcripts under ~/.pi/agent/sessions/ (machine-global, already visible from this worktree), a read tool_call whose path ends in this artifact's own definition file, with a timestamp strictly after $cutoff_iso. skills/ai-author/SKILL.md's Reflect step documents exactly how to scan and read these. With zero votes there is no judged critique to Reflect against — writing a long Reflect essay re-reading the same usage hits is NOT the right action here (every real gepa-due run so far that hit this exact zero-vote case concluded 'no mutation' with nothing but the incumbent's own contract restated). The right action: dispatch $JUDGE_SAMPLE_SIZE SEPARATE fresh-context sub-agents (the Agent tool, general-purpose type — NOT your own context, blindness is the whole point per SKILL.md's judge protocol section), one per the $JUDGE_SAMPLE_SIZE most recent real transcript hits you find (after $cutoff_iso) for this artifact. Each sub-agent gets ONLY the artifact's own source file and its one assigned transcript excerpt — never this prompt, never the other hits, never prior votes — and grades harshly per the judge protocol, submitting via 'python3 skills/ai-author/scripts/submit_vote.py --artifact <name> --grade <grade>' with the vote's first line being 'prompt_version: <this artifact's current short commit hash>'. After all $JUDGE_SAMPLE_SIZE votes land, THEN run Reflect for real with actual judge signal. If Reflect proposes no candidate, write no tracked note, make no commit, do not push, and do not open a PR; the trigger records the review in machine-local state after this session settles. If Reflect proposes a candidate, run Test and Decide. Whether Decide accepts or rejects it, commit the tracked frontier evidence, push the branch, and open a PR with 'gh pr create --base main'. A rejected candidate must leave the live definition unchanged and the PR must say it preserves test evidence only. An accepted candidate includes the live definition mutation. Never merge the PR. This is a scheduled gepa-due run for exactly this one artifact, started at $RUN_STAMP."
  else
    kickoff="tools/gepa-due found this artifact due for a GEPA tuning look: $artifact (usage_count=$usage_count real transcript hits since $cutoff_iso, vote_count=$vote_count, reason: $reason). There is no logs/usage.jsonl to read — usage evidence is real Pi session transcripts under ~/.pi/agent/sessions/ (machine-global, already visible from this worktree), a read tool_call whose path ends in this artifact's own definition file, with a timestamp strictly after $cutoff_iso. Its real $artifact/votes/votes.jsonl has already been copied into THIS worktree at that exact path — read it directly. Read skills/ai-author/SKILL.md's GEPA loop (Reflect/Propose/Test/Decide) and its 'applying frontier data' section, then decide what to do — run a real tuning pass or stop after Reflect. If Reflect proposes no candidate, write no tracked note, make no commit, do not push, and do not open a PR; the trigger records the review in machine-local state after this session settles. If Reflect proposes a candidate, run Test and Decide. Whether Decide accepts or rejects it, commit the tracked frontier evidence, push the branch, and open a PR with 'gh pr create --base main'. A rejected candidate must leave the live definition unchanged and the PR must say it preserves test evidence only. An accepted candidate includes the live definition mutation. Never merge the PR. This is a scheduled gepa-due run for exactly this one artifact, started at $RUN_STAMP."
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
    log "$tag WARN: agent wait did not settle cleanly within ${PROMPT_TIMEOUT_MS}ms — pane is still open and interactive, check it directly. NOT recording a reviewed-through entry: it never reached a verdict, so re-firing tomorrow is the correct behavior, not a repeat of wasted work."
    return 1
  fi
  log "$tag SUCCESS: session settled — branch=$branch workspace=$workspace_id"

  if [ -f "$worktree_path/$artifact/votes/votes.jsonl" ]; then
    local votes_source="$worktree_path/$artifact/votes/votes.jsonl"
    local votes_destination="$REPO/$artifact/votes/votes.jsonl"
    mkdir -p "$(dirname "$votes_destination")"
    touch "$votes_destination"
    lockf -kw "$votes_destination" /bin/zsh -c 'while IFS= read -r vote; do grep -Fqx -- "$vote" "$2" || printf "%s\n" "$vote" >> "$2"; done < "$1"' zsh "$votes_source" "$votes_destination"
    log "$tag merged updated votes/ back into the main checkout"
  fi

  local settled_diff_hash settled_untracked
  settled_diff_hash="$(git -C "$worktree_path" diff --binary HEAD | git hash-object --stdin)"
  settled_untracked="$(git -C "$worktree_path" ls-files --others --exclude-standard | LC_ALL=C sort)"
  if [ "$settled_diff_hash" != "$baseline_diff_hash" ] || [ "$settled_untracked" != "$baseline_untracked" ]; then
    log "$tag ERROR: session settled with uncommitted work; refusing to record a verdict"
    return 1
  fi

  local pr_number=""
  if git -C "$worktree_path" diff --quiet "$base_commit" HEAD; then
    log "$tag confirmed: Reflect-only review; no push or PR required"
  elif git -C "$worktree_path" rev-parse --abbrev-ref --symbolic-full-name '@{u}' >/dev/null 2>&1; then
    local repo_slug pr_json
    repo_slug="$(git -C "$worktree_path" remote get-url origin | sed -E 's#.*github.com[:/]##; s#\.git$##')"
    pr_json="$(gh pr view "$branch" --repo "$repo_slug" --json url,number 2>/dev/null || true)"
    pr_number="$(printf '%s' "$pr_json" | jq -r '.number // empty' 2>/dev/null)"
    if [ -n "$pr_number" ]; then
      log "$tag confirmed: GEPA result branch pushed, PR #$pr_number open at $(printf '%s' "$pr_json" | jq -r '.url')"
    else
      log "$tag ERROR: GEPA result branch $branch was pushed but no open PR found for it"
      return 1
    fi
  else
    log "$tag ERROR: GEPA result branch $branch was never pushed; its commit is stuck in the local worktree"
    return 1
  fi

  # reviewed-through: gated on reaching a VERDICT (we're past the settle check above
  # without returning early — the session ran to completion: idle/done/blocked, not
  # stuck or timed out), never on the verdict's CONTENT. "No mutation, nothing to
  # commit" is as much a verdict as a real mutation — the session looked at the real
  # evidence and reached a conclusion, so that evidence is reviewed, full stop.
  # Append-only: this NEVER edits or removes a prior line for this or any artifact.
  local now_iso
  now_iso="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  mkdir -p "$(dirname "$STATE_FILE")"
  printf '{"artifact":"%s","reviewed_through":"%s","pr_number":%s,"branch":"%s","dispatched_at":"%s"}\n' \
    "$artifact" "$now_iso" "${pr_number:-null}" "$branch" "$RUN_STAMP" >> "$STATE_FILE"
  log "$tag recorded reviewed_through=$now_iso in $STATE_FILE"
  return 0
}

PIDS=()
for i in $(seq 0 $(( SELECTED_COUNT - 1 ))); do
  entry="$(printf '%s' "$SELECTED_JSON" | jq -c ".[$i]")"
  artifact="$(printf '%s' "$entry" | jq -r '.artifact')"
  usage_count="$(printf '%s' "$entry" | jq -r '.usage_count')"
  vote_count="$(printf '%s' "$entry" | jq -r '.vote_count')"
  reason="$(printf '%s' "$entry" | jq -r '.reason')"
  cutoff_iso="$(printf '%s' "$entry" | jq -r '.cutoff_iso')"
  handle_artifact "$artifact" "$usage_count" "$vote_count" "$reason" "$cutoff_iso" &
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
