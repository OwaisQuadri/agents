#!/bin/zsh
set -euo pipefail

repo=${0:A:h:h}
fixture=$(mktemp -d "${TMPDIR:-/tmp}/install-runtime.XXXXXX")
trap 'rm -rf "$fixture"' EXIT
fails=0

check() {
  local label=$1
  shift
  if "$@"; then
    print "PASS $label"
  else
    print "FAIL $label"
    (( fails += 1 ))
  fi
}

mkdir -p "$fixture/bin" "$fixture/repository/skills"
cat > "$fixture/bin/lockf" <<'LOCKF'
#!/bin/zsh
print -r -- "$@" > "$LOCK_RECORD"
exit 75
LOCKF
chmod +x "$fixture/bin/lockf"

set +e
lock_output=$(LOCK_RECORD="$fixture/lock-args" PATH="$fixture/bin:$PATH" INSTALL_LOCK_TIMEOUT_SECONDS=0 \
  HOME_TARGET="$fixture/home" REPO_TARGET="$fixture/repository" \
  "$repo/install.sh" 2>&1)
lock_status=$?
set -e
check "macOS lock wiring ran" test -f "$fixture/lock-args"
check "macOS lock wiring is nonblocking" grep -q "^-s -t 0 9$" "$fixture/lock-args"
check "macOS lock file is home-scoped" test -f "$fixture/home/.cache/agents/install.lock"
check "macOS contention returns temporary failure" test "$lock_status" -eq 75
check "macOS contention requests retry" grep -q "retry installation" <(print -r -- "$lock_output")

mkdir -p "$fixture/linux-bin"
cat > "$fixture/linux-bin/uname" <<'UNAME'
#!/bin/zsh
print Linux
UNAME
cat > "$fixture/linux-bin/flock" <<'FLOCK'
#!/bin/zsh
print -r -- "$@" > "$FLOCK_RECORD"
exit 75
FLOCK
chmod +x "$fixture/linux-bin/uname" "$fixture/linux-bin/flock"
set +e
flock_output=$(FLOCK_RECORD="$fixture/flock-args" PATH="$fixture/linux-bin:$PATH" INSTALL_LOCK_TIMEOUT_SECONDS=0 \
  HOME_TARGET="$fixture/linux-home" REPO_TARGET="$fixture/repository" \
  "$repo/install.sh" 2>&1)
flock_status=$?
set -e
check "Linux lock wiring ran" test -f "$fixture/flock-args"
check "Linux lock wiring has a timeout" grep -q "^-w 0 9$" "$fixture/flock-args"
check "Linux contention returns temporary failure" test "$flock_status" -eq 75
check "Linux contention requests retry" grep -q "retry installation" <(print -r -- "$flock_output")

if command -v lockf >/dev/null 2>&1; then
  mkdir -p "$fixture/contended-home/.cache/agents"
  oracle_lock="$fixture/contended-home/.cache/agents/install.lock"
  mkfifo "$fixture/ready" "$fixture/release"
  lockf -k "$oracle_lock" /bin/zsh -c 'print ready > "$1"; read -r release < "$2"' oracle "$fixture/ready" "$fixture/release" &
  holder=$!
  read -r ready < "$fixture/ready"
  is_contended=0
  lockf -t 0 "$oracle_lock" /usr/bin/true 2>/dev/null || is_contended=1
  check "lockf excludes a concurrent holder" test "$is_contended" -eq 1
  set +e
  contended=$(INSTALL_LOCK_TIMEOUT_SECONDS=0 HOME_TARGET="$fixture/contended-home" REPO_TARGET="$fixture/repository" "$repo/install.sh" 2>&1)
  contended_status=$?
  set -e
  check "concurrent install returns temporary failure" test "$contended_status" -eq 75
  check "concurrent install requests retry" grep -q "retry installation" <(print -r -- "$contended")
  print release > "$fixture/release"
  wait "$holder"
  check "lockf releases after exit" lockf -t 0 "$oracle_lock" /usr/bin/true
elif command -v flock >/dev/null 2>&1; then
  mkdir -p "$fixture/contended-home/.cache/agents"
  oracle_lock="$fixture/contended-home/.cache/agents/install.lock"
  mkfifo "$fixture/ready" "$fixture/release"
  flock "$oracle_lock" /bin/zsh -c 'print ready > "$1"; read -r release < "$2"' oracle "$fixture/ready" "$fixture/release" &
  holder=$!
  read -r ready < "$fixture/ready"
  is_contended=0
  flock -n "$oracle_lock" /usr/bin/true 2>/dev/null || is_contended=1
  check "flock excludes a concurrent holder" test "$is_contended" -eq 1
  set +e
  contended=$(INSTALL_LOCK_TIMEOUT_SECONDS=0 HOME_TARGET="$fixture/contended-home" REPO_TARGET="$fixture/repository" "$repo/install.sh" 2>&1)
  contended_status=$?
  set -e
  check "concurrent install returns temporary failure" test "$contended_status" -eq 75
  check "concurrent install requests retry" grep -q "retry installation" <(print -r -- "$contended")
  print release > "$fixture/release"
  wait "$holder"
  check "flock releases after exit" flock -n "$oracle_lock" /usr/bin/true
fi

mkdir -p "$fixture/positive-repo/skills/x" "$fixture/positive-repo/tools/ste-check/src"
cat > "$fixture/positive-repo/tools/ste-check/Cargo.toml" <<'CARGO'
[package]
name = "ste-check"
version = "0.1.0"
edition = "2021"
CARGO
cat > "$fixture/positive-repo/tools/ste-check/src/main.rs" <<'RUST'
fn main() {}
RUST
positive=$(CARGO_TARGET_DIR="$fixture/alternate-target" HOME_TARGET="$fixture/positive-home" REPO_TARGET="$fixture/positive-repo" "$repo/install.sh" 2>/dev/null)
check "alternate Cargo target install completes" grep -q "plan: done" <(print -r -- "$positive")
check "alternate Cargo target binary is stable" test -x "$fixture/positive-home/.local/lib/agents-tools/ste-check"
check "alternate Cargo target command is linked" test "$(readlink "$fixture/positive-home/.local/bin/ste-check")" = "$fixture/positive-home/.local/lib/agents-tools/ste-check"
if command -v lockf >/dev/null 2>&1; then
  check "completed install releases its lock" lockf -t 0 "$fixture/positive-home/.cache/agents/install.lock" /usr/bin/true
else
  check "completed install releases its lock" flock -n "$fixture/positive-home/.cache/agents/install.lock" /usr/bin/true
fi

mkdir -p "$fixture/positive-repo/.cargo" "$fixture/positive-repo/tools/tool-sync/src"
cat > "$fixture/positive-repo/tools/tool-sync/Cargo.toml" <<'CARGO'
[package]
name = "tool-sync"
version = "0.1.0"
edition = "2021"
CARGO
cat > "$fixture/positive-repo/tools/tool-sync/src/main.rs" <<'RUST'
fn main() {}
RUST
cat > "$fixture/positive-repo/.cargo/config.toml" <<CARGO
[build]
target-dir = "$fixture/configured-target"
CARGO
configured=$(cd "$fixture/positive-repo" && HOME_TARGET="$fixture/configured-home" REPO_TARGET="$fixture/positive-repo" "$repo/install.sh" 2>/dev/null)
check "Cargo config target install completes" grep -q "plan: done" <(print -r -- "$configured")
check "Cargo config target binary is stable" test -x "$fixture/configured-home/.local/lib/agents-tools/ste-check"
configured_dry=$(cd "$fixture/positive-repo" && HOME_TARGET="$fixture/configured-dry-home" REPO_TARGET="$fixture/positive-repo" "$repo/install.sh" --dry-run 2>/dev/null)
check "Cargo config target dry run completes" grep -q "plan: done" <(print -r -- "$configured_dry")

mkdir -p "$fixture/no-cargo-repo/skills/x" "$fixture/no-cargo-repo/tools/tool-sync/target/release"
cat > "$fixture/no-cargo-repo/tools/tool-sync/Cargo.toml" <<'CARGO'
[package]
name = "tool-sync"
version = "0.1.0"
edition = "2021"
CARGO
cat > "$fixture/no-cargo-repo/tools/tool-sync/target/release/tool-sync" <<'TOOL'
#!/bin/sh
printf 'tool-sync fallback ran\n'
TOOL
chmod +x "$fixture/no-cargo-repo/tools/tool-sync/target/release/tool-sync"
no_cargo=$(env -u CARGO_TARGET_DIR -u CARGO_BUILD_TARGET PATH="/usr/bin:/bin" HOME_TARGET="$fixture/no-cargo-home" REPO_TARGET="$fixture/no-cargo-repo" "$repo/install.sh" 2>/dev/null)
check "prebuilt tool-sync works without Cargo" grep -q "tool-sync fallback ran" <(print -r -- "$no_cargo")
check "no-Cargo install completes" grep -q "plan: done" <(print -r -- "$no_cargo")

plan=$(HOME_TARGET="$fixture/home" "$repo/install.sh" --dry-run)
check "stable binary directory planned" grep -q "$fixture/home/.local/lib/agents-tools/ste-check" <(print -r -- "$plan")
check "command link uses stable binary" grep -q "$fixture/home/.local/bin/ste-check -> $fixture/home/.local/lib/agents-tools/ste-check" <(print -r -- "$plan")
check "tool-sync stable binary is planned" grep -q "$fixture/home/.local/lib/agents-tools/tool-sync" <(print -r -- "$plan")

if (( fails > 0 )); then
  print "$fails FAILURES"
  exit 1
fi

print "ALL PASS"
