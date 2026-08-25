# prose style: Simplified Technical English, every register

Every line here is a procedure or a report. ASD-STE100 (Simplified Technical English, the
aerospace maintenance-manual spec) is the closed-language answer to a procedure being
misread. The rules below are the part of that spec that ports. The rest is aerospace
vocabulary and does not apply here.

Applies to every register. That means SKILL.md bodies, phase files, agent definitions,
GRAPH SPEC blocks, and dispatch text. It also means the replies the user reads, spoken
replies, and prose that ships under his name. Each register skill adds its own medium
rules on top of these. Where a register rule and a rule here disagree, the rule here wins.
Code comments are the one exception, and comment-style.md owns them.

## the rules

- One instruction per sentence. Two things the agent must do become two sentences, even
  when the second one arrives as a parenthetical or as a clause after a dash.
- 20 words per instruction sentence, 25 per descriptive one. Over the cap, split it.
- Write every word. Never drop one to compress a line.
- Sentence case, and a closing period, question mark, or exclamation mark every time.
- Simple tenses only: simple present, simple past, simple future.
- One word, one meaning, across the whole config. Pick `dispatch` and never also write
  spawn, launch, fire, or hand off. A synonym reads as a second concept. Grep the config
  for the concept before you name it a new way.
- One part of speech per word. Where `gate` is a noun, write "the human gate applies", never "gate the change".
- **`message` means between the user and an agent.** Write `agent-to-agent message` for one
  agent's text to another, and never let `message` carry both. Owner's instruction, 2026-08-11,
  after a bare `message` in a requirement got read as the agent-to-agent kind and a whole design
  followed the wrong one. Vendor field names are exempt and stay as their API spells them.
- Active voice, actor named. Write "the orchestrator appends the failure line", never "failure lines are appended". An unnamed actor is an unassigned step.
- No noun stack over three words. "phase-04 data-only engine drive matrix" becomes "the drive matrix for the phase-04 engine".
- Keep the articles and the relative pronouns. Write "the angles that partition the failure space", not "angles partitioning the failure space".
- A parenthetical holds examples only. An instruction inside parentheses is one the agent
  is free to skip. Promote it to its own sentence.
- The simplest word that carries the meaning. Write "use", not "utilize". Write "before", not "prior to". Write "to", not "in order to". Write "stop", not "terminate". Write "start", not "initiate". Write "about", not "approximately". Write "next", not "subsequent".
- One topic per paragraph, and at most six sentences.
- Never write "genuine" or "genuinely", in any form. Owner's instruction, 2026-08-11. It is
  a hedge that adds no fact: strike it and the claim reads the same, or the claim was thin
  and needs evidence rather than an adverb.

## the shared register rules

These held in triplicate across mouthpiece, computah-voice and bro, near-verbatim in each.
They live here once, and every register inherits them. A register restates one of them only
where its own medium changes the rule.

- Facts are verbatim. Every path, number, file:line, command, error string, and verdict
  comes from the actual work. The words around them are yours, and the facts are not.
- Where sources disagree, say they disagree, and never pick one.
- Where something is missing, say that, and never fill the gap with something plausible.
- The output is the message and nothing else. No preamble, no "here is the summary", and no
  sign-off.
- Never use the words awesome, excellent, absolutely, amazing, perfect, or great.
- Brutally honest, never preachy, never moralizing, and no sycophancy. You earn warmth.
- Where the work still runs and nothing real has landed, give one short line, then stop.

## why there is no approved word list

ASD-STE100 closes its dictionary at roughly 900 words because its readers are non-native
mechanics and 1980s machine translation. A model has neither limit. The rules above port,
and the dictionary does not. A closed list would also ban the leading word that
skill-author's craft rules run on: refute, skeptic, relentless, and red are all unapproved
in STE.

## the cost

STE prose runs longer than the compressed line it replaces, so one-reading sentences get
bought with tokens. Spend them where a misread is expensive. Dispatch text, exit criteria,
and contracts qualify. So does the reply the user acts on, and so does anything a
fresh-context agent reads once.

## the checker

`ste-check` grades the mechanical part of these rules, plus whatever the register adds on
top. Run `ste-check --register agent FILE` over agent-facing prose. The other registers are
`mouthpiece`, `computah`, `byline`, and `bro`. Source and rule list live in
`tools/ste-check`.

Two rules above are guidance for a human, and the checker does not grade them. The
noun-stack rule shipped once and came back out. It flagged four good clauses for every one
it caught. A noun stack and a noun-verb-noun clause need part-of-speech data to tell apart,
and the tool has none. The one-word-one-meaning rule needs the whole config in view, so
grep that one by hand.

The relative-pronoun rule under-reports on purpose. It fires only where a determiner
follows the gerund. A rule that flags every message becomes noise, and noise gets ignored.
