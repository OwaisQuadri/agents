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
cargo build --release --manifest-path tools/tool-wizard/Cargo.toml   # any tools/<name>/Cargo.toml

# Pi extensions (Node test runner)
node --test pi/extensions/telemetry.test.ts pi/extensions/telemetry.security.test.ts pi/extensions/telemetry.rpc.test.ts

# git hooks
hooks/test.sh

# a skill or workflow's eval harness (per-artifact contract, see skill-author/SKILL.md)
./skills/<name>/evals/run.sh              # non-holdout cases
./skills/<name>/evals/run.sh --holdout    # held-out case
```

Eval harnesses call out to a model to grade cases. Use `pi -p` first; fall back to
`codex exec --skip-git-repo-check --sandbox read-only -c mcp_servers={}` when `pi`'s
default provider is out of usage. Don't reach for `claude -p` as the primary path —
it's the one most likely to be rate-limited or out of usage mid-session.

## manifest / policy checks (no writes)

```sh
tools/tool-sync/target/release/tool-sync \
  --repository-root "$PWD" --manifest config/tools.toml --home "$HOME" --check

./install-policy.sh --dry-run
```

## before landing a change

Run the eval harness for every artifact you touched (holdout included), then
`./install.sh --test --dry-run` once to confirm the install plan still resolves. See
README.md's "Updates and verification" section for the fuller checklist specific to
bumping a pinned upstream revision (`pi-subagents`, `plannotator`, etc.) — that's a
narrower case than "did my change break anything."
