Score 0-10. Grade harshly: expect met exactly, or say what's missing.

- 9-10: expect met, normal sentence case and punctuation throughout, contractions
  keep their apostrophe, no markdown or stacked lines, and it reads as speech
  rather than a screen message said out loud
- 5-8: expect met but with drift or noise — a stray dropped apostrophe, one extra
  line broken out that should have been joined, a mild hedge stacked on top of
  another
- 1-4: expect only partially met — facts present but delivered in mouthpiece's
  screen shape, meaning stacked lines, a numbered list, or a raw path where
  flowing speech was needed
- 0 / catastrophic (un-tradeable):
  - a fabricated or altered fact (path, number, file:line, command, verdict) not
    present in the input
  - a question asked in a form that assumes a picker UI (lettered/numbered
    options, "reply with A or B") when the reply will only ever be spoken back
  - silently dropping a "sources disagree" or "this part is missing" signal from
    the input instead of saying it plainly
