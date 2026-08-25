# phase 20 — signoff

JOB: 6-8 manual-test bullets a human can run, each under 75 characters, happy path first
IN:  test-cases.md (happy cases first), ux.md, panel.md; phase 19 clean
OUT: `.map/<ID>/signoff.md`; HUMAN GATE C

## steps

1. build the candidate pool. Write one bullet per happy-path case that touches a changed user-visible flow, and one per `mode: manual` case from test-cases.md — those cases exist because no harness can run them, so the human list is their only executor. Then add the highest-risk edge and security observations from panel.md. Done when the pool is ranked.
2. write each bullet with an imperative verb first, one action, and one observable outcome: "Pause mid-run, relaunch — timer resumes at same elapsed". Done when every bullet names what the human will SEE.
3. enforce the frame of 6 to 8 bullets. Under 6, split the compound happy-path bullets. Over 8, drop the lowest-risk edge bullets first. Happy-path bullets are never dropped. Every line stays under 75 characters: `grep '^- ' signoff.md | sed 's/^- //' | awk 'length($0)>74{exit 1}'`. Done when both gates pass.
4. HUMAN GATE C. Apply STANDING APPROVAL from SKILL.md over the checklist, its ranked source cases, and `tested_sha`, the latest phase-19 commit from `git log -1 --format=%H --grep='^map(<ID>): phase 19 '`. Write that SHA into the gate presentation. A prior manual verdict carries only for the same phase-19-tested state; resuming phase 20 or adding its marker does not move that identity, while any walk that rebuilds and re-tests the implementation creates a new phase-19 commit and fires Gate C. Present the checklist and STOP only when the governing rule fires the gate. Invoke /show-me on the checklist. Ask for its smallest fitting view. Do not select its output format. Prefer a console-safe view. The human runs it and returns a verdict. A reported failure routes through phase 15. Commit `map(<ID>): phase 20 signoff`. Done when approval carries or the verdict is in and `state.json.gates.C` records the approval number, immutable snapshot, and the human's words.

## blame tags

`signoff-bullet-wrong` `happy-path-omitted`
