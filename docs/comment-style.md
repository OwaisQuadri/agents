# comment style

Comments are a last resort for making code understandable to maintainers. The code
failing to explain itself is the bug: rename, extract, and restructure until it reads
naturally. Fewer comments beat more; zero is the default target.

## the whitelist

A comment ships only if it is one of these shapes. Anything else: delete it and fix the
code instead. `tools/comment-check` enforces the mechanical part: a PreToolUse hook on
`git commit` denies a staged source file whose non-doc comment block runs past 4 lines.

- inexpressible concept or architecture — a design decision, invariant, or
  cross-component contract that cannot be made implicit in the code itself
- standard-violation exception — the code deliberately breaks a standing convention;
  the comment marks the exception and why it is allowed here
- TODO — explicit and deliberate only, never reflexive; states the follow-up and why it
  is deferred
- advanced math / physics / formula — the derivation, units, or source equation behind
  non-obvious arithmetic
- docstring on a public API declaration — a documentation comment on a declaration that
  callers outside the module use. It states what the declaration does, its inputs, its
  output, and its errors. It never explains the body. A caller reads the signature and
  the docstring, and never the body, so those four facts have nowhere else to live at the
  call site. Approved by the owner on 2026-08-10. The format per language lives in
  docs/docstring-style.md, which covers every language that generates inline documentation.

## whitelisting a new shape

The list is closed, and adding to it is a HUMAN GATE. A shape not on it does not ship —
it applies first: add the proposed entry here (shape, one-sentence justification, one
example), then stop and wait for the user's explicit approval. No candidate shape is
used before the gate clears.

## cleared gates

- 2026-08-10 docstring on a public API declaration. The owner's note asked to "make sure
  docstrings are formatted like google and apple (research more)", which presumes a shape
  this list did not carry. He approved it the same day.
