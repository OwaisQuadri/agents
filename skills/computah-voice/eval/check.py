#!/usr/bin/env python3
"""computah-voice skill eval — scores one candidate spoken line against the voice rules.

usage: python3 check.py line.txt   (or pipe the line on stdin)

Unlike mouthpiece, this skill wants ordinary sentence case and punctuation, so
those rules check for their PRESENCE rather than their absence. Prints one
pass/FAIL line per rule and a final `score: passed/total (fraction)`; exits
nonzero on any failure. GEPA (genetic-pareto prompt optimization) runs read the
score line and use the FAIL lines as feedback. Stdlib only.
"""
import re
import sys

MAX_SENTENCES = 3

MISSING_APOSTROPHE = re.compile(
    r"\b(?:dont|cant|wont|didnt|isnt|arent|wasnt|werent|hasnt|havent|hadnt"
    r"|wouldnt|couldnt|shouldnt|im|ive|youre|youve|theyre|theyve|thats"
    r"|whats|lets|were(?=\s)|id\b)\b", re.I,
)

TERMINATORS = re.compile(r"[.!?]")

PRAISE = re.compile(
    r"\b(?:awesome|excellent|absolutely|amazing|perfect|great|genuine|genuinely)\b",
    re.I,
)


def r_has_terminal_punctuation(t):
    return [] if TERMINATORS.search(t) else ["no . ! or ? found anywhere"]


def r_apostrophes_present(t):
    return MISSING_APOSTROPHE.findall(t)


def r_no_markdown(t):
    bad = re.findall(r"\*\*|__|^#+\s|^\s*[-*+]\s|^\s*\d+[.)]\s", t, re.M)
    bad += re.findall(r"(?<!\*)\*[^*\n]+\*(?!\*)", t)
    bad += [c for c in t if 0x1F000 <= ord(c) <= 0x1FAFF
            or 0x2600 <= ord(c) <= 0x27BF or ord(c) == 0xFE0F]
    return bad


def r_no_stacked_lines(t):
    lines = [ln for ln in t.splitlines() if ln.strip()]
    return ["%d lines" % len(lines)] if len(lines) > 2 else []


def r_praise_words(t):
    return PRAISE.findall(t)


def r_sentence_cap(t):
    n = len(TERMINATORS.findall(t))
    return ["%d sentences over a %d guidance" % (n, MAX_SENTENCES)] if n > MAX_SENTENCES else []


RULES = [
    ("has normal terminal punctuation somewhere", r_has_terminal_punctuation),
    ("contractions keep their apostrophe", r_apostrophes_present),
    ("no markdown: no bold/headings/bullets/numbered lists/emoji", r_no_markdown),
    ("not stacked into separate lines, one flowing bit of speech", r_no_stacked_lines),
    ("no awesome/excellent/absolutely/amazing/perfect/great/genuine", r_praise_words),
    ("roughly <= %d sentences" % MAX_SENTENCES, r_sentence_cap),
]


def run(text):
    passed = 0
    for name, rule in RULES:
        bad = rule(text)
        if bad:
            shown = ", ".join(repr(str(b).strip()[:40]) for b in bad[:3])
            more = " (+%d more)" % (len(bad) - 3) if len(bad) > 3 else ""
            print("FAIL  %s: %s%s" % (name, shown, more))
        else:
            passed += 1
            print("pass  %s" % name)
    print("score: %d/%d (%.3f)" % (passed, len(RULES), passed / len(RULES)))
    return passed == len(RULES)


if __name__ == "__main__":
    if len(sys.argv) > 1:
        with open(sys.argv[1], encoding="utf-8") as f:
            text = f.read()
    else:
        text = sys.stdin.read()
    sys.exit(0 if run(text) else 1)
