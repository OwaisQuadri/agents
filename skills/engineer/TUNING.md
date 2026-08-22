<!-- TODO(AGNT-0032.T49): record only evidence-backed engineer mutations -->
# engineer: tuning record

The GEPA loop's inputs and outputs for this skill. `SKILL.md` never loads it. Open it when you
tune the map, and not when you walk a ticket.

## accepted mutations

- 2026-08-20, the spine gained a `## routing` section. Owner's ask, with his routing research
  in hand (research/pi-harness-routing-research-fable-opus.md): integrate optimized model routing, and
  optimize this skill first because it eats the most context. The map's semantics did not
  move — same phases, same gates, same walk-back. The section binds four routing rules: a
  sticky T3 orchestrator with a GATE A warning when the session enters on fable, fleet
  frontmatter as the tier authority for dispatches, evidence-only escalation riding the
  existing `loop_counts` counters, and a T5 gate with opus as automatic fallback. The
  dispatch-context rule (compiled context only, never the transcript) moved from folklore to
  the spine. Fleet pins changed in the same pass: anchor-verifier opus→sonnet (highest-volume
  reviewer seat, grades on executed evidence), web-research-summarizer sonnet→haiku (Claude
  Code) / gpt-5.6-luna (pi). Non-holdout mean 9.00 over 15 cases; holdout 9.80 over 5. The
  one low case, g11 at 3 (gate-showme-skipped), scores 3 on the pre-mutation baseline too —
  pre-existing, not introduced. Same day, owner's follow-up: no hardcoded model ids. The
  section now names tiers only, and `config/model-tiers.json` in the agents repo is the one
  file that maps tier to model — the installer compiles it into pi settings, and Claude Code
  aliases get a drift warning. Re-run after the wording change: mean 9.00 over 15, g11
  unchanged.

- 2026-08-11, phase 12 gained gate F. Owner, mid-run on RAG-0001: "when in the engineer skill
  do i get to read the todos? that should be in the implementation human gate". He never did.
  Phase 12 placed the markers, wrote `todo.sh`, committed the stash anchor, and fell straight
  into phase 13's fan-out. Gate B at phase 11 cannot cover it, because neither the markers nor
  `todo.sh` exist at that point. The run that surfaced it supplies the evidence. `todo.sh`'s
  phase-05 FIX step ran a package install on his machine from a script he had never seen. And
  25 markers went into 25 files unreviewed, three of them live modules that a hook executes on
  every prompt in every session. Both arms of the gate test hold. A marker at the wrong site
  sends parallel fresh builders to the wrong place before anyone sees it, and no cheaper stop
  exists. The gate is lettered F, not inserted as a new C. C, D and E are named in phase files,
  in every `state.json.gates` entry, and in finished run records. Renumbering buys alphabetical
  order and costs correctness. The letters are identities, never a firing order.
- 2026-08-17, engineering views now use human-readable task names as their primary labels.
  A live phase-11 plan showed all 55 tasks only as identifiers. The owner said that the
  numbers meant nothing to him. Phase 11, phase 13, and phase 22 now use each `short` value as
  the primary label. Each view keeps the identifier beside the name only for exact lookup.
  This is a defect fix. The user's correction is the reproduction, and the blind `g9` case
  scored 9 of 10. The full run scored 8.77 over 13 non-holdout cases. The holdout run scored
  9.00 over 5 cases.
- 2026-08-10, phase 22 files edges both ways. Owner, at the live CPU-0026 gate D. Injecting a
  ticket computed only that ticket's own dependencies. So a new ticket that blocks three filed
  ones landed with nothing pointing at it. `next-ticket.sh` ranks by transitive dependents, so
  such a ticket scores 0 unlocks and waits for everything else. The graph stays valid, so the
  failure stays silent. Gate D presents dependencies both ways now. The blocking direction
  changes what the human keeps, and they cannot reconstruct that half. The edge rule itself
  lives in `/task-graph`, which the same pass mutated.
- 2026-08-10, phase 07 gained the call-stack outline. The owner's note asked for an outline of
  the implementation's call stacks. Each level names its input and output types, its errors, and
  its side-effects. The note lives in `~/Documents/notes/ai_tools.md`. It extends 07 rather than
  adding a 24th phase. Phase 07 already owns each signature's contract, and the outline traces
  that contract across call paths. A new phase would renumber 08 to 23. That breaks every phase
  number this map leans on. Those are the walk-back integer ranges and the phase-12 anchor.
  They are also the 13 to 16 span, phase 17's 06 to 12 walk, and the gates at 11, 20, 22, and
  23. `call-stacks.md` joins gate B's bundle, because the user reads it before the build.
- 2026-08-07, the run dir left git. The owner said this during CPU-0003's phase 23:
  "i dont think we should be committing ANY ticket specific artifacts". Three ticket dirs had
  reached a product repository's history. The map leaked one run's scratch into every PR it
  opened. Phase 01 gitignores `.map/<ID>/` before it exists. Phase commits became
  `--allow-empty` markers. Phase 16 dropped its `:(exclude).map`, since gitignore gives the same
  guarantee. Phase 23 checks `git ls-files` before the PR. The repo-level map stays tracked,
  because it holds shared state and not run scratch.
- 2026-08-07, landing is not always a PR. The same repository had no git remote, and the owner
  chose a local squash merge into main. Phase 23 step 7 carries both routes now. Phase 01's
  resolved-to-done sweep dropped to a safety net. A squash merge hides from
  `git branch --merged`, because the squashed commit is new and the branch tip never becomes an
  ancestor, so the sweep never fires. Whoever merges flips the ticket at merge time. One more
  lesson from that run: untracking a run dir on the branch means a merge DELETES those files
  from the default branch. Back up and count-verify first.
- 2026-08-07, two walk-back corrections, dictated at the live MJLS-0021 run. First, walk-forward
  runs consecutively. The orchestrator proposed 02, 05, 07 and up, and it called 03, 04, and 06
  unaffected. Step 3 enumerates the range now, and it names "the finding doesn't touch phase 04"
  as a non-reason. Second, `loop_counts` belongs to one walk and resets when a walk opens. One
  walk's two plan reviews sat on the counter when the next walk opened, so the run halted on a
  thrash that never happened. A blind judge, grade 6, then caught that both first drafts broke
  things. The cap counts cycles inside a walk now, and `walkback.jsonl`'s open lines carry the
  run-scoped backstop. Phase 17's plan-only 06 to 12 walk sits outside "every integer in
  between". An empty-`changed` visit carries a mandatory `note`, and the close line asserts it,
  so the arithmetic gates it. A resume into an open walk never resets. Mid-walk recursion
  continues the walk and never starts one. Walk-forward reads every visited phase's file. The
  judge endorsed leaving 13, 14, 15, 17, and 19 alone. It asked for the invariant behind that
  decline: no phase file owns a counter. That made phase 11's inline note redundant.
- 2026-08-06, gate refinement round 2, at the live CPU-0003 run. Gate UX became unconditional,
  so it fires at the end of 03 with or without a web search. The user picks the direction, and
  the phase writes `state.json.gates.UX`. Phase 21's candidate pool gained a mandatory first
  source, the run's own accumulation. Its web scan gained a second angle beside bleeding-edge.
- 2026-08-06, the gate audit. A run had filed 12 tickets unreviewed. Gate D landed at 22, before
  any roadmap write, because ids are immutable and the filed set is next scope. Gate E landed at
  23, the push, once a check confirmed that `/create-pr` does not gate. Gate B widened to carry
  the chosen interaction pattern, which had been reaching implementation unshown. The audit
  declined gates at 16 and at 13, 15, 17, and 19. Phase 16 is local and rewindable, and the rest
  already escalate on their caps. A blind judge, grade 7, drove five more. Every gate writes
  `state.json.gates.<letter>`, so it leaves evidence and not a claim. Phase 23 requires D's
  entry. Gate E gained the unrecoverable-push rule, since the repo bans force-push. The gate
  test grew the second arm that produced those declines. Phase 22 gained a re-entry reconcile,
  so a redo cannot mint a second permanent id for the same work.
- 2026-08-06, three user corrections at one live gate B. The phase-03 gate presents an
  experience brief and never a raw findings dump, and the first delivery was a term-definition
  list. "Prior inspiration" means what has inspired the user before when he designs interfaces,
  and not a shipped example, and the second delivery guessed generic products. His history
  anchors one direction, and the brief always offers options beyond it, including one candidate
  that is new to him. One he adopts appends to `.map/inspiration.md`.
- 2026-08-05, phase 03 gained a conditional human gate. A web search in the UX phase always
  stops for the user before anyone drafts options. The owner directed it through ai-author, so
  it bypassed the loop by authority.
- 2026-08-03, authored. The user designed the 23-phase map, and it absorbed the prime-agent
  mechanics. Same-day blind-judge fixes, grade 6: phase 16 stashes with `:(exclude).map`, so the
  ledgers survive for 17 and ride the phase-16 commit. Phase-17 walks run plan-only over 06 to
  12. The phase-12 anchor freezes while a stash stays outstanding. Phase 18's content check
  reads untracked files, and its suite rerun uses fresh testers. The panel re-attacks after
  fixes. Resume honours open walks. Take-over's phase 12 marks only unimplemented sites. Plan
  reviews got their own counter.

## 2026-08-20 — gate E route: PR by default

Mutation: phase 23's landing route is chosen by `git remote`. A project WITH a remote lands
through a PR; only a repo with no remote lands by local squash merge. This reverses the
2026-08-07 default ("merge locally into main (squash always)"), on the owner's instruction
during the AGNT-0015 run: "gate e will run through PR as is the new standard for git projects
with a remote". Propagated to SKILL.md's OUT line, its phase-index row for 23 and its gate E
sentence, to phase 23's JOB, OUT, step 5 and step 7, and to phase 01's close-out sweep, which
previously justified itself with "the owner squash-merges always" and now names which route
each half of the sweep serves. Path used: owner instruction, not a harness win — no eval case
measures which route a run picks, and inventing one would measure the instruction rather than
the behaviour.
