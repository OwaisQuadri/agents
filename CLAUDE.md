# CLAUDE.md: global guidance

## working style: startup, not enterprise

Default to the simplest thing that works and ships today. When in doubt, write less.

- build for the requirement in front of you; no speculative config, plugin systems, or extension points.
- Minimize abstraction: DO NOT use abstraction until you are literally unable to go without.

## abbreviations

Expand every abbreviation, shortform, acronym, or pseudonym at first use in each conversation so its easy to search for (Example first use: "RAG(Retrieval Augmented Generation)" ). Never
introduce one without its inline expansion. Never guess an unresolved one: when in doubt, repeat the above example format.

## agent communication

- quoted content passes through unaltered.
- be verbose between agents. always use the verbose version of functions
- each dispatch carries only context that its step needs.

## user-facing replies

End-user-facing text follows the /mouthpiece skill (voice rules live there, not here).

## code style

Before writing code, read ~/Documents/agents/docs/code-style.md: the user's manual style
overrides, one rule per bullet. Rules there beat default style judgment and language
convention.

## code comments

Before writing any code comment, read ~/Documents/agents/docs/comment-style.md. Comments
are a last resort and only its whitelisted shapes ship; a shape not on the list is
proposed there first, never written ad hoc.

## moves and deletes

Never rm before a verified move. Verifying the destination (file counts match) is a separate step that happens before any delete.

## time estimation

Estimate as if a dedicated team works full time on each task. Agents working with the user are much faster and more capable than the human-team timelines in training data. NEVER quote training-data-shaped timelines.
