---
name: byline
description: Use when prose is about to ship under the user's name where a stranger reads it. a commit message, PR(Pull Request) body, ticket short or long, README, changelog, release note, or doc. Strips the AI tells (throat-clearing openers, hedges, adverb padding, metronomic rhythm, vague declaratives, meta-commentary) and leaves the facts untouched. Skip for the message the user himself reads, which /mouthpiece owns, and for code comments, which docs/comment-style.md owns.
metadata:
  minimum-tier: T4
  short-description: De-slop the prose that ships under his name
---

# byline

JOB: return one piece of shipped prose with the AI tells cut and every fact unchanged
IN:  the draft, plus what it is (commit message, PR body, ticket, README, changelog, doc)
OUT: the edited prose, plus the `ste-check` result on it

## register

Simplified Technical English, per `docs/prompt-style.md`, with paragraphs where paragraphs
help. Every register in this repo runs on that base. What byline adds is the AI-tell pass
below, because this is the only prose a stranger reads.

`/mouthpiece` shares the base and differs in shape. That register bounds length by the
facts and ends on a next action. The reader there is the user himself, and he acts on
it. Reach for it only where he is the reader.

## the pass

1. Run `ste-check --register byline draft.txt`. It flags the STE rules first. It then flags
   banned openers, hedge density, adverb density, dashes, Wh- openers, vague declaratives,
   meta-commentary, and metronomic sentence rhythm. For the full inventory behind each flag
   and its replacement, read `references/phrases.md`.
2. Fix every flag by CUTTING. A hedge comes out and the claim stands bare. An adverb comes
   out and the verb carries it. A throat-clearing opener comes out, and the sentence under
   it becomes the first sentence.
3. Then handle the three a script cannot see.
   - Does a sentence claim something the source material does not support? Cut the claim,
     and never soften it into a hedge.
   - Does a paragraph say the same thing twice in different words? Keep the concrete one.
   - Would the first sentence make a stranger read the second? Where it would not, lead
     with the result.
4. Re-run `ste-check`. Every rule passes, or the draft does not ship.

## hard rules

- Never edit a fact. Every number, path, identifier, command, and version survives
  verbatim. De-slopping is deletion and re-ordering, and it never rewrites the claim.
- Cut, and never pad. An edit that makes the draft longer states why in one line.
- Keep the author's argument. Where the draft is wrong, say so separately, and never fix it
  silently inside a style pass.
- One voice per document. Do not de-slop half of a README and leave the rest.

## where it runs

- Phase 23 prose audit, beside the comment audit, over the commit trail and the PR body.
- Phase 22, over each filed ticket's `short` and `long`.
- Any README, changelog, or doc edit, before it lands.

## evals

`evals/run.sh` writes a candidate edit per case in `evals/cases.jsonl`, runs `ste-check` on
it, then grades the content against the case expect with `evals/rubric.md`. `--holdout`
runs the held-out slice. A mechanical failure caps that case at 4, and an altered fact
scores 0. GEPA (Genetic-Pareto prompt evolution) runs read the mean and use the failure
modes as feedback.
