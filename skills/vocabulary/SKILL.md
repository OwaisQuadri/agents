---
name: vocabulary
description: Use when reaching for the exact word for a design or UI concept ("what's the term for the space between two specific letters?"), when someone rambles about making an interface look or feel better without the word for it ("make it pop", "feels cramped", "something's off"), when choosing between confusable near-synonyms (badge vs tag, tooltip vs popover, kerning vs tracking, opacity vs visibility), or when reviewing copy, specs, or commits for vague language where an exact term exists. Covers typography, color, iconography, layout, interaction, motion, accessibility, information architecture, copywriting, tools, analysis, and components. Skip when the feel-words are about prose or code rather than an interface, or when the ask is already precise and only the change remains.
metadata:
  short-description: Precise design and UI terminology
---

# Design Vocabulary

A reverse dictionary for design and UI work: go from a fuzzy description to the precise,
agreed-upon word, and know the boundary between words that look interchangeable but aren't.
Terms from `index.how/to/articulate`.

## Files

- `vocabulary.md` — canonical definitions. Quote from here verbatim; the wording is deliberate.
- `terms-index.md` — flat list of every term name by category, for scanning when you don't
  yet know the word you want.
- `symptom-map.md` — ramble words ("pop", "cramped", "janky") mapped to candidate terms.
- `upstream-SKILL.md` — the original author's instructions, for reference.

British spellings throughout ("centre"). Match them.

## Lookup

1. Read the loose description. Narrow by category (letters → Typography, empty area →
   Layout, a small attached label → Components).
2. Scan `terms-index.md` for candidates. When the description is feel-words rather than
   a nameable concept ("cramped", "pop"), read `symptom-map.md` first.
3. Read the matching definition in `vocabulary.md`.
4. Answer with the term, its definition, AND the contrasting term the definition names.
   The value here is the boundary between words, not the word alone.

For "what does X mean" / "is this a badge or a tag", same thing in reverse: pull both
definitions and state the distinguishing rule plainly.

When reviewing copy, specs, or commits, swap vague phrasing for the exact term where one
exists ("the spacing between the letters" → tracking). Don't force jargon where plain
language is clearer.

## Reverse lookup from a ramble

For when nobody asked for a word: the ask is a vague wish about how an interface should
look or feel ("make it pop", "it feels cramped", "something's off about the header").
Translate the ramble into terms before touching anything.

1. Pull each distinct complaint or wish out of the ramble, one line each, in the
   speaker's own words. Done when every feel-word is on the list.
2. For each feel-word, read `symptom-map.md` and take its candidate terms. For a
   feel-word the map doesn't cover, ask what a designer would adjust to fix it, then
   scan `terms-index.md` across every plausible category. Done when every feel-word
   from step 1 has at least one candidate.
3. Read each candidate's definition in `vocabulary.md`. Keep the terms whose definition
   describes this symptom — at most three per complaint — and drop the rest.
4. Answer with the mapping: their phrase → the surviving term(s), each handled per the
   Guidelines below, plus what adjusting it would change. When two terms genuinely fit,
   give both with the distinguishing rule. Done when every complaint from step 1
   appears in the mapping.
5. If the ramble arrived mid-task ("make it classier" during a build), carry on with
   the task. Done when the change is described in the mapped terms, not the feel-words.

## When a term is indexed but NOT defined

`terms-index.md` lists ~188 terms; `vocabulary.md` defines ~160. About 28 are index-only,
so a lookup can come back empty. When grep finds the term in the index but no definition
in `vocabulary.md`:

1. Say plainly that this one isn't in the bundled definitions.
2. Web-search it scoped to design, e.g. `"<term>" UI design term` or `"<term>" UX
   definition`. Weight results toward Nielsen Norman Group, Apple HIG, Material Design,
   MDN, W3C/WCAG, and established design-system docs. A general-English hit for a word
   that also has a design sense (spring, depth, voice, tone, signpost) is the wrong
   answer — the design-context sense is the one being asked for.
3. Answer with the sourced definition and name where it came from, so it's visibly
   external rather than bundled.
4. If the search turns up nothing clearly in the design sense, say the term is undefined
   and describe the concept plainly. Never coin jargon.

Only search for terms `vocabulary.md` is missing. If it defines the term, quote it — a
search result never overrides a bundled definition.

## Near-synonyms worth naming proactively

- Kerning vs tracking — between two specific characters vs uniform across a run.
- Badge vs tag — attached and informational vs standalone, selectable, removable.
- Tooltip vs popover — can't hold interactive content vs can.
- Opacity vs visibility — opacity 0 still takes pointer events; visibility hidden doesn't.
- Ease-in vs ease-out — ease-out for entering, ease-in for leaving.
- Modal vs sheet vs drawer — interrupting overlay vs side-edge panel vs bottom pull-up.
- Voice vs tone — constant personality vs shifts with the moment.
- Chroma vs saturation — chroma is the OKLCH, perceptually-accurate equivalent.
- WCAG vs APCA — established contrast standard vs newer model accounting for size and weight.
- Variables vs tokens — named values vs design decisions both tools and code reference.

## Guidelines

- Quote `vocabulary.md`, don't paraphrase it.
- Always surface the contrasting term when the definition names one.
- Never invent a definition. Search (above) or say so.

## evals

`evals/run.sh` grades every non-holdout case in `evals/cases.jsonl` against this file
plus `symptom-map.md` and `terms-index.md`, using `evals/rubric.md` via `claude -p`.
`./run.sh --holdout` runs the held-out slice.

## logging

At the end of a use, append ONE JSON(JavaScript Object Notation) line to this
skill's `logs/usage.jsonl`:

```json
{"ts":"<local iso with offset, e.g. 2026-07-31T14:05:09-0400>","artifact":"vocabulary","trigger":"<what fired it>","excerpt":"<relevant transcript excerpt>","outcome":"success|failure|partial","notes":"<corrections, surprises>"}
```

- `ts` is the machine's current local timezone with offset
  (`date +%Y-%m-%dT%H:%M:%S%z`), never UTC(Coordinated Universal Time): the user
  analyzes these against their own day.
- The excerpt is the relevant transcript parts only — the trigger, the key outputs,
  any human correction. Never the full transcript; cap ~2KB per line.
