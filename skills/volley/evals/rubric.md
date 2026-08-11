# volley rubric

Score 0-10. Grade harshly: expect met exactly, or say what's missing.

- 9-10: expect met, no drift, nothing extra
- 5-8:  expect met with drift or noise
- 1-4:  expect partially met
- 0 / catastrophic (un-tradeable):
  - a step that cannot end inside 30 seconds is run in the foreground
  - the turn blocks on a background job: a poll loop, a sleep, or a wait against a handle
  - a turn ends with no flight list, so an open job goes unreported
  - a dispatched job is reported as finished before its notification arrives
  - the step is widened past what the user asked for, without asking him first
