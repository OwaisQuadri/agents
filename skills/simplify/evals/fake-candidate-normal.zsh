#!/bin/zsh
set -euo pipefail

workspace=${SIMPLIFY_EVAL_WORKSPACE:?}
case_id=${SIMPLIFY_EVAL_CASE_ID:?}
expected_skill_sha=${SIMPLIFY_EVAL_EXPECTED_SKILL_SHA:?}
[[ "${PWD:A}" == "${workspace:A}" ]]
[[ "$(shasum -a 256 "$workspace/.candidate/SKILL.md" | cut -d ' ' -f 1)" == "$expected_skill_sha" ]]
for contained_path in "$HOME" "$PI_CODING_AGENT_DIR" "$PI_CONFIG_DIR" "$PI_CODING_AGENT_SESSION_DIR" "$XDG_CONFIG_HOME" "$XDG_CACHE_HOME" "$XDG_DATA_HOME" "$TMPDIR"; do
  contained_path=${contained_path:A}
  [[ "$contained_path" == "${workspace:A}"/* ]]
done
args=" $* "
for fence in '--no-session' '--no-skills' '--no-extensions' '--no-prompt-templates' '--no-themes' '--no-context-files' '--no-approve'; do
  [[ "$args" == *" $fence "* ]]
done
[[ "$args" == *" --session-dir $workspace/.pi/session "* ]]
[[ "$args" == *" --skill $workspace/.candidate/SKILL.md "* ]]
[[ "$args" == *" --tools read,write,edit,bash "* ]]
for hidden_name in SIMPLIFY_EVAL_HIDDEN_RUBRIC SIMPLIFY_EVAL_HIDDEN_CASES SIMPLIFY_EVAL_HIDDEN_HOLDOUT SIMPLIFY_EVAL_HIDDEN_SOURCE SIMPLIFY_EVAL_HIDDEN_HOME; do
  hidden_path=${(P)hidden_name}
  if hidden_text=$(/bin/cat "$hidden_path" 2>&1); then
    exit 81
  fi
done

case "$case_id" in
  s1)
    ./test.zsh > .harness/before.txt
    print -r -- 'import { existsSync } from "node:fs";

function calculateFee(amount) {
  return Math.round(amount * 0.05);
}

export function quote(orders) {
  return orders.map((order) => ({
    id: order.id,
    total: order.amount + calculateFee(order.amount),
  }));
}

if (existsSync(new URL(import.meta.url))) {
  console.log(JSON.stringify(quote([
    { id: "a", kind: "retail", amount: 100 },
    { id: "b", kind: "wholesale", amount: 80 },
  ])));
}' > pricing.ts
    ./test.zsh > .harness/after.txt
    ./format-check.zsh
    ./static-check.zsh
    print -r -- 'Scope: pricing.ts and the quote output boundary.
Baseline: ./test.zsh passed with [{"id":"a","total":105},{"id":"b","total":84}].
Reduced: Removed the unused readFileSync import, merged duplicate fee helpers, and replaced the manual loop with Array.map.
Kept: The quote output shape and descriptive names.
Final: ./test.zsh passed with the same output. format-check.zsh and static-check.zsh passed.' > REPORT.md
    ;;
  s2)
    ./test.zsh > .harness/before.txt
    ./test.zsh > .harness/after.txt
    print -r -- 'Scope: policy.rs and the policy parsing error boundary.
Baseline: ./test.zsh passed with the expected retail and invalid-policy output.
Reduced: None.
Kept: parse_customer_policy names the domain operation and keeps it out of the long match expression. A line-count target does not justify inlining it.
Final: ./test.zsh passed with the same output. rustc completed without errors.' > REPORT.md
    ;;
  s3)
    set +e
    ./test.zsh > .harness/before.txt 2>&1
    baseline_status=$?
    set -e
    [[ "$baseline_status" != 0 ]]
    print -r -- 'Scope: repository.py and the unavailable test database boundary.
Baseline: ./test.zsh failed: database unavailable: connection refused.
Reduced: No simplify edit. The pass stopped after the failing baseline.
Kept: All source code.
Final: Not run because the baseline failed.' > REPORT.md
    ;;
  s4)
    ./test.zsh > .harness/before.txt
    ./test.zsh > .harness/after.txt
    print -r -- 'Scope: decoder.mjs and the external JSON request boundary.
Baseline: ./test.zsh passed for valid and missing names.
Reduced: None.
Kept: The runtime name check guards untyped external input before the internal required type applies.
Final: ./test.zsh passed with the same output. node syntax checks passed.' > REPORT.md
    ;;
  s5)
    ./test.zsh > .harness/before.txt
    grep -R configured_handler . --exclude=REPORT.md > .harness/references.txt
    ./test.zsh > .harness/after.txt
    print -r -- 'Scope: handlers.py, framework.json, test.py, and the configured framework entry point.
Baseline: ./test.zsh passed with handled:event.
Reduced: None.
Kept: configured_handler because the reference search found its framework.json configuration string.
Final: ./test.zsh passed with handled:event. Python import checks passed.' > REPORT.md
    ;;
  s6)
    ./test.zsh > .harness/before.txt
    ./test.zsh > .harness/after.txt
    print -r -- 'Scope: status.mjs and all four account-state branches.
Baseline: ./test.zsh passed for every branch.
Reduced: None.
Kept: The readable branches. Statement packing, one-letter names, and a nested conditional expression are code golf.
Final: ./test.zsh passed with the same output. node syntax checks passed.' > REPORT.md
    ;;
  s7)
    ./test.zsh > .harness/before.txt
    print -r -- 'export function normalizeName(value) {
  return value.trim().toLowerCase();
}

export function displayName(value) {
  return value.charAt(0).toUpperCase() + value.slice(1);
}' > shared.mjs
    print -r -- 'import { displayName, normalizeName } from "./shared.mjs";

export function customerLabel(rawName) {
  return `customer:${displayName(normalizeName(rawName))}`;
}' > customer.mjs
    print -r -- 'import { displayName, normalizeName } from "./shared.mjs";

export function adminLabel(rawName) {
  return `admin:${displayName(normalizeName(rawName))}`;
}' > admin.mjs
    ./test.zsh > .harness/after.txt
    print -r -- 'Scope: shared.mjs, customer.mjs, admin.mjs, api.mjs, both callers, and the API interface boundary. The labels are the observable side effect.
Baseline: ./test.zsh passed for the admin and customer call paths.
Reduced: Merged the repeated displayName logic into shared.mjs and updated both callers.
Kept: normalizeName and the API response boundary.
Final: ./test.zsh passed with the same output. node syntax checks passed.' > REPORT.md
    ;;
  *) exit 64 ;;
esac

print -r -- '{"type":"result","status":"complete","text":"Executed the loaded simplify skill. REPORT.md contains the scope, baseline, reductions, retained candidates, and final checks."}'
