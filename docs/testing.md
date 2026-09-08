# testing this repo

The one canonical answer to "how do I test this" — read this before improvising a
command. Every check below is safe to run from a fresh checkout; none of them touch
your real `~/.claude`, `~/.codex`, `~/.pi`, `~/.local/bin`, or `~/.zshrc` unless you say
so explicitly.

## the one command for "does the install still work"

```sh
./install.sh --test --dry-run
```

`--test` pins `HOME_TARGET` to `.install-test-home/` (gitignored, inside this worktree)
instead of your real home. `--dry-run` on top of it means the whole run is read-only —
plan only, nothing written, and it returns to your shell. This is the safe default for
an agent checking its own change: fast, non-destructive, no surprises.

Without `--dry-run`, `./install.sh --test` writes links into the sandbox home and
returns to the shell. Use `./test/build` for the same build-only behavior. Use
`./test/run` to open the existing sandbox, or use `./test/build_run` to rebuild it and
open an interactive Pi session.

## the env var is `REPO_TARGET`, not `REPO_ROOT`

`install.sh` reads `REPO_TARGET` (default: the script's own directory). `REPO_ROOT` is
not a recognized variable — setting it does nothing, and the script silently falls back
to its default, which happens to already match `$PWD` when you run it from the repo
root. That's why `REPO_ROOT=$PWD ./install.sh` appears to work: the env var is simply
ignored and the default was already correct. Use `REPO_TARGET` explicitly when testing
a worktree that isn't your current directory:

```sh
REPO_TARGET="$PWD" ./install.sh --dry-run
```

## component-level checks

```sh
# Rust tools
cargo test --manifest-path tools/tool-sync/Cargo.toml
cargo test --manifest-path tools/tier-dispatch/Cargo.toml
cargo test --manifest-path tools/skill-eval/Cargo.toml
tools/skill-eval/timing-test.sh
cargo build --release --manifest-path tools/tool-wizard/Cargo.toml   # any tools/<name>/Cargo.toml

# Pi extensions (Node test runner)
node --test pi/extensions/telemetry.test.ts pi/extensions/telemetry.security.test.ts pi/extensions/telemetry.rpc.test.ts
# telemetry loads its store lazily: a corrupt telemetry.jsonl no longer aborts pi
# startup; it surfaces as an extension_error on the first lifecycle event instead

# git hooks
hooks/test.sh

# a skill or workflow's eval harness (per-artifact contract, see skill-author/SKILL.md)
./skills/<name>/evals/run.sh                                  # incumbent baseline, all tiers and slices
./skills/<name>/evals/run.sh <candidate>                      # paired dry comparison, all tiers and slices
./skills/<name>/evals/run.sh --accept-if-winning <candidate>  # conditionally apply a paired winner
./skills/<name>/evals/run.sh --holdout                        # diagnostic holdout mode
./skills/<name>/evals/run.sh --tier T3                        # diagnostic single-tier mode
```

Every skill and workflow runner delegates to `tools/skill-eval`. A full candidate run
compares the live incumbent and candidate together. It executes every configured tier and records
paired evidence. The runner selects the highest-scoring contiguous suffix ending at the highest
tier. It runs bounded `(arm, tier, slice, case, repeat)` units with four workers by default.
Use `--jobs N` or `SKILL_EVAL_JOBS` to set one through 16 workers. The full paired modes save
complete units under `evals/.skill-eval-state/<run-key>/` and resume the exact incomplete run.
A completed `null` score remains complete on automatic resume. Use `--restart` to retry unavailable
providers after they recover. `--restart` discards only that run's saved units. The runner writes
progress to standard error after each durable unit. It writes incumbent records before candidate
records. Each arm orders records by configured tier, non-holdout cases, then holdout cases. It
serializes `output-check.sh` while model dispatches run concurrently. The runner uses
`evals/.skill-eval-state/run.lock` as one stable advisory lock per artifact. The lock permits one
paired coordinator while its workers remain concurrent. After a completed run writes its frontier
and live definition, a state cleanup failure is a warning. The completed run keeps its success exit.

A surviving median keeps a partly ungraded tier in that ranking. The completeness gate then
rejects a selected suffix with any missing repeat. Tiers below the selected floor remain recorded,
but they do not gate acceptance.

These exit codes apply to the paired candidate forms. A complete dry comparison exits 0.
Conditional acceptance exits 0 when applied and 1 for a valid rejection. Incomplete evidence or
an execution failure exits 2.

Configuration and usage errors also exit 2 in every mode. A corrupt saved run or a second coordinator
for the same run also exits 2. Baseline and narrow diagnostic runs otherwise retain their earlier
result behavior. The runner uses `tools/tier-dispatch` for real artifact runs and judge runs. It
disables extension discovery and loads `pi-anthropic-auth` as the minimum extension.

Each case row reports its total time. Its repeat records report generation and judge times,
requested tiers, final models, and each fallback attempt. `tools/tier-dispatch` reports the
model, thinking level, time, and result for every attempt.

Each run prints its comparison identifier before the first case. It writes append-only events
under `${SKILL_EVAL_STATE_DIR:-$HOME/.local/state/skill-eval}/runs`. Resume an interrupted run
with the same candidate and `--resume <comparison-id>`. The runner names changed inputs and
stops before dispatch when the saved inputs are stale. It reruns an interrupted case and skips
completed cases. A checkpoint cannot update the frontier or live artifact.

The state directory keeps the latest 100 completed runs per artifact. It keeps active and
resumable runs and removes stale incomplete runs. Set `SKILL_EVAL_STATE_DIR` to isolate tests.
Use `--resume-from-log <path>` once to import a legacy mixed log that forms an exact case prefix.
Imported rows keep unknown time and model fields as null.

Custom agent harnesses use Pi through `tools/tier-dispatch` and write the same timing state.
Run `tools/skill-eval/timing-test.sh` to test their case records, parallel-run safety, web
extension preflight, dispatch bound, and retention without a live model.

## manifest / policy checks

```sh
tools/tool-sync/target/release/tool-sync \
  --repository-root "$PWD" --manifest config/tools.toml --home "$HOME" --check

./install-policy.sh --dry-run

cargo run --quiet --manifest-path tools/tier-dispatch/Cargo.toml -- \
  --verify-registry --tiers-file config/model-tiers.json
```

The registry check finds Pi's registry from `HOME`; do not put its home-directory path on
the command line. This keeps the command runnable inside Pi sessions.

## before landing a change

Run the eval harness for every artifact you touched (holdout included), then
`./install.sh --test --dry-run` once to confirm the install plan still resolves. See
README.md's "Updates and verification" section for the fuller checklist specific to
bumping a pinned upstream revision (`pi-subagents`, `plannotator`, etc.) — that's a
narrower case than "did my change break anything."
