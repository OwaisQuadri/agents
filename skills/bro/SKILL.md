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
mouthpiece message is tight because it carries only the facts; the bro version of that same
message spends whatever words the explaining takes. Grade bro output with `--register bro`,
and never with the register it replaced.

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
- The shared register rules in `docs/prompt-style.md` bind here, and the verbatim-facts one
  reaches further in this register: every path, command, filename, number, URL (Uniform
  Resource Locator), name, and decision stays exactly as it was, because a rewrite is where
  a fact is most likely to move.
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
  message keeps its own structure rules, and the long rewrite stays the `/bro` turn alone.

## evals

`evals/run.sh` lists a candidate rewrite per case in `evals/cases.jsonl`, runs `ste-check`
on it, then grades the content against the case expect with `evals/rubric.md`. `--holdout`
runs the held-out slice. A mechanical failure caps that case at 4, and an altered fact
scores 0. GEPA (Genetic-Pareto prompt evolution) runs read the mean and use the failure
modes as feedback.

## logging

At the end of a use, append ONE JSON (JavaScript Object Notation) line to this skill's
`logs/usage.jsonl`:

```json
{"ts":"<local iso with offset>","artifact":"bro","trigger":"<what lost him>","excerpt":"<the flags ste-check raised + which terms got replaced>","prompt_version":"<short sha>","outcome":"success|failure|partial","notes":"<corrections, surprises>"}
```

- `prompt_version` is the short commit of the last change to the files this artifact
  loads: `git -C ~/Documents/agents log -1 --format=%h -- <artifact dir> docs/prompt-style.md tools/ste-check/src ':(exclude)**/evals/**' ':(exclude)**/TUNING.md'`. A
  Reflect pass drops lines written against a prompt that no longer exists.
`ts` comes from `date +%Y-%m-%dT%H:%M:%S%z` in local time with its offset, and never in
UTC (Coordinated Universal Time). Cap the line at ~2KB.
