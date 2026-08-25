# phase 07 — interfaces

JOB: every planned capability reachable through a written signature, with its contract documented along the call path — and no pre-existing body edited
IN:  data-structures.md; phase 06 committed
OUT: signatures in-repo; `.map/<ID>/interfaces.md`; `.map/<ID>/call-stacks.md`; `state.json.gates.S`

paths: interface, protocol, header, and service files. Name the concrete glob in interfaces.md.

## steps

1. DO NOT BUILD. DO NOT TEST. The prime rule: change a signature, DO NOT EDIT THE BODY. Existing bodies stay untouched, even when the new signature breaks them.
2. write every changed or added signature in code. New functions get throwing or `unimplemented` stub bodies. Done when every capability in the plan is reachable through a written signature.
3. record each signature's contract in interfaces.md: the preconditions, the postconditions, and the error behavior. The invariants in phase 17 attach to these contracts, so an undocumented contract is unattachable later. Done when every signature in the diff has a contract entry.
4. trace each capability into call-stacks.md, entry point down to leaf. Give every level these four facts.
   - the input types the level takes.
   - the output type it gives back.
   - every error it can raise, throw, or return.
   - every side-effect: a file write, a network call, a mutation of shared state, a spawned process, a change on screen.

   Plan the call path, and do not write the body. The user reads this file at GATE S, one phase from now, and he judges the design before phase 08 decomposes it and phase 13 builds it. Write `none` where a level has no errors or no side-effects. An omitted fact is a gap, not an empty set. Done when every signature from step 2 sits at some level of some stack.
5. verify the scope: the glob plus `.map/` only, and no body edits of pre-existing functions.
6. HUMAN GATE S. Present the structure through /show-me. Ask for the smallest fitting view, prefer a console-safe one, and do not select the format. Three parts. (a) every phase-06 type as before→after, each externally-owned shape beside the executed probe output that proves it. (b) every new or changed signature as one line, with the type that owns it. (c) each call stack as its levels, so a signature at the wrong level is visible as a line in the wrong place. Declarations only, never bodies. Ask two questions and no more: is any shape wrong, missing a state, or owned by the wrong type; does any signature sit at the wrong level. On a walk-back visit, apply STANDING APPROVAL from SKILL.md: diff the material against `gates.S.snapshot`, and where no shape, signature or owner moved, record that the standing approval carries and walk on without stopping. Where something moved, present the DELTA against the approved snapshot. Wait for the verdict; `state.json.gates.S` records it in his words, with the new snapshot, replacing the previous entry. Commit `map(<ID>): phase 07 interfaces`. Done when the user has seen all three parts and the entry exists.

## blame tags

`wrong-signature` `caller-mismatch` `missing-error-path` `contract-undocumented` `call-stack-gap` `missing-side-effect` `gate-s-unpresented`
