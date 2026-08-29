# ai-author: tuning record

The GEPA loop's own inputs and outputs for this skill. Step 1 reads the open list below, and
step 5 records an accepted mutation here. Nothing here is instruction, so `SKILL.md` never
loads it. Open it when you tune this skill, and not when you run it.

## accepted mutations

- 2026-08-10, arrival routing (`## what arrived?`). Five of the first six blind votes reported
  the same failure. An invocation that was no authoring job walked into the should-it-exist
  tree, and the operator improvised a verdict. **UNMEASURED.** The first write-up claimed a
  stub score of 5.50 against 7.80, and the next judge showed the claim is circular, so it is
  retracted. The cases came after the mutation, from an author who could read it.
- 2026-08-10, the defect-fix acceptance path (step 4). Step 4 assumed the harness can see
  every mutation. For a fix no case measures, a tie was then the only compliant outcome, so
  shipping meant a tie reported as a win. **UNMEASURED**, retracted for the reason above.
- 2026-08-10, the deferred-verdict destination. The routing mutation forbade the destination
  that does not count and named none that does. Two signals said so, the fenced case author
  and a `d1` score of 5 against 7.
- 2026-08-10, the harness itself. The skill had no `evals/` while telling every artifact that
  no harness means not done. A fenced author built 15 cases, held 5 out, and proved
  discrimination against 8 defect stubs. Two stubs raise the visible mean while a holdout case
  catches them.
- 2026-08-19, the strict-YAML frontmatter rule (authoring contract). The engineer skill's
  description held a bare `: ` in a plain scalar. Claude Code's lenient parser accepted it,
  and pi's strict `yaml` loader rejected the whole skill ("Nested mappings are not allowed
  in compact mappings"). One observed failure in the wild. The rule: a frontmatter value
  containing `: ` is written as a `>-` block scalar, checked with the strict parser before
  shipping. **UNMEASURED** — no harness case grades it yet.
- 2026-08-10, the answer-key fence. The judge's own fix, taken over banning self-tuning,
  because the weaker rule catches more. Cases written with sight of the candidate were the
  failure, and every author can write those.
- 2026-08-26, the bounded session-evidence sweep for AGNT-0030. A fenced case author saw the
  demonstrated gap and requirements, but not the candidate text. The incumbent scored 7.09
  over 11 non-holdout cases and 7.17 over 6 holdout cases. The candidate scored 7.27 and 8.50.
  The added cases distinguish the missing sweep from a bounded, redacted, zero-valid procedure.
- 2026-08-29, Pareto-frontier candidate selection (GEPA loop step 2). **UNMEASURED** — no
  harness case exercises this yet, and no artifact currently has ≥2 competing candidates on
  record to test it against. Research grounding: the actual GEPA paper (arXiv:2507.19457,
  ICLR 2026 Oral) selects the next mutation's parent from the Pareto frontier of past
  candidates, not always the single incumbent — a candidate survives if it's the best-known
  scorer on at least one eval case, even with a lower mean overall, so specialist variants
  aren't discarded before their ideas can be recombined. This repo's `run.sh` already computes
  per-case scores every Test run and discarded them the instant the run exited; the change adds
  `evals/frontier.jsonl` (score vectors) + `evals/frontier/<id>.md` (full candidate text),
  both git-tracked (reasoning: they only ever hold scores against already-tracked synthetic
  cases plus draft variants of the artifact's own already-tracked definition file — no live
  session content, unlike the gitignored `logs/`/`votes/`). Step 2 (Propose) now samples its
  mutation parent from the frontier once an artifact has ≥2 non-incumbent frontier members;
  below that threshold it behaves exactly as before. Step 4's shipping rule (mean win + holdout
  + no new catastrophic + weakest-wins-on-tie) is unchanged — this only changes what step 2
  starts mutating from. Next real signal: the first artifact to accumulate 2+ tested
  candidates under the new template proves or disproves this in practice.

## deferred verdicts

Verdicts this skill reached and did not execute. Case `d1` grades whether a verdict lands
somewhere that tracks it. This heading is that place, and a log line is not.

- No true pre/post-SKILL lifecycle hook exists in `@earendil-works/pi-coding-agent`'s
  extension API. Confirmed by reading the installed package's own type definitions
  (`~/.bun/install/cache/@earendil-works/pi-coding-agent@0.84.2@@@1/dist/core/extensions/types.d.ts`)
  and every extension already in `pi/extensions/`: the real event vocabulary is
  `tool_call`, `tool_execution_start/update/end`, `agent_start/end/settled`,
  `before_agent_start`, `turn_start/end`, `message_start/update/end`,
  `session_start/shutdown/before_compact/before_fork/before_switch/before_tree/compact/
  tree/info_changed`, `model_select`, `thinking_level_select`,
  `before_provider_request/headers`, `after_provider_response`, `resources_discover`.
  `skill` appears only as a config field (`skillPaths?: string[]`) and in a doc comment,
  never as an event. GEPA loop step 2 (Propose) already routes a mechanizable finding to
  PROSE, a CHECKER, or a PI EXTENSION — but if that finding needs skill-lifecycle
  granularity specifically (not just a `tool_call` filtered to a `Read` of a `SKILL.md`,
  the nearest approximation `pi/extensions/logpath-guard.ts`'s pattern could build), no
  platform hook exists to build it against. File an upstream ticket against
  `@earendil-works/pi-coding-agent` only once a real Propose pass reaches a
  PI-EXTENSION verdict the tool_call approximation can't satisfy — not speculatively; the
  same "narrowing needs an observed false positive" rule this repo applies to prose
  rules applies here in reverse to requesting a new platform capability. Owner: whichever
  future GEPA pass hits this wall first.
- Propagate the session preamble to the symlink target. A dispatched subagent listed the 11
  pre-change headings and not the new one, so a workspace edit alone does not reach a
  background session. Owner: the merge of that branch.
- Branch 1 names three destinations and no rule for picking one. The operator invented the
  comparison and credited it to the skill. A blind judge graded that run 5 for it.
- The `gate` field has no documentation. The open list calls it the highest-value mutation,
  and the 2026-08-11 run emitted one ad hoc instead of adding it to the format.
- Step 4 clause 3 fights step 4's own same-pass case author. The mouthpiece run gained 0.85
  over the whole holdout slice. It gained 0.00 over the 5 holdout cases that predate the pass,
  on per-case scores of 9, 2, 8, 4, and 4 in both arms. The 2 new holdout cases carry all of
  it. The fix reports the arms over pre-existing cases apart from the new ones. It also judges
  clause 3 on the old holdout cases only. `templates/eval-harness.md` has the same hole.
- Step 3 never freezes the instrument. It names the same cases and nothing else. The
  mouthpiece run graded its two arms with two checker builds, and that pair read backwards.
  `evals/run.sh` also falls back to a second model and records neither choice.
- The narrowing licence says "logged" and should say "observed". The mouthpiece run narrowed
  three times on false positives seen in harness output, which the strict reading forbids.
- An accepted mutation's evidence lives where git cannot keep it. That run wrote its 88 graded
  messages and its per-case scores under `.context/`, which a local exclude drops. The
  repository ignores `logs/` and `votes/` as well, so no clone can recompute a number.
- The live clone carries a deferred verdict this file does not. Commit `3c26536` sits unpushed
  in `~/Documents/agents`. It records that `find_words` misses a quoted term, so a writer can
  evade every word ban. The mouthpiece run borrowed a rule that runs on `find_words` and never
  saw the entry. Owner: the merge of `3c26536`.

## open, measured, not yet fixed

The harness scores the incumbent at 7.80 on both slices, and it does not sweep its own exam.

- `c1` scores 4. The contract states the go-live condition and never asks for the evidence, so
  "the evals were authored" reads as "the evals passed". A vote asked for a gate field in July.
- The holdout gate was never shown met for either 2026-08-10 mutation. Both reported
  non-holdout figures only, and one quoted a holdout case inside a non-holdout claim.
- Tree rule 1 ends at "update it" and never routes to the GEPA loop, though the arrival branch
  routes that same state there. `b4` sits at 7. One line.
- `h1` at 6 and `b4` at 7: branches this skill states and does not make checkable.
- The one-sentence test, vote aggregation, the loop's trigger threshold, and "no churn on
  noise" are unobservable as written. No run can be said to have honoured or skipped them.
