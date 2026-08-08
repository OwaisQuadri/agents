#!/usr/bin/env python3
"""byline skill eval — scores one piece of shipped prose against the AI-tell rules.

usage: python3 check.py draft.txt   (or pipe the draft on stdin)

Exact information — fenced code, inline backtick spans, file paths, URLs — is masked out
first, so the rules only see the prose. Prints one pass/FAIL line per rule and a final
`score: passed/total (fraction)`; exits nonzero on any failure. The inventory behind each
rule lives in references/phrases.md. Stdlib only.
"""
import re
import statistics
import sys

ADVERB_PER_100 = 2.0
RHYTHM_MIN_STDEV = 3.0
RHYTHM_MIN_SENTENCES = 4
MASK = "\x00"

EXACT_SPANS = [
    re.compile(r"```.*?(?:```|\Z)", re.S),
    re.compile(r"`[^`\n]+`"),
    re.compile(r"https?://\S+"),
    re.compile(r"(?:~|\./|/)[\w.\-]+(?:/[\w.\-]+)*"),
    re.compile(r"^FLAG:.*$", re.M),
]

OPENERS = [
    "in today's fast-paced", "it's worth noting", "it is worth noting", "at its core",
    "in this section", "let's dive in", "when it comes to", "it is important to understand",
    "first and foremost", "in order to better", "needless to say",
]
HEDGES = ["arguably", "somewhat", "fairly", "quite", "rather", "perhaps", "essentially",
          "basically", "virtually", "relatively", "generally speaking", "in some sense"]
VAGUE = ["game-changer", "powerful", "robust", "seamless", "leverage", "delve", "landscape",
         "realm", "tapestry", "underscore", "pivotal", "crucial", "cutting-edge",
         "best-in-class", "world-class", "revolutionize", "unlock", "elevate", "streamline"]
META = ["as mentioned above", "as we will see", "this document will", "in this article",
        "the following section", "it should be noted", "for the purposes of this"]
WH_OPENERS = ["what makes this", "why this matters", "how this works is",
              "what's interesting here"]

# an -ly word that is not an adverb of this kind
LY_EXCEPTIONS = {"only", "early", "family", "reply", "apply", "supply", "likely", "daily",
                 "weekly", "monthly", "yearly", "ugly", "italy", "assembly", "anomaly"}


def mask_exact(text):
    chars = list(text)
    for pat in EXACT_SPANS:
        for m in pat.finditer(text):
            for i in range(*m.span()):
                if chars[i] != "\n":
                    chars[i] = MASK
    return "".join(chars)


def _phrases(t, phrases):
    low = t.lower()
    return [p for p in phrases if p in low]


def _words(t, words):
    return re.findall(r"\b(?:%s)\b" % "|".join(words), t, re.I)


def sentences(t):
    parts = re.split(r"(?<=[.!?])\s+", t.replace(MASK, " "))
    return [s for s in (p.strip() for p in parts) if len(s.split()) > 1]


def r_openers(t):
    return _phrases(t, OPENERS)


def r_hedges(t):
    return _words(t, [h for h in HEDGES if " " not in h]) + _phrases(t, [h for h in HEDGES if " " in h])


def r_vague(t):
    pattern = r"\b(?:%s)\b" % "|".join(v.replace("-", r"\-") for v in VAGUE)
    return re.findall(pattern, t)


def r_meta(t):
    return _phrases(t, META)


def r_wh_openers(t):
    return _phrases(t, WH_OPENERS)


def r_binary_contrast(t):
    hits = re.findall(r"\bnot just\b[^.\n]{0,60}?,\s*but\b", t, re.I)
    hits += re.findall(r"\bit'?s not about\b[^.\n]{0,60}?[.!]\s*it'?s about\b", t, re.I)
    return hits


def r_dashes(t):
    hits = re.findall(r"—|–", t)
    for ln in t.splitlines():
        hits += re.findall(r"[^\S\n]--?[^\S\n]", re.sub(r"^\s*[-*+]\s", "", ln))
    return hits


def r_adverbs(t):
    words = re.findall(r"\b[a-z]+\b", t.lower())
    if not words:
        return []
    ly = [w for w in words if w.endswith("ly") and w not in LY_EXCEPTIONS and len(w) > 4]
    density = len(ly) * 100.0 / len(words)
    if density > ADVERB_PER_100:
        return ["%.1f per 100 words (cap %.0f): %s" % (density, ADVERB_PER_100, ", ".join(ly[:5]))]
    return []


def r_rhythm(t):
    lens = [len(s.split()) for s in sentences(t)]
    if len(lens) < RHYTHM_MIN_SENTENCES:
        return []
    sd = statistics.pstdev(lens)
    if sd < RHYTHM_MIN_STDEV:
        return ["sentence lengths %s, stdev %.1f (min %.0f)" % (lens[:6], sd, RHYTHM_MIN_STDEV)]
    return []


RULES = [
    ("no throat-clearing openers", r_openers),
    ("no hedges", r_hedges),
    ("no vague declaratives", r_vague),
    ("no meta-commentary", r_meta),
    ("no Wh- openers", r_wh_openers),
    ("no binary contrast shape", r_binary_contrast),
    ("no dash between clauses", r_dashes),
    ("adverb density within cap", r_adverbs),
    ("sentence rhythm varied", r_rhythm),
]


def run(text):
    masked = mask_exact(text)
    passed = 0
    for name, rule in RULES:
        bad = rule(masked)
        if bad:
            shown = ", ".join(repr(str(b).strip()[:60]) for b in bad[:3])
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
