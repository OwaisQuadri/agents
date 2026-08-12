# engineer: tuning record

The GEPA loop's inputs and outputs for this skill. `SKILL.md` never loads it. Open it when you
tune the map, and not when you walk a ticket.

## accepted mutations

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
