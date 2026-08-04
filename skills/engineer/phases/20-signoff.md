# phase 20 — signoff

JOB: 6-8 manual-test bullets a human can run, each under 75 characters, happy path first
IN:  test-cases.md (happy cases first), ux.md, panel.md; phase 19 clean
OUT: `.map/<ID>/signoff.md`; HUMAN GATE C

## steps

1. build the candidate pool: one bullet per happy-path case touching a changed user-visible flow, then the highest-risk edge and security observations from panel.md. Done when the pool is ranked.
2. write each bullet: imperative verb first, one action, one observable outcome — "Pause mid-run, relaunch — timer resumes at same elapsed". Done when every bullet names what the human will SEE.
3. enforce the frame: 6-8 bullets (under 6 → split compound happy-path bullets; over 8 → drop lowest-risk edge bullets first — happy-path bullets are never dropped); every line under 75 characters: `grep '^- ' signoff.md | sed 's/^- //' | awk 'length($0)>74{exit 1}'`. Done when both gates pass.
4. HUMAN GATE C: present the checklist and STOP — the human runs it and returns a verdict. A reported failure routes through phase 15. Commit `map(<ID>): phase 20 signoff`. Done when the verdict is in.

## blame tags

`signoff-bullet-wrong` `happy-path-omitted`
