# phase 07 — interfaces

JOB: every planned capability reachable through a written signature, with its contract documented — and no pre-existing body edited
IN:  data-structures.md; phase 06 committed
OUT: signatures in-repo; `.map/<ID>/interfaces.md`

paths: interface, protocol, header, and service files. Name the concrete glob in interfaces.md.

## steps

1. DO NOT BUILD. DO NOT TEST. The prime rule: change a signature, DO NOT EDIT THE BODY. Existing bodies stay untouched, even when the new signature breaks them.
2. write every changed or added signature in code. New functions get throwing or `unimplemented` stub bodies. Done when every capability in the plan is reachable through a written signature.
3. record each signature's contract in interfaces.md: the preconditions, the postconditions, and the error behavior. The invariants in phase 17 attach to these contracts, so an undocumented contract is unattachable later. Done when every signature in the diff has a contract entry.
4. verify the scope: the glob plus `.map/` only, and no body edits of pre-existing functions. Commit `map(<ID>): phase 07 interfaces`.

## blame tags

`wrong-signature` `caller-mismatch` `missing-error-path` `contract-undocumented`
