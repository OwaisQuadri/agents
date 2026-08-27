#!/bin/zsh
set -euo pipefail

workspace=${LADDER_EVAL_WORKSPACE:?}
id=${LADDER_EVAL_CASE_ID:?}
args=" $* "
[[ "${PWD:A}" == "${workspace:A}" ]] || exit 70
for fence in --no-session --no-skills --no-extensions --no-prompt-templates --no-themes --no-context-files --no-approve; do
  [[ "$args" == *" $fence "* ]] || exit 71
done
[[ "$args" == *" --skill $workspace/.candidate/SKILL.md "* ]] || exit 72
[[ "$(shasum -a 256 "$workspace/.candidate/SKILL.md" | cut -d ' ' -f 1)" == "$LADDER_EVAL_EXPECTED_SKILL_SHA" ]] || exit 73
[[ ! -e "$workspace/cases.jsonl" && ! -e "$workspace/rubric.md" && ! -e "$workspace/evals" ]] || exit 74
for hidden in "$LADDER_EVAL_HIDDEN_RUBRIC" "$LADDER_EVAL_HIDDEN_CASES" "$LADDER_EVAL_HIDDEN_HOLDOUT" "$LADDER_EVAL_HIDDEN_SOURCE" "$LADDER_EVAL_HIDDEN_HOME" "$LADDER_EVAL_HIDDEN_SNAPSHOT"; do
  [[ -z "$hidden" ]] || ! /bin/cat "$hidden" >/dev/null 2>&1 || exit 75
done

write_wrong() {
  print -r -- '# Interference log

| Current habit | Technology reality |
|---|---|
| Implicit ownership | Ownership is explicit. |
| Familiar defaults | Defaults differ. |
| Framework conventions | The primary source controls. |
| Runtime recovery | Compile-time checks can reject the program. |
| Recognition | Free recall proves retrieval. |
| Guided edits | Cold reconstruction proves skill. |
| Local code | A merged foreign patch proves codebase skill. |
| Short projects | Maintenance reveals long-term costs. |

Silent row: framework conventions can look like an unrelated defect.' > "$workspace/learning/wrong.md"
}

case "$id" in
  c1)
    write_wrong
    print -r -- '# Rust ladder

## Provenance

This ladder merges the agreed six-step plan in `PRIOR-PLAN.md`, dated 2026-08-23. Rungs map to P1 through P6.

## Time horizon

Full mastery has no deadline.

## Dimension table

| Dimension | Level | Transfer |
|---|---:|---|
| Enums with payloads | R4 | Swift enums transfer. |
| Value semantics | R4 | Swift structs transfer. |
| Optional and Result | R3 | Swift Optional and Dart Result patterns transfer. |
| Protocols as traits | R3 | Swift protocols transfer. |
| Ownership | R0 | This rule is new. |
| Borrowing | R0 | This rule is new. |
| Lifetimes | R0 | Explicit relationships are new. |
| Cargo workflow | R1 | Package tooling partly transfers. |

## Red cells

Ownership, borrowing, and lifetimes are the R0 curriculum.

## Adapted loop

Primary sources: The Rust Reference and the Rust standard-library source. Predict first, then use `rustc` as the ground-truth oracle. Reimplement without reference. Use a fresh inverted-Feynman examiner. Finish with free recall.

## Rung table

| Rung | Artifact | Dimension | Prior plan |
|---:|---|---|---|
| 1 | A port that builds and passes tests | transfer boundary | P1 |
| 2 | An ownership probe that compiles | ownership | P2 |
| 3 | A lifetime test suite that passes | lifetimes | P3 |
| 4 | A cold timed rebuild that passes | borrowing | P4 |
| 5 | A merged patch to a Rust dependency | foreign code | P5 |
| 6 | A maintained Rust project with monthly releases | integration | P6 |

## Cadence

Use a 3:4:2:1 ratio for primary sources, builds, adversarial work, and old free recall.

## Interference log seed

Use `wrong.md`. It has eight rows and names the silent row.

## Adversary rule

The model explains, grades, and attacks. It never authors.' > "$workspace/learning/LADDER.md"
    print -r -- 'Merged the indexed six-step Rust plan into learning/LADDER.md and seeded learning/wrong.md.'
    ;;
  c2)
    write_wrong
    print -r -- '# React Native ladder

## Provenance

No prior plan was found. This ladder uses the supplied Swift and Flutter experience, dated 2026-08-23.

## Time horizon

The deadline box is three days before the live 60-minute no-artificial-intelligence interview.

## Dimension table

| Dimension | Level | Transfer |
|---|---:|---|
| Mobile architecture | R4 | Swift and Flutter pay for it. |
| State concepts | R4 | Flutter state transfers. |
| Layout | R4 | Both mobile stacks transfer. |
| JavaScript idiom | R0 | Language behavior is new. |
| React render model | R0 | Rendering is new. |
| Dependencies and keys | R0 | Identity rules are new. |
| Native bridge | R1 | Platform knowledge partly transfers. |
| Metro tooling | R1 | Build concepts transfer. |

## Red cells

JavaScript idiom and the React render, dependencies, and keys model are the R0 curriculum.

## Adapted loop

Primary sources: React documentation and the React Native source repository. Predict first, then use the test runner and renderer as oracles. Reimplement without reference. Use a fresh examiner. Finish with free recall.

## Rung table

| Rung | Artifact | Dimension | Timing |
|---:|---|---|---|
| 1 | A JavaScript behavior probe with passing tests | JavaScript | Day 1 |
| 2 | A rendered list with dependency and key tests | React model | Day 1 |
| 3 | A timed cold rebuild that runs in 60 minutes | pressure | Day 2 |
| 4 | A recorded no-reference mock interview with a passing app | integration | Day 3 |

### After the deadline

| Rung | Artifact | Dimension | Timing |
|---:|---|---|---|
| 5 | A merged patch to a React Native dependency | foreign code | Later |
| 6 | A maintained React Native project with monthly releases | integration | Long term |

## Cadence

Use daily blocks in the 3:4:2:1 ratio.

## Interference log seed

Use `wrong.md`. It has eight rows and names the silent row.

## Adversary rule

The model explains, grades, and attacks. It never authors.' > "$workspace/learning/LADDER.md"
    print -r -- 'Wrote the three-day React Native ladder and retained the later mastery rungs.'
    ;;
  c3)
    print -r -- 'I did not find the remembered Kubernetes plan. The rag store does not index claude.ai web chats. Paste the agreed plan, and I will merge it instead of reconstructing it.'
    ;;
  c4)
    write_wrong
    print -r -- '# Zig ladder

## Provenance

No prior plan was found. This ladder uses the supplied C and Rust experience, dated 2026-08-23.

## Time horizon

Full mastery has no deadline.

## Dimension table

| Dimension | Level | Transfer |
|---|---:|---|
| Manual memory management | R4 | C pays for it. |
| Pointer safety | R3 | C and Rust transfer. |
| Error unions | R3 | Rust Result transfers. |
| Build tooling | R1 | Concepts transfer, syntax does not. |
| comptime | R0 | Compile-time execution is new. |
| Allocator-passing convention | R0 | Explicit allocator flow is new. |
| Packed layout | R2 | C layout transfers. |
| C interoperability | R4 | C pays for it. |

## Red cells

`comptime` and the allocator-passing convention are the R0 curriculum. Syntax is not a red cell.

## Adapted loop

Primary sources: `https://ziglang.org/documentation/0.14.1/` and `https://github.com/ziglang/zig/tree/0.14.1/lib/std`. Predict first, then use the Zig compiler as the ground-truth oracle. Reimplement without reference. Use a fresh examiner. Finish with free recall.

## Rung table

| Rung | Artifact | Dimension | Source |
|---:|---|---|---|
| 1 | A comptime probe that builds | comptime | New |
| 2 | An allocator-injected library with passing tests | allocators | New |
| 3 | A timed cold rebuild that passes | both red cells | New |
| 4 | A merged patch to a Zig dependency | foreign code | New |
| 5 | A maintained Zig project with monthly releases | integration | New |

## Cadence

Use a 3:4:2:1 ratio for primary sources, builds, adversarial work, and old free recall.

## Interference log seed

Use `wrong.md`. It has eight rows and names the silent row.

## Adversary rule

The model explains, grades, and attacks. It never authors.' > "$workspace/learning/LADDER.md"
    print -r -- 'Wrote the Zig ladder with compiler and primary-source checks.'
    ;;
  c5)
    print -r -- '`Box<T>` gives one owner heap allocation and supports moving that ownership. `Rc<T>` gives shared single-threaded ownership through reference counting. Use `Box<T>` when one owner is enough. Use `Rc<T>` when several values must own the same allocation.'
    ;;
  c6)
    write_wrong
    print -r -- '# Postgres internals ladder

## Provenance

No prior plan was found. This ladder uses shipped application-backend experience, dated 2026-08-23.

## Time horizon

Full mastery has no deadline.

## Dimension table

| Dimension | Level | Transfer |
|---|---:|---|
| SQL behavior | R3 | Backend work transfers. |
| Transactions | R2 | Application use partly transfers. |
| Query planner | R0 | Planner internals are new. |
| Executor | R0 | Executor internals are new. |
| Storage pages | R0 | Physical representation is new. |
| Write-ahead log | R0 | Recovery internals are new. |
| C codebase workflow | R0 | Foreign source work is new. |
| Operations | R2 | Backend operations transfer. |

## Red cells

The planner, executor, storage, recovery, and C source workflow form the R0 curriculum.

## Adapted loop

Primary sources: PostgreSQL documentation and `https://github.com/postgres/postgres`. Predict first, then use the Postgres test suite and `EXPLAIN ANALYZE` as oracles. Reimplement without reference. Use a fresh examiner. Finish with free recall.

## Rung table

| Rung | Artifact | Dimension | Source |
|---:|---|---|---|
| 1 | A patched Postgres build that runs its tests | source workflow | New |
| 2 | A query plan reproduced with `EXPLAIN ANALYZE` | planner | New |
| 3 | A storage-page decoder with golden tests | storage | New |
| 4 | A timed cold planner probe that passes | planner | New |
| 5 | A merged documentation patch in the Postgres project | foreign code | New |
| 6 | A maintained Postgres extension with monthly releases | integration | New |

## Cadence

Use a 3:4:2:1 ratio for primary sources, builds, adversarial work, and old free recall.

## Interference log seed

Use `wrong.md`. It has eight rows and names the silent row.

## Adversary rule

The model explains, grades, and attacks. It never authors.' > "$workspace/learning/LADDER.md"
    print -r -- 'Wrote the Postgres internals ladder with verifiable artifacts.'
    ;;
  *)
    exit 64
    ;;
esac
