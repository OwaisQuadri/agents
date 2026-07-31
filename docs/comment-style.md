# comment style

Comments are a last resort for making code understandable to maintainers. The code
failing to explain itself is the bug: rename, extract, and restructure until it reads
naturally. Fewer comments beat more; zero is the default target.

## the whitelist

A comment ships only if it is one of these shapes. Anything else: delete it and fix the
code instead.

1. inexpressible concept or architecture — a design decision, invariant, or
   cross-component contract that cannot be made implicit in the code itself
2. standard-violation exception — the code deliberately breaks a standing convention;
   the comment marks the exception and why it is allowed here
3. TODO — explicit and deliberate only, never reflexive; states the follow-up and why it
   is deferred
4. advanced math / physics / formula — the derivation, units, or source equation behind
   non-obvious arithmetic

## whitelisting a new shape

The list is closed. A shape not on it does not ship — it applies first: add the proposed
entry here (shape, one-sentence justification, one example) and get owais's approval in
the same change. Only then does it get used.
