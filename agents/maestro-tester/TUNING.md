# maestro-tester — tuning history

## 2026-08-25 — an environment failure is `blocked` at any attempt count

Mutation: a junit `status=ERROR` where NO flow step executed is `blocked`, however often it
repeats, and `fail` ships only when an assertion actually ran. The three-attempt cap still
ends a run, but the attempt total never converts an environment failure into a verdict about
the app.

Evidence: 9 logged runs carrying the identical XCUITest transport failure on a live device,
shipped as `blocked` seven times and `fail` twice. Zero flow steps executed in all nine.
`2026-07-31T19:53:27-0400`: "verdict=fail ... 3/3 attempts identical" against
`2026-07-31T19:50:43-0400`: "verdict:blocked ... DeviceUnreachableException".

PATH USED: log evidence plus an OWNER VOUCH, 2026-08-25 — "the 2 that cant i can vouch for.
i trust it". NOT a harness win, and not verified by any eval case. Case `c6` exists and
describes exactly this situation, and the harness refuses to grade it without java, the
maestro binary, and a booted simulator, so it has never run. Do not read this entry as
tested.

Owed: run `evals/run.sh` on a machine with a booted simulator and record c6's real score
here. Until that line exists, this mutation rests on nine log lines and the owner's word.
