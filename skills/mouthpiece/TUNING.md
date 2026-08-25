# mouthpiece: tuning record

The GEPA loop's inputs and outputs for this skill. `SKILL.md` never loads it. Open it when you
tune this skill, and not when you write a message with it.

## accepted mutations

- 2026-08-11, the bro pass. Owais asked that the skill run `/bro` on each output. Two logged
  uses say why: he could not parse `break panel` on 2026-08-07, and he caught `GEPA` used bare
  earlier that day. The skill already asked for an expansion, and nothing graded it. The
  reproduction, kept here because no clone holds another copy, on one line so the checker reads
  it as exact information:

  "The sweep is done. I dispatched the checker over every register and the run is idempotent, so a rerun changes nothing. Two rules are orthogonal to the cap, and one invariant still fails on the RAG path. The PR is open and CI is green. Next: tell me whether to widen the JSON schema."

  It passed all 18 rules of the old register and failed 2 of the 14 bro rules. The register
  borrows both now, so it carries 20 rules and that text passes 18. Defect-fix path, so it
  ships on the reproduction and never on a mean. The literal reading did not ship, and that
  reading has bro rewrite each message. Bro replaces the register it rewrites, so it would void
  the cap and every voice rule.
- 2026-08-11, four false positives in the borrowed abbreviation rule, each in a real message.
  `GATE` read as an abbreviation, and three messages quote a gate name as a fact. The rule
  demanded the acronym first, so it rejected `Model Context Protocol (MCP)`, which cost five
  more messages. It never carried its own "at first use", so a term expanded once and reused
  short failed on the second use, in two messages. A hyphenated identifier flagged its leading
  run, and `CPU-0003`, `ABCD-1204`, and `ASD-STE100` all did it with no in-prose escape.
- 2026-08-11, a fifth one, caught by a fresh review before merge. Accepting any `word (ACRONYM)`
  as an expansion made the rule weaker than the version it borrowed: `The run is green (CI). CI
  is red now.` passed here and failed on `main`. One of the last N words must now share the
  run's initial. Matching every word's initial in order is too strict, because
  `Genetic-Pareto prompt evolution (GEPA)` and `JavaScript Object Notation (JSON)` both fail it.

## what the pass measured, and what it did not

A re-grade of saved messages carries no judge noise, so it beats every mean here. The final
checker read 88 real messages as four samples of 22. Take the 16 cases that predate the pass.
Both arms break a plain-words rule in 0 of 32 messages there, so no pre-existing case measures
this defect. Over the 6 cases the pass added, the pre-mutation file breaks one in 5 of 12
messages and this file in 0 of 12.

So the whole measured signal sits in cases written during the pass, from the reproduction.
They are the next pass's exam, and not this one's proof.

No mean here is a win. The clean pair ran under one checker build. It scored 5.60 and 4.86 for
the pre-mutation file over 15 non-holdout and 7 holdout cases, against 6.33 and 5.71. Those
gains of 0.73 and 0.85 clear the 0.6 line an earlier pass called noise. They still fail its
other ask of 3 runs per side.

Now split the holdout slice by case age. The 5 old cases tie exactly at 5.40, so step 4
clause 3 is unmet on numbers. The first pair, before the narrowings, reads backwards at 4.67 and 4.86
against 4.47 and 4.71. All 8 abbreviation failures in it were false positives.

## open, measured, not yet fixed

- Multi-word repo jargon survives, and the word list cannot be the fix. `break panel` is the
  term the human could not parse, and both m21 messages left it bare. The engineer skill owns
  that name for a phase, so the message keeps it as a fact. The rubric grades a dropped
  load-bearing fact as catastrophic. A list entry would demand the phrase go away, so no
  message could satisfy both. The instrument has to see a gloss, and a word list cannot.
- A quoted term of art escapes both rules, because the masker hides a double-quoted span. That
  is right for a real quotation and it is also a way around the rule. The hole predates the
  pass and reaches every word ban in the tool.
- The expansion rule cannot see order, so a short form used first and expanded later passes.
  The reader can still decode that message, which is why the weaker form shipped.
- The initial-sharing guard is deliberately weak. A nearby word that starts with the same
  letter still licenses a parenthesis, as in "Continuous checks ran (CI)".
- The cap exemption in the hard rules over-promises. A bare number, a bare identifier, a
  table's pipes, and the first segment of a relative path all count against the 600. Four
  things come out: a fenced span, a backtick span, a double-quoted span, and a path of two or
  more segments. Single quotes do not protect an error string.
- A number-heavy report hits the 6-sentence paragraph cap in `docs/prompt-style.md` before the
  600 characters. A gloss adds sentences, so the bro pass makes that worse.
- The pass evidence lives under `.context/`, which a local exclude drops, so the 88 graded
  messages and the per-case scores die with the session. Only the reproduction above survives.

## 2026-08-24 — list cap raised to 5

Mutation: `SKILL.md:95` capped lists at 3 while `tools/ste-check/src/mouthpiece.rs:9`
(`LIST_CAP`) has always enforced 5. Owner ruled for 5. The eval paragraph also claimed the
checker does not grade the list cap; it does, through the `numbered lists capped at 5` rule,
so that sentence moved the cap into the graded list and left only the backtick restriction
and the negation stack ungraded. Evidence: 14 logged lines where SKILL.md and the checker
disagreed, and case m10, unscorable as written because it asks for a 5-step walkthrough
against a skill capping at 3. Path used: owner ruling on a contradiction, plus a defect fix
on a false statement about the checker. Not a harness win.

Open, measured: m10, m12 and m22 were authored against the cap of 3. m12 expects 3
explicitly. They need re-authoring by a case author blind to this change.

## 2026-08-24 — the character cap is gone

Mutation: the 600-character cap is removed from the register and from the checker.
`tools/ste-check` drops `CHAR_CAP`, `char_cap()` and its `RULES` entry, so the register now
carries 19 rules rather than 20, and the SKILL.md eval paragraph says 19. The hard rule that
replaces it: length is bounded by the facts, and by nothing else. Cap references in
`skills/byline/SKILL.md` and `skills/bro/SKILL.md` are reworded, since both defined
themselves against it.

Owner instruction, and ticket AGNT-0083 already carried it. The logs agree: 22 lines record
the cap as the binding constraint that cut a fact, and 9 record invented specifics under
that same pressure ("m6 invented commit 7ec9b70 which is nowhere in the case input"). The
proposed mutation "the cap yields to a fact" is now moot — the cause is gone rather than
ranked.

Open, owed: cases m7, m8 and m21 phrase their expectations as "sits outside the cap", which
is now vacuous rather than wrong. m12 still expects a list capped at 3. AGNT-0083 also asks
for three new cases: a concise reply, a necessarily long one, and a verbose one that fails
for a reason other than length. All of it needs a case author blind to this change.
