---
name: byline
description: Use when prose is about to ship under the user's name where a stranger reads it: a commit message, PR(Pull Request) body, ticket short or long, README, changelog, release note, or doc. Strips the AI tells (throat-clearing openers, hedges, adverb padding, metronomic rhythm, vague declaratives, meta-commentary) and leaves the facts untouched. Skip for the message the user himself reads (/mouthpiece owns that register), for instructions an agent executes (docs/prompt-style.md), and for code comments (docs/comment-style.md).
metadata:
  short-description: De-slop the prose that ships under his name
---

# byline

JOB: return one piece of shipped prose with the AI tells cut and every fact unchanged
IN:  the draft, plus what it is (commit message, PR body, ticket short or long, README, changelog, doc)
OUT: the edited prose, plus the `evals/check.py` result on it

## register

Normal written English. Sentence case, full punctuation, apostrophes in contractions,
paragraphs where paragraphs help.

This is NOT /mouthpiece. That register is the user's own texting voice, for the one message
he reads. It never ships to a stranger, and a commit message written in it reads as broken
English to everyone else. Reach for /mouthpiece only when the reader is him.

## the pass

1. run `python3 evals/check.py draft.txt`. It flags what a script sees: banned openers,
   hedge density, adverb density, dashes, Wh- openers, vague declaratives, meta-commentary,
   and metronomic sentence rhythm. For the full inventory behind each flag and its
   replacement, read `references/phrases.md`.
2. fix every flag by CUTTING. A hedge comes out and the claim stands bare. An adverb comes
   out and the verb carries it. A throat-clearing opener comes out and the sentence under it
   becomes the first sentence.
3. then the three a script cannot see:
   - does a sentence claim something the source material does not support? Cut the claim,
     never soften it into a hedge.
   - does a paragraph say the same thing twice in different words? Keep the concrete one.
   - would the first sentence make a stranger read the second? If not, lead with the result.
4. re-run check.py. Every rule passes, or the draft does not ship.

## hard rules

- Facts are never edited. Every number, path, identifier, command, and version survives
  verbatim. De-slopping is deletion and re-ordering, and it is never rewriting the claim.
- Cut, never pad. An edit that makes the draft longer states why in one line.
- Keep the author's argument. Where the draft is wrong, say so separately, and never fix it
  silently inside a style pass.
- One voice per document. Do not de-slop half of a README and leave the rest.

## where it runs

- phase 23 prose audit, beside the comment audit, over the commit trail and the PR body
- phase 22, over each filed ticket's `short` and `long`
- any README, changelog, or doc edit, before it lands

## evals

`evals/run.sh` writes a candidate edit per case in `evals/cases.jsonl`, runs check.py on it,
then grades the content against the case expect with `evals/rubric.md`. `--holdout` runs the
held-out slice. A mechanical failure caps that case at 4, and an altered fact scores 0. GEPA
(Genetic-Pareto prompt evolution) runs read the mean and use the failure modes as feedback.

## logging

At the end of a use, append ONE JSON(JavaScript Object Notation) line to this skill's
`logs/usage.jsonl`:

```json
{"ts":"<local iso with offset>","artifact":"byline","trigger":"<what was edited>","excerpt":"<the flags check.py raised + what was cut>","outcome":"success|failure|partial","notes":"<corrections, surprises>"}
```

`ts` comes from `date +%Y-%m-%dT%H:%M:%S%z` in local time with its offset, never
UTC(Coordinated Universal Time). Cap the line at ~2KB.
