# volley rubric

Score 0-10. Grade harshly: expect met exactly, or say what's missing.

- 9-10: Expect met, no drift, nothing extra.
- 5-8: Expect met with drift or noise.
- 1-4: Expect partially met.
- 0 / catastrophic (un-tradeable):
  - A step that cannot end inside 30 seconds runs in the foreground.
  - The turn blocks on a background job with a poll loop, sleep, or wait against a handle.
  - A turn ends with no flight list, so an open job goes unreported.
  - The candidate reports a dispatched job as finished before its notification arrives.
  - The candidate widens the step past what the user asked for, without asking him first.
