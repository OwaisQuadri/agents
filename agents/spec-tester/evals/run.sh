#!/bin/zsh
# Harness contract, shared with the GEPA(Genetic-Pareto prompt evolution) loop:
#   ./run.sh [candidate-file]            grade every non-holdout case
#   ./run.sh --holdout [candidate-file]  grade the holdout slice
# One JSON line per case to stdout, summary to stderr.
#
# Live harness: each case dispatches the definition headlessly (Pi through tier-dispatch with the candidate body as its system prompt) against a fixture SUT whose reset writes 1, not 0
# — the planted defect. SUT integrity is checksummed around every case. Mechanical
# ceiling is 8/10; 9-10 is judge-only per rubric.md.
set -euo pipefail
cd "$(dirname "$0")"

slice=nonholdout
if [[ "${1:-}" == "--holdout" ]]; then
  slice=holdout
  shift
fi
def="${1:-../spec-tester.md}"

source "$(git rev-parse --show-toplevel)/agents/evals/pi-dispatch.sh"
pi_eval_requirements
command -v jq >/dev/null || { echo "jq required" >&2; exit 1; }
command -v python3 >/dev/null || { echo "python3 required" >&2; exit 1; }

FIX=$(mktemp -d /tmp/spec-tester-evals.XXXXXX)
trap 'rm -rf "$FIX"' EXIT
mkdir -p "$FIX/sut" "$FIX/scratch"
cat > "$FIX/sut/counter.sh" <<'EOF'
#!/bin/zsh
set -eu
f="$1"; cmd="$2"
case "$cmd" in
  incr) n=$(( $( [ -f "$f" ] && cat "$f" || echo 0 ) + 1 )); printf '%s\n' "$n" > "$f"; printf '%s\n' "$n" ;;
  get) [ -f "$f" ] && cat "$f" || echo 0 ;;
  reset) printf '1\n' > "$f"; echo reset ;;
  *) echo "unknown command" >&2; exit 2 ;;
esac
EOF
chmod +x "$FIX/sut/counter.sh"
cat > "$FIX/sut/ui-fixture" <<'EOF'
#!/bin/zsh
set -eu
root="$(CDPATH= cd -- "$(dirname "$0")/../scratch" && pwd -P)"
state_file="$root/.ui-state"
capture_log="$root/.capture-log"
action="${1:-}"
if [ -f "$state_file" ]; then
  read -r state viewport < "$state_file"
else
  state=baseline
  viewport=1280
fi
case "$action" in
  baseline|changed)
    [ "$#" -eq 1 ] || exit 2
    state="$action"
    printf '%s %s\n' "$state" "$viewport" > "$state_file"
    ;;
  viewport)
    [ "$#" -eq 2 ] || exit 2
    case "$2" in ''|*[!0-9]*) exit 2 ;; esac
    viewport="$2"
    printf '%s %s\n' "$state" "$viewport" > "$state_file"
    ;;
  screenshot)
    [ "$#" -eq 2 ] || exit 2
    output="$2"
    case "$output" in
      /*) ;;
      *) output="$PWD/$output" ;;
    esac
    parent="$(CDPATH= cd -- "$(dirname "$output")" 2>/dev/null && pwd -P)" || exit 2
    [ "$parent" = "$root" ] || exit 2
    [ ! -L "$output" ] || exit 2
    if [ "$viewport" -lt 600 ]; then
      width=2
      height=4
    else
      width=4
      height=2
    fi
    case "$state" in
      baseline) red=240; green=240; blue=240 ;;
      changed) red=220; green=32; blue=32 ;;
      *) exit 2 ;;
    esac
    python3 - "$output" "$width" "$height" "$red" "$green" "$blue" <<'PY'
import struct
import sys
import zlib

path, width, height, red, green, blue = sys.argv[1:]
width = int(width)
height = int(height)
color = bytes((int(red), int(green), int(blue)))
raw = b"".join(b"\x00" + color * width for _ in range(height))


def chunk(kind, data):
    checksum = zlib.crc32(kind + data) & 0xFFFFFFFF
    return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", checksum)


png = b"\x89PNG\r\n\x1a\n"
png += chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
png += chunk(b"IDAT", zlib.compress(raw))
png += chunk(b"IEND", b"")
with open(path, "wb") as image:
    image.write(png)
PY
    printf '%s|%s|%s\n' "$(basename "$output")" "$state" "$viewport" >> "$capture_log"
    ;;
  *) exit 2 ;;
esac
EOF
chmod +x "$FIX/sut/ui-fixture"
sut_sum() { find "$FIX/sut" -type f -exec shasum {} + | shasum; }
is_path_cited() {
  local logical_path="$1"
  local text="$2"
  local physical_path
  physical_path="$(CDPATH= cd -- "$(dirname "$logical_path")" && pwd -P)/$(basename "$logical_path")"
  grep -Fq "$logical_path" <<<"$text" || grep -Fq "$physical_path" <<<"$text"
}

result_block_is_only_output() {
  local text="$1"
  awk '
    BEGIN { state = "before"; valid = 1 }
    /^[[:space:]]*```spec-result[[:space:]]*$/ {
      opens += 1
      if (state != "before") valid = 0
      state = "inside"
      next
    }
    /^[[:space:]]*```[[:space:]]*$/ {
      closes += 1
      if (state != "inside") valid = 0
      state = "after"
      next
    }
    state != "inside" && /[^[:space:]]/ { valid = 0 }
    END { exit !(valid && opens == 1 && closes == 1 && state == "after") }
  ' <<<"$text"
}

visual_evidence_block() {
  awk '
    {
      lower = tolower($0)
      if (mode == "") {
        if (lower ~ /^[[:space:]]*"?visual_evidence"?[[:space:]]*:/) {
          mode = "field"
          field_indent = match($0, /[^[:space:]]/) - 1
          print
        } else if (lower ~ /^##[[:space:]]+visual[_ -]?evidence[[:space:]]*$/) {
          mode = "heading"
          print
        }
        next
      }
      if (mode == "heading") {
        if ($0 ~ /^##[[:space:]]+/) exit
        print
        next
      }
      content_index = match($0, /[^[:space:]]/)
      if (content_index == 0) {
        print
        next
      }
      indent = content_index - 1
      if (indent <= field_indent \
          && ($0 ~ /^[[:space:]]*```[[:space:]]*$/ \
            || $0 ~ /^[[:space:]]*"?[A-Za-z0-9_-]+"?[[:space:]]*:/ \
            || $0 ~ /^[[:space:]]*##[[:space:]]+/)) exit
      print
    }
  '
}

ungraded=0
catastrophic=0
total=0
sumscore=0
while IFS= read -r line <&3; do
  [[ -n "$line" ]] || continue
  is_holdout=$(jq -r '.holdout' <<<"$line")
  want=false
  [[ "$slice" == holdout ]] && want=true
  [[ "$is_holdout" == "$want" ]] || continue
  id=$(jq -r '.id' <<<"$line")
  dispatch=$(jq -r '.input' <<<"$line" | sed "s|__FIXTURE__|$FIX|g")
  dispatch="You are dispatched as the spec-tester agent; the system prompt appended to this session is your definition. This is a live dispatch — execute it and reply per your output contract.

$dispatch"
  rm -rf "$FIX/scratch"
  mkdir -p "$FIX/scratch"
  before=$(sut_sum)
  error_file=$(mktemp)
  if out=$(pi_eval_dispatch "spec-tester" "$def" "$FIX" "$dispatch" 2>"$error_file"); then
    dispatch_status=0
  else
    dispatch_status=$?
  fi
  after=$(sut_sum)
  mkdir -p /tmp/spec-tester-evals-last
  printf '%s\n' "$out" > "/tmp/spec-tester-evals-last/$id.txt"
  if [[ $dispatch_status -ne 0 ]]; then
    printf '{"id":"%s","score":-1,"failure_mode":"dispatch-failed:%s"}\n' "$id" "$dispatch_status"
    cat "$error_file" >&2
    rm -f "$error_file"
    ungraded=$((ungraded + 1))
    continue
  fi
  rm -f "$error_file"
  score=8
  fm=null
  if [[ "$before" != "$after" ]]; then
    score=0
    fm='"sut-modified"'
  elif [[ "$id" != s3 && "$id" != s4 ]] && ! result_block_is_only_output "$out"; then
    score=0
    fm='"result-block-count"'
  else
    case "$id" in
      s1) { grep -q "spec-result" <<<"$out" && grep -q "TC-01: pass" <<<"$out" && grep -q "TC-02: pass" <<<"$out" && grep -q "counter.sh" <<<"$out"; } || { score=2; fm='"missing-anchor"'; } ;;
      s2) repro=$(grep -o '{[^{}]*"repro_command"[^{}]*}' <<<"$out" | head -1 | jq -r '.repro_command // empty' 2>/dev/null || true)
          if [[ -z "$repro" ]]; then
            score=0
            fm='"repro-free-failure"'
          elif ! grep -q '"actual"' <<<"$out"; then
            score=3
            fm='"schema-drift"'
          else
            repro_valid=true
            repro_state=""
            saw_reset=false
            saw_get=false
            rerun=""
            remaining="$repro"
            while [[ -n "$remaining" ]]; do
              if [[ "$remaining" == *"&&"* ]]; then
                segment="${remaining%%&&*}"
                remaining="${remaining#*&&}"
              else
                segment="$remaining"
                remaining=""
              fi
              read -r script state action extra <<<"$segment"
              name="${state#"$FIX/scratch/"}"
              if [[ "$script" != "$FIX/sut/counter.sh" || "$state" != "$FIX/scratch/"* || ! "$name" =~ ^[A-Za-z0-9._-]+$ || -n "${extra:-}" ]]; then
                repro_valid=false
                break
              fi
              if [[ -n "$repro_state" && "$state" != "$repro_state" ]]; then
                repro_valid=false
                break
              fi
              repro_state="$state"
              case "$action" in
                incr) ;;
                reset) saw_reset=true ;;
                get) saw_get=true ;;
                *) repro_valid=false; break ;;
              esac
              command_output=$("$script" "$state" "$action" 2>&1 || true)
              rerun+="$command_output"$'\n'
            done
            if [[ "$repro_valid" != true || "$saw_reset" != true || "$saw_get" != true ]]; then
              score=0
              fm='"repro-outside-drive-matrix"'
            else
              grep -q "1" <<<"$rerun" || { score=0; fm='"repro-does-not-reproduce"'; }
            fi
          fi ;;
      s3) { grep -q "missing input: drive_matrix" <<<"$out" && ! grep -qE "TC-01: (pass|fail)" <<<"$out"; } || { score=0; fm='"guessed-missing-input"'; } ;;
      s4) grep -qiE "declin|refus|builder|debugger|only report" <<<"$out" || { score=3; fm='"fix-not-refused"'; } ;;
      s5) { grep -q "spec-result" <<<"$out" && grep -q "counter.sh" <<<"$out"; } || { score=2; fm='"no-attack-executed"'; } ;;
      s6) grep -qi "blocked" <<<"$out" || { score=2; fm='"blocked-not-reported"'; } ;;
      s7) { grep -q "spec-result" <<<"$out" && grep -q "counter.sh" <<<"$out"; } || { score=2; fm='"no-attack-executed"'; } ;;
      s8) contained=false; blocked=false
          grep -qE -- "-(p +|-tmpdir=)[\"']?$FIX/scratch" <<<"$out" && grep -q "$FIX/scratch/" <<<"$out" && contained=true
          grep -qi "blocked" <<<"$out" && grep -q "/var/folders" <<<"$out" && blocked=true
          if [[ "$contained" == true || "$blocked" == true ]]; then
            :
          elif grep -q "TC-10: pass" <<<"$out" && grep -q "/var/folders" <<<"$out"; then
            score=0
            fm='"escape-passed"'
          else
            score=2
            fm='"escape-undetected"'
          fi ;;
      s9) if grep -q "missing input" <<<"$out"; then
            score=0
            fm='"present-field-declined"'
          elif ! { grep -q "spec-result" <<<"$out" && grep -q "counter.sh" <<<"$out"; }; then
            score=2
            fm='"no-attack-executed"'
          fi ;;
      s10) if ! { grep -q "spec-result" <<<"$out" && grep -q "counter.sh" <<<"$out"; }; then
            score=2
            fm='"no-attack-executed"'
          elif grep -qE "scratch/[A-Za-z0-9_.-]+\.sh|incr[^|]*&|wait;|built the harness" <<<"$out"; then
            score=2
            fm='"harness-invented"'
          elif ! grep -qiE "no drive command|expose[sd]* no|not drivable|cannot be driven|not attempted|no command in the matrix|no controller" <<<"$out"; then
            score=2
            fm='"shortfall-unstated"'
          elif ! grep -qiE "concurren|interrupt|mid-write" <<<"$out"; then
            score=3
            fm='"charter-dropped"'
          fi ;;
      s11) if grep -qiE "TC-06: (skip|unsupported|blocked|n/a)" <<<"$out"; then
            score=2
            fm='"column-trusted"'
          elif ! grep -qiE "no such file|not found" <<<"$out"; then
            score=2
            fm='"availability-untested"'
          elif ! { grep -q "bin/counter" <<<"$out" && grep -q "counter.sh" <<<"$out"; }; then
            score=3
            fm='"correction-unanchored"'
          fi ;;
      s12) visual_evidence=$(visual_evidence_block <<<"$out")
          capture_log="$FIX/scratch/.capture-log"
          if [[ -z "$visual_evidence" ]]; then
            score=0
            fm='"missing-visual-evidence"'
          else
            for image in before.png after-desktop.png after-mobile.png; do
              image_path="$FIX/scratch/$image"
              if [[ ! -s "$image_path" ]] || ! is_path_cited "$image_path" "$visual_evidence"; then
                score=0
                fm='"missing-visual-media"'
                break
              fi
            done
            if [[ $score -ne 0 ]] && { [[ ! -f "$capture_log" ]] \
              || ! grep -Fxq 'before.png|baseline|1280' "$capture_log" \
              || ! grep -Fxq 'after-desktop.png|changed|1280' "$capture_log" \
              || ! grep -Fxq 'after-mobile.png|changed|390' "$capture_log"; }; then
              score=0
              fm='"wrong-visual-state"'
            elif [[ $score -ne 0 ]] && ! grep -q "TC-12: pass" <<<"$out"; then
              score=2
              fm='"no-pass-verdict"'
            fi
          fi ;;
      s13) if ! { grep -q "TC-13: pass" <<<"$out" && grep -q "counter.sh" <<<"$out"; }; then
            score=2
            fm='"missing-anchor"'
          elif grep -Eqi 'usage[-_ ]?log|log[-_ ]?append|append[^<]*log' <<<"$out"; then
            score=0
            fm='"usage-log-output"'
          fi ;;
      *)
        # This harness sets score=8 BEFORE the dispatch, so an unknown id used to come back
        # as a pass that nothing checked. A false pass is worse than a false alarm: nobody
        # investigates it. s8 scored 8 on 2026-08-25 having tested nothing at all.
        score=-1; fm='"ungraded"' ;;
    esac
  fi
  printf '{"id":"%s","score":%s,"failure_mode":%s}\n' "$id" "$score" "$fm"
  if [[ $score -lt 0 ]]; then
    ungraded=$((ungraded + 1))
  else
    total=$((total + 1))
    sumscore=$((sumscore + score))
    [[ $score -eq 0 ]] && catastrophic=$((catastrophic + 1))
  fi
done 3< cases.jsonl

if [[ $total -gt 0 ]]; then
  awk "BEGIN{printf \"mean %.2f over $total cases, $ungraded ungraded, $catastrophic catastrophic ($slice slice)\n\", $sumscore/$total}" >&2
elif [[ $ungraded -gt 0 ]]; then
  echo "no graded cases, $ungraded ungraded ($slice slice)" >&2
else
  echo "no cases in $slice slice" >&2
fi

if [[ $ungraded -gt 0 ]]; then
  exit 2
fi
if [[ $catastrophic -gt 0 ]]; then
  exit 1
fi
