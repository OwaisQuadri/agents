# comment style

Comments are a last resort for making code understandable to maintainers. The code
failing to explain itself is the bug: rename, extract, and restructure until it reads
naturally. Fewer comments beat more; zero is the default target.

## the whitelist

A comment ships only if it is one of these shapes. Anything else: delete it and fix the
code instead.

- inexpressible concept or architecture — a design decision, invariant, or
  cross-component contract that cannot be made implicit in the code itself
- standard-violation exception — the code deliberately breaks a standing convention;
  the comment marks the exception and why it is allowed here
- TODO — explicit and deliberate only, never reflexive; states the follow-up and why it
  is deferred
- advanced math / physics / formula — the derivation, units, or source equation behind
  non-obvious arithmetic

## whitelisting a new shape

The list is closed, and adding to it is a HUMAN GATE. A shape not on it does not ship —
it applies first: add the proposed entry here (shape, one-sentence justification, one
example), then stop and wait for the user's explicit approval. No candidate shape is
used before the gate clears.
