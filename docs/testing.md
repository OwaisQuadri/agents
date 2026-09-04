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
cargo test --manifest-path tools/skill-eval/Cargo.toml
cargo build --release --manifest-path tools/tool-wizard/Cargo.toml   # any tools/<name>/Cargo.toml

# Pi extensions (Node test runner)
node --test pi/extensions/telemetry.test.ts pi/extensions/telemetry.security.test.ts pi/extensions/telemetry.rpc.test.ts
# telemetry loads its store lazily: a corrupt telemetry.jsonl no longer aborts pi
# startup; it surfaces as an extension_error on the first lifecycle event instead

# git hooks
hooks/test.sh

# a skill or workflow's eval harness (per-artifact contract, see skill-author/SKILL.md)
./skills/<name>/evals/run.sh              # all tiers, both slices, frontier writes
./skills/<name>/evals/run.sh --holdout    # all tiers, held-out slice, no frontier writes
./skills/<name>/evals/run.sh --tier T3    # one requested tier, both slices
```

Every skill and workflow runner delegates to `tools/skill-eval`. The runner uses
`tools/tier-dispatch` for real artifact runs and judge runs. It disables extension
discovery and loads `pi-anthropic-auth` as the minimum extension.

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
