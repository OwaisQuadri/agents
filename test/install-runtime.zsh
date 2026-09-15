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

check "Pi Model Context Protocol config is credential-free" jq -e '.mcpServers | type == "object" and has("linear-pillars") and all(.[]; . == {"url":"https://mcp.linear.app/mcp"})' "$repo/config/pi-mcp.json"
mkdir -p "$fixture/home/.pi/agent/extensions" "$fixture/home/.config/mcp"
ln -s "$fixture/old-checkout/pi/extensions/world-clock.ts" "$fixture/home/.pi/agent/extensions/world-clock.ts"
ln -s "$fixture/old-checkout/config/world-clock.json" "$fixture/home/.pi/agent/world-clock.json"
ln -s "$repo/config/pi-mcp.json" "$fixture/home/.pi/agent/mcp.json"
ln -s "$repo/config/pi-mcp.json" "$fixture/home/.config/mcp/mcp.json"
plan=$(HOME_TARGET="$fixture/home" REPO_TARGET="$repo" "$repo/install.sh" --dry-run)
check "retired world-clock extension is planned" grep -q "retire $fixture/home/.pi/agent/extensions/world-clock.ts" <(print -r -- "$plan")
check "retired world-clock config is planned" grep -q "unlink retired $fixture/home/.pi/agent/world-clock.json" <(print -r -- "$plan")
check "world-clock install is absent" test "$(print -r -- "$plan" | grep -c "world-clock.json ->" || true)" -eq 0
check "stable binary directory planned" grep -q "$fixture/home/.local/lib/agents-tools/ste-check" <(print -r -- "$plan")
check "command link uses stable binary" grep -q "$fixture/home/.local/bin/ste-check -> $fixture/home/.local/lib/agents-tools/ste-check" <(print -r -- "$plan")
check "tool-sync stable binary is planned" grep -q "$fixture/home/.local/lib/agents-tools/tool-sync" <(print -r -- "$plan")
check "legacy Pi Model Context Protocol link backup is planned" grep -Fq "backup $fixture/home/.pi/agent/mcp.json" <(print -r -- "$plan")
check "shared Model Context Protocol link backup is planned" grep -Fq "backup $fixture/home/.config/mcp/mcp.json" <(print -r -- "$plan")
check "shared Model Context Protocol config merge is planned" grep -Fq "merge $repo/config/pi-mcp.json -> $fixture/home/.config/mcp/mcp.json" <(print -r -- "$plan")
check "Pi Model Context Protocol adapter is pinned" grep -Fq 'name = "pi-mcp-adapter"' "$repo/config/tools.toml"
for override in .agents/mcp.json .agents/mcp/mcp.json .pi/agent/mcp.json; do
  override_home="$fixture/override-${override//\//-}"
  mkdir -p "$override_home/${override:h}"
  if [[ "$override" == .agents/mcp.json ]]; then
    print -r -- '{"mcp-servers":{"linear-pillars":{"command":"/tmp/not-linear"}}}' > "$override_home/$override"
  elif [[ "$override" == .pi/agent/mcp.json ]]; then
    print -r -- '{"mcpServers":{"linear-pillars":{"command":"/tmp/not-linear"}},"mcp-servers":{"linear-pillars":{"url":"https://mcp.linear.app/mcp"}}}' > "$override_home/$override"
  else
    print -r -- '{"mcpServers":{"linear-pillars":{"command":"/tmp/not-linear"}}}' > "$override_home/$override"
  fi
  override_status=0
  override_output=$(HOME_TARGET="$override_home" REPO_TARGET="$repo" "$repo/install.sh" --dry-run 2>&1) || override_status=$?
  check "managed server override is rejected in $override" test "$override_status" -ne 0
  override_failure="FATAL: $override_home/$override overrides a managed Model Context Protocol server"
  [[ "$override" == .pi/agent/mcp.json ]] && override_failure="FATAL: $override_home/$override overrides a managed Model Context Protocol transport"
  check "managed server override names the failure in $override" grep -Fq "$override_failure" <(print -r -- "$override_output")
done
dangling_home="$fixture/dangling-override"
mkdir -p "$dangling_home/.agents"
ln -s "$dangling_home/missing.json" "$dangling_home/.agents/mcp.json"
dangling_status=0
dangling_output=$(HOME_TARGET="$dangling_home" REPO_TARGET="$repo" "$repo/install.sh" --dry-run 2>&1) || dangling_status=$?
check "dangling override link is rejected" test "$dangling_status" -ne 0
check "dangling override link failure is named" grep -Fq "FATAL: $dangling_home/.agents/mcp.json is a dangling link" <(print -r -- "$dangling_output")

mkdir -p "$fixture/mcp-apply-repo/config" "$fixture/mcp-apply-repo/skills" "$fixture/mcp-apply-repo/tools/tool-sync/target/release" "$fixture/mcp-apply-home/.config/mcp" "$fixture/mcp-bin"
cp "$repo/config/pi-mcp.json" "$fixture/mcp-apply-repo/config/pi-mcp.json"
cat > "$fixture/mcp-apply-repo/tools/tool-sync/target/release/tool-sync" <<'TOOL'
#!/bin/sh
printf 'tool-sync fallback ran\n'
TOOL
chmod +x "$fixture/mcp-apply-repo/tools/tool-sync/target/release/tool-sync"
ln -s "$(command -v jq)" "$fixture/mcp-bin/jq"
mkdir -p "$fixture/jsonc-home/.config/mcp"
printf '%s\n' '{' '  // comment' '  "mcpServers": {},' '}' > "$fixture/jsonc-home/.config/mcp/mcp.json"
jsonc_status=0
jsonc_output=$(env -u CARGO_TARGET_DIR -u CARGO_BUILD_TARGET PATH="$fixture/mcp-bin:/usr/bin:/bin" HOME_TARGET="$fixture/jsonc-home" REPO_TARGET="$fixture/mcp-apply-repo" "$repo/install.sh" 2>&1) || jsonc_status=$?
check "non-strict shared config is rejected" test "$jsonc_status" -ne 0
check "non-strict shared config failure is named" grep -Fq "must use strict JSON without comments or trailing commas" <(print -r -- "$jsonc_output")
check "non-strict shared config fails before mutation" test ! -e "$fixture/jsonc-home/.pi"
mkdir -p "$fixture/bad-shape-home/.config/mcp"
print -r -- '{"mcpServers":"not-a-map"}' > "$fixture/bad-shape-home/.config/mcp/mcp.json"
bad_shape_status=0
bad_shape_output=$(env -u CARGO_TARGET_DIR -u CARGO_BUILD_TARGET PATH="$fixture/mcp-bin:/usr/bin:/bin" HOME_TARGET="$fixture/bad-shape-home" REPO_TARGET="$fixture/mcp-apply-repo" "$repo/install.sh" 2>&1) || bad_shape_status=$?
check "invalid shared server map is rejected" test "$bad_shape_status" -ne 0
check "invalid shared server map failure is named" grep -Fq "has an invalid server map" <(print -r -- "$bad_shape_output")
check "invalid shared server map fails before mutation" test ! -e "$fixture/bad-shape-home/.pi"
mkdir -p "$fixture/multi-document-home/.config/mcp"
printf '%s\n' '{"mcpServers":{}}' '{"note":"second document"}' > "$fixture/multi-document-home/.config/mcp/mcp.json"
multi_document_status=0
multi_document_output=$(env -u CARGO_TARGET_DIR -u CARGO_BUILD_TARGET PATH="$fixture/mcp-bin:/usr/bin:/bin" HOME_TARGET="$fixture/multi-document-home" REPO_TARGET="$fixture/mcp-apply-repo" "$repo/install.sh" 2>&1) || multi_document_status=$?
check "multi-document shared config is rejected" test "$multi_document_status" -ne 0
check "multi-document shared config failure is named" grep -Fq "must contain exactly one JSON object" <(print -r -- "$multi_document_output")
check "multi-document shared config fails before mutation" test ! -e "$fixture/multi-document-home/.pi"

mkdir -p "$fixture/custom-agent-dir" "$fixture/custom-home"
print -r -- '{"mcpServers":{"linear-pillars":{"requestHeadersCommand":{"command":"/tmp/not-linear"}}}}' > "$fixture/custom-agent-dir/mcp.json"
custom_status=0
custom_output=$(env -u CARGO_TARGET_DIR -u CARGO_BUILD_TARGET HOME="$fixture/custom-home" PI_CODING_AGENT_DIR="$fixture/custom-agent-dir" PATH="$fixture/mcp-bin:/usr/bin:/bin" HOME_TARGET="$fixture/custom-home" REPO_TARGET="$fixture/mcp-apply-repo" "$repo/install.sh" --dry-run 2>&1) || custom_status=$?
check "custom Pi agent directory override is rejected" test "$custom_status" -ne 0
check "custom Pi agent directory failure is named" grep -Fq "FATAL: $fixture/custom-agent-dir/mcp.json overrides a managed Model Context Protocol transport" <(print -r -- "$custom_output")
relative_status=0
relative_output=$(env -u CARGO_TARGET_DIR -u CARGO_BUILD_TARGET HOME="$fixture/custom-home" PI_CODING_AGENT_DIR="relative-agent" PATH="$fixture/mcp-bin:/usr/bin:/bin" HOME_TARGET="$fixture/custom-home" REPO_TARGET="$fixture/mcp-apply-repo" "$repo/install.sh" --dry-run 2>&1) || relative_status=$?
check "relative Pi agent directory is rejected" test "$relative_status" -ne 0
check "relative Pi agent directory failure is named" grep -Fq "FATAL: PI_CODING_AGENT_DIR must be absolute or start with ~/" <(print -r -- "$relative_output")
tilde_status=0
tilde_output=$(env -u CARGO_TARGET_DIR -u CARGO_BUILD_TARGET HOME="$fixture/custom-home" PI_CODING_AGENT_DIR="~" PATH="$fixture/mcp-bin:/usr/bin:/bin" HOME_TARGET="$fixture/custom-home" REPO_TARGET="$fixture/mcp-apply-repo" "$repo/install.sh" --dry-run 2>&1) || tilde_status=$?
check "bare-tilde Pi agent directory is rejected" test "$tilde_status" -ne 0
check "bare-tilde Pi agent directory failure is named" grep -Fq "FATAL: PI_CODING_AGENT_DIR must be absolute or start with ~/" <(print -r -- "$tilde_output")
exclusive_status=0
exclusive_output=$(env -u CARGO_TARGET_DIR -u CARGO_BUILD_TARGET PI_MCP_CONFIG_MODE=exclusive PATH="$fixture/mcp-bin:/usr/bin:/bin" HOME_TARGET="$fixture/exclusive-home" REPO_TARGET="$fixture/mcp-apply-repo" "$repo/install.sh" --dry-run 2>&1) || exclusive_status=$?
check "exclusive Model Context Protocol mode is rejected" test "$exclusive_status" -ne 0
check "exclusive Model Context Protocol mode failure is named" grep -Fq "FATAL: PI_MCP_CONFIG_MODE=exclusive does not load" <(print -r -- "$exclusive_output")
mkdir -p "$fixture/direct-tools-home/.pi/agent"
print -r -- '{"mcpServers":{"linear-pillars":{"url":"https://mcp.linear.app/mcp","directTools":["list_issues","create_issue"]}}}' > "$fixture/direct-tools-home/.pi/agent/mcp.json"
direct_tools_status=0
env -u CARGO_TARGET_DIR -u CARGO_BUILD_TARGET PATH="$fixture/mcp-bin:/usr/bin:/bin" HOME_TARGET="$fixture/direct-tools-home" REPO_TARGET="$fixture/mcp-apply-repo" "$repo/install.sh" --dry-run >/dev/null 2>&1 || direct_tools_status=$?
check "Pi direct-tools subset override is accepted" test "$direct_tools_status" -eq 0
print -r -- '{"mcpServers":{"linear-pillars":{"directTools":true}}}' > "$fixture/direct-tools-home/.pi/agent/mcp.json"
direct_tools_status=0
env -u CARGO_TARGET_DIR -u CARGO_BUILD_TARGET PATH="$fixture/mcp-bin:/usr/bin:/bin" HOME_TARGET="$fixture/direct-tools-home" REPO_TARGET="$fixture/mcp-apply-repo" "$repo/install.sh" --dry-run >/dev/null 2>&1 || direct_tools_status=$?
check "partial Pi direct-tools override is accepted" test "$direct_tools_status" -eq 0

print -r -- '{"mcp-servers":{"linear-pillars":{"headers":{"Authorization":"Bearer test-value"}},"other":{"url":"https://example.com/mcp"}}}' > "$fixture/mcp-apply-home/.config/mcp/mcp.json"
mcp_apply=$(env -u CARGO_TARGET_DIR -u CARGO_BUILD_TARGET PATH="$fixture/mcp-bin:/usr/bin:/bin" HOME_TARGET="$fixture/mcp-apply-home" REPO_TARGET="$fixture/mcp-apply-repo" "$repo/install.sh" 2>&1)
check "Model Context Protocol merge applies" grep -Fq "merge $fixture/mcp-apply-repo/config/pi-mcp.json -> $fixture/mcp-apply-home/.config/mcp/mcp.json" <(print -r -- "$mcp_apply")
check "applied merge drops stale credentials" jq -e '.mcpServers["linear-pillars"] == {"url":"https://mcp.linear.app/mcp"}' "$fixture/mcp-apply-home/.config/mcp/mcp.json"
check "applied merge preserves hyphenated other servers" jq -e '.mcpServers.other == {"url":"https://example.com/mcp"} and has("mcp-servers") == false' "$fixture/mcp-apply-home/.config/mcp/mcp.json"
if [[ "$(uname -s)" == "Darwin" ]]; then
  mcp_mode=$(stat -f '%Lp' "$fixture/mcp-apply-home/.config/mcp/mcp.json")
else
  mcp_mode=$(stat -c '%a' "$fixture/mcp-apply-home/.config/mcp/mcp.json")
fi
check "applied merge uses private file mode" test "$mcp_mode" = 600
mcp_backup="$fixture/mcp-apply-home/.config/mcp/mcp.json.pre-reset-$(date +%Y%m%d)"
if [[ "$(uname -s)" == "Darwin" ]]; then
  mcp_backup_mode=$(stat -f '%Lp' "$mcp_backup")
else
  mcp_backup_mode=$(stat -c '%a' "$mcp_backup")
fi
check "merge backup uses private file mode" test "$mcp_backup_mode" = 600
mcp_temps=("$fixture/mcp-apply-home/.config/mcp/mcp.json".tmp.*(N))
check "applied merge removes its temporary file" test "${#mcp_temps[@]}" -eq 0
mcp_repeat=$(env -u CARGO_TARGET_DIR -u CARGO_BUILD_TARGET PATH="$fixture/mcp-bin:/usr/bin:/bin" HOME_TARGET="$fixture/mcp-apply-home" REPO_TARGET="$fixture/mcp-apply-repo" "$repo/install.sh" 2>&1)
check "Model Context Protocol merge is idempotent" grep -Fq "ok   $fixture/mcp-apply-home/.config/mcp/mcp.json managed Model Context Protocol servers" <(print -r -- "$mcp_repeat")

mkdir -p "$fixture/mcp-both-home/.config/mcp"
print -r -- '{"mcp-servers":{"dormant":{"command":"/tmp/not-active"}},"mcpServers":{"other":{"url":"https://example.com/mcp"}}}' > "$fixture/mcp-both-home/.config/mcp/mcp.json"
env -u CARGO_TARGET_DIR -u CARGO_BUILD_TARGET PATH="$fixture/mcp-bin:/usr/bin:/bin" HOME_TARGET="$fixture/mcp-both-home" REPO_TARGET="$fixture/mcp-apply-repo" "$repo/install.sh" >/dev/null 2>&1
check "active key wins when both forms exist" jq -e '.mcpServers.other == {"url":"https://example.com/mcp"} and (.mcpServers.dormant == null) and .["mcp-servers"].dormant == {"command":"/tmp/not-active"}' "$fixture/mcp-both-home/.config/mcp/mcp.json"

mkdir -p "$fixture/external-agent-dir" "$fixture/sandbox-home"
ln -s "$fixture/mcp-apply-repo/config/pi-mcp.json" "$fixture/external-agent-dir/mcp.json"
PI_CODING_AGENT_DIR="$fixture/external-agent-dir" env -u CARGO_TARGET_DIR -u CARGO_BUILD_TARGET PATH="$fixture/mcp-bin:/usr/bin:/bin" HOME_TARGET="$fixture/sandbox-home" REPO_TARGET="$fixture/mcp-apply-repo" "$repo/install.sh" >/dev/null 2>&1
check "sandbox install ignores external Pi agent directory" test -L "$fixture/external-agent-dir/mcp.json"

mkdir -p "$fixture/mcp-dangling-home/.config/mcp"
ln -s "$fixture/mcp-dangling-home/missing.json" "$fixture/mcp-dangling-home/.config/mcp/mcp.json"
shared_dangling_status=0
shared_dangling_output=$(env -u CARGO_TARGET_DIR -u CARGO_BUILD_TARGET PATH="$fixture/mcp-bin:/usr/bin:/bin" HOME_TARGET="$fixture/mcp-dangling-home" REPO_TARGET="$fixture/mcp-apply-repo" "$repo/install.sh" 2>&1) || shared_dangling_status=$?
check "dangling shared link is rejected" test "$shared_dangling_status" -ne 0
check "dangling shared link failure is named" grep -Fq "FATAL: $fixture/mcp-dangling-home/.config/mcp/mcp.json is a dangling link" <(print -r -- "$shared_dangling_output")

if (( fails > 0 )); then
  print "$fails FAILURES"
  exit 1
fi

print "ALL PASS"
