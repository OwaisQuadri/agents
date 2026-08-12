---
name: bro
description: Use when the last reply did not land, because it was too dense, too jargon-heavy, or too formal. Returns the same message again with every term of art swapped for plain words and every fact untouched. Skip when the ask is a new question, which gets a new answer rather than a re-explanation, and skip for prose that ships under the user's name, which /byline owns.
metadata:
  short-description: Re-explain the last reply in plain words
---

# bro

JOB: return your own last message again, with every term of art replaced by plain words
IN:  the user types `/bro`; the material is your most recent assistant message
OUT: the plain-words version, plus the `ste-check --register bro` result on it

## files

- `upstream-SKILL.md` — the original author's instructions, for reference. It is MIT
  licensed, and it comes from `github.com/luchasarie/bro-skill`.

## register

Simplified Technical English, per `docs/prompt-style.md`. Every register in this repo runs
on that base. What bro adds is the de-jargon pass below.

Bro replaces the register of the message it rewrites, and it never stacks on top of it. A
mouthpiece message caps at 600 characters, and the bro version of that same message has no
cap at all. Grade bro output with `--register bro`, and never with the register it replaced.

Casual connectives stay. Byline bans "basically" and "essentially" as hedges, and bro keeps
them, because they are the plain-talk joins this register asks for.

## the pass

1. Read your own most recent message again. That message is the only input, and a new
   question never enters here. A message that already reads plainly comes back nearly
   unchanged, because this pass has nothing to do on it.
2. Replace each term of art with plain words. A fact is not a term of art, so it survives
   step 2 untouched.
3. Expand each abbreviation at first use, then give the plain-words version of it. "DAG
   (directed acyclic graph)" becomes a list of steps where each one waits on the ones
   before it.
4. Flatten the shape. Headings and tables become plain sentences, and a list survives only
   where the original message held real parts.
5. Run `ste-check --register bro`. Fix every flag by substitution, and never by cutting a
   fact.
6. Re-run `ste-check`. Every rule passes, or the message does not ship.

## hard rules

- Re-explain, and never re-answer. A new question earns a new answer, and that answer is
  not this skill's job.
- Never call a tool. You wrote the material, so you already hold it.
- Add nothing. A detail, a cause, or a number the message never stated stays out. A figure
  you work out from two of its numbers is a new number.
- Facts survive verbatim. Every path, command, filename, number, URL (Uniform Resource
  Locator), name, and decision stays exactly as it was.
- Simpler, and not always shorter. An idea that needs room gets the room, and no sentence
  gets padding.
- Answer in the language of the original message.
- A touch of personality is welcome, and a meme is not.
- Where no previous assistant message exists, say there is nothing to simplify yet.

## where it runs

- Any turn where the user types `/bro` after a reply that lost him.
- Never inside another skill's phase. Bro re-explains a finished message, and it never
  writes the message in the first place.
- The plain-words rules travel, and this skill does not. The mouthpiece register grades
  every message on steps 2 and 3 above, under the rule names `plain words, no term of art`
  and `every abbreviation expanded at first use`. A borrowed rule is not a bro run. That
  message keeps its 600-character cap and its own structure rules, and the uncapped rewrite
  stays the `/bro` turn alone.

## evals

`evals/run.sh` lists a candidate rewrite per case in `evals/cases.jsonl`, runs `ste-check`
on it, then grades the content against the case expect with `evals/rubric.md`. `--holdout`
runs the held-out slice. A mechanical failure caps that case at 4, and an altered fact
scores 0. GEPA (Genetic-Pareto prompt evolution) runs read the mean and use the failure
modes as feedback.

## history

- 2026-08-11, the add-nothing rule. The seed draft split upstream rule 1 into two
  bullets and lost "never add new information". The length bullet then invited padding on
  top of that. The first harness run measured the cost. Four of nine cases failed on added
  content, and `r4` scored 2 for padding a message that already read plainly. The rule came
  back, the length bullet gained "and no sentence gets padding", and step 1 of the pass now
  says a plain message returns nearly unchanged. The non-holdout mean went from 7.33 to
  8.22 over the same nine cases, and `r4` went from 2 to 9. The case list predates the
  mutation, so no expect paraphrases it.

- 2026-08-11, the plain-words rules went to the mouthpiece register. That run measured two
  false positives in `bare_acronym`, and this pass narrows both. `GATE` joined
  `NOT_ACRONYMS`. The engineer skill names its gates GATE A through GATE E, and three
  messages quote one as a fact. The rule also demanded the acronym first. It took
  `MCP (Model Context Protocol)` and rejected `Model Context Protocol (MCP)`. Five more
  messages paid for that, and both orders count as expanded now. Neither narrowing rests on
  a guess. All 8 failures over 22 real messages were one of these two, and none was a real
  miss.
- 2026-08-11, the rule finally means "at first use". A second run of 22 messages found the
  third false positive, and it was the worst one. The name carries the words "at first use",
  and the code never did. It flagged each bare use, so a message that expands a term once and
  then uses the short form failed on the second use. Step 3 above asks for exactly that
  shape, and so does the repo `CLAUDE.md`. Two messages failed this way, on `STE` and on
  `GEPA`. The rule reads the whole text now. It flags a term that no expansion covers
  anywhere, in either order.
- 2026-08-11, the fourth false positive: a hyphenated identifier. The rule flagged the
  leading run of `CPU-0003`, of `ABCD-1204`, and of `ASD-STE100`. No in-prose form could
  clear it, because the parenthesis would have to follow the digits, so even
  `CPU-0003 (the migration branch)` failed. Two mouthpiece cases hand the writer
  `map/CPU-0003` in the input, so the harness would have manufactured that failure. A blind
  judge found this one and measured it three ways. The rule now skips an uppercase run that
  a hyphen joins to a digit or to another uppercase run.

### open, measured, not yet fixed

- Added detail is still the top failure mode after the fix. Five cases name a form of it.
  They are "of records" in `r1`, a validation gloss in `r2`, and stage descriptions in
  `r3`. A causal overclaim in `r5` and a closing gloss in `r8` complete the list. The rule
  forbids the state, and it never
  tells the writer what to do where a plain word needs a subject. That gap is the next
  mutation.
- The holdout slice holds one measurement, 8.25 after the fix, with no case under 8. No
  pre-fix holdout run exists, so that number is a level and not a win.
- `JARGON` in `tools/ste-check/src/bro.rs` is a seed list of 24 entries. No logged miss
  widens it yet.

## logging

At the end of a use, append ONE JSON (JavaScript Object Notation) line to this skill's
`logs/usage.jsonl`:

```json
{"ts":"<local iso with offset>","artifact":"bro","trigger":"<what lost him>","excerpt":"<the flags ste-check raised + which terms got replaced>","outcome":"success|failure|partial","notes":"<corrections, surprises>"}
```

`ts` comes from `date +%Y-%m-%dT%H:%M:%S%z` in local time with its offset, and never in
UTC (Coordinated Universal Time). Cap the line at ~2KB.
