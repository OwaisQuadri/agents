# phase 20 — signoff

JOB: 6-8 manual-test bullets a human can run, each under 75 characters, happy path first
IN:  test-cases.md (happy cases first), ux.md, panel.md; phase 19 clean
OUT: `.map/<ID>/signoff.md`; HUMAN GATE C

## steps

1. build the candidate pool. Write one bullet per happy-path case that touches a changed user-visible flow. Then add the highest-risk edge and security observations from panel.md. Done when the pool is ranked.
2. write each bullet with an imperative verb first, one action, and one observable outcome: "Pause mid-run, relaunch — timer resumes at same elapsed". Done when every bullet names what the human will SEE.
3. enforce the frame of 6 to 8 bullets. Under 6, split the compound happy-path bullets. Over 8, drop the lowest-risk edge bullets first. Happy-path bullets are never dropped. Every line stays under 75 characters: `grep '^- ' signoff.md | sed 's/^- //' | awk 'length($0)>74{exit 1}'`. Done when both gates pass.
4. HUMAN GATE C. Present the checklist and STOP. Invoke /show-me on the checklist. Ask for its smallest fitting view. Do not select its output format. Prefer a console-safe view. The human runs it and returns a verdict. A reported failure routes through phase 15. Commit `map(<ID>): phase 20 signoff`. Done when the verdict is in and `state.json.gates.C` records it in the human's words.

## blame tags

`signoff-bullet-wrong` `happy-path-omitted`
