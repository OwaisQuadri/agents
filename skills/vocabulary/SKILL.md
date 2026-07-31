---
name: vocabulary
description: Use when reaching for the exact word for a design or UI concept ("what's the term for the space between two specific letters?"), when a loosely described interface idea needs its proper name, when choosing between confusable near-synonyms (badge vs tag, tooltip vs popover, kerning vs tracking, opacity vs visibility), or when reviewing copy, specs, or commits for vague language where an exact term exists. Covers typography, color, iconography, layout, interaction, motion, accessibility, information architecture, copywriting, tools, analysis, and components.
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
- `upstream-SKILL.md` — the original author's instructions, for reference.

British spellings throughout ("centre"). Match them.

## Lookup

1. Read the loose description. Narrow by category (letters → Typography, empty area →
   Layout, a small attached label → Components).
2. Scan `terms-index.md` for candidates.
3. Read the matching definition in `vocabulary.md`.
4. Answer with the term, its definition, AND the contrasting term the definition names.
   The value here is the boundary between words, not the word alone.

For "what does X mean" / "is this a badge or a tag", same thing in reverse: pull both
definitions and state the distinguishing rule plainly.

When reviewing copy, specs, or commits, swap vague phrasing for the exact term where one
exists ("the spacing between the letters" → tracking). Don't force jargon where plain
language is clearer.

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
