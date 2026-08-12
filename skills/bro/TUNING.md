# bro: tuning record

The GEPA loop's inputs and outputs for this skill. `SKILL.md` never loads it. Open it when you
tune this skill, and not when you rewrite a message with it.

## accepted mutations

- 2026-08-11, the add-nothing rule. The seed draft split upstream rule 1 into two bullets and
  lost "never add new information", and the length bullet then invited padding. Four of nine
  cases failed on added content, and `r4` scored 2 for padding a message that already read
  plainly. The non-holdout mean went 7.33 to 8.22 over the same nine cases, and `r4` went 2 to
  9. The case list predates the mutation, so no expect paraphrases it.
- 2026-08-11, the plain-words rules went to the mouthpiece register, and that run measured four
  false positives in `bare_acronym`. `GATE` joined `NOT_ACRONYMS`, because the engineer skill
  names its gates GATE A through GATE E and three messages quote one as a fact. Both expansion
  orders count now, since demanding the acronym first rejected `Model Context Protocol (MCP)`
  and cost five more messages. The rule also never carried its own "at first use". A term
  expanded once and reused short failed on the second use, on `STE` and on `GEPA`. A hyphenated
  identifier no longer flags its leading run, which `CPU-0003`, `ABCD-1204`, and `ASD-STE100`
  all did with no in-prose escape. All 8 abbreviation failures over 22 real messages were one
  of the first two, and none was a real miss.
- 2026-08-11, a parenthesis alone is not an expansion. A fresh review caught it before merge:
  accepting any `word (ACRONYM)` left the rule weaker than before, so `The run is green (CI).
  CI is red now.` passed here and failed on `main`. One of the last N words must share the
  run's initial now, where N is the run length.

## open, measured, not yet fixed

- Added detail is still the top failure mode after the add-nothing fix. Five cases name a form
  of it. They are "of records" in `r1`, a validation gloss in `r2`, and stage descriptions in
  `r3`. A causal overclaim in `r5` and a closing gloss in `r8` finish the list. The rule
  forbids the state and never tells
  the writer what to do where a plain word needs a subject. That gap is the next mutation.
- The holdout slice holds one measurement, 8.25 after the fix, with no case under 8. No pre-fix
  holdout run exists, so that number is a level and not a win.
- `JARGON` is a seed list of 24 single words. It holds no phrase, so multi-word repo jargon
  passes. The mouthpiece record carries why a list entry cannot fix that.
- A quoted term of art escapes every rule here, because the masker hides a double-quoted span.
  The unpushed commit `3c26536` records the same hole.
