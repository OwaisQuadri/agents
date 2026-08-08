# prompt style: Simplified Technical English for agent-facing prose

Every line an agent executes is a procedure, and ASD-STE100 (Simplified Technical
English, the aerospace maintenance-manual spec) is the closed-language answer to
procedures being misread. The rules below are the part of that spec that ports; the rest
is aerospace vocabulary and does not apply here.

Applies to: SKILL.md bodies, phase files, agent definitions, GRAPH SPEC blocks, dispatch
text. Not to user-facing replies (that is /mouthpiece) or code comments
(comment-style.md).

## the rules

- One instruction per sentence. Two things the agent must do become two sentences, even
  when the second one arrives as a parenthetical or as a clause after a dash.
- 20 words per instruction sentence, 25 per descriptive one. Over the cap, split.
- One word, one meaning, across the whole config. Pick `dispatch` and never also write
  spawn, launch, fire, or hand off. A synonym reads as a second concept. Grep the config
  for the concept before you name it a new way.
- One part of speech per word. Where `gate` is a noun, write "the human gate applies",
  never "gate the change".
- Active voice, actor named. Write "the orchestrator appends the failure line", never
  "failure lines are appended". An unnamed actor is an unassigned step.
- No noun stack over three words. "phase-04 data-only engine drive matrix" becomes "the
  drive matrix for the phase-04 engine".
- Keep the articles and the relative pronouns. Write "the angles that partition the
  failure space", not "angles partitioning the failure space".
- A parenthetical holds examples only. An instruction inside parentheses is an
  instruction the agent is free to skip; promote it to its own sentence.

## why there is no approved word list

ASD-STE100 closes its dictionary at roughly 900 words because its readers are non-native
mechanics and 1980s machine translation. A model has neither limit. The rules above port;
the dictionary does not. A closed list would also ban the leading word that skill-author's
craft rules run on: refute, skeptic, relentless, and red are all unapproved in STE.

## the cost

STE prose runs longer than the compressed line it replaces, so one-reading sentences get
bought with tokens. Spend them where a misread is expensive: dispatch text, exit
criteria, contracts, anything a fresh-context agent reads once. Leave compressed prose
alone where the agent reads only for orientation.
