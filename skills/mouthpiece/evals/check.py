#!/usr/bin/env python3
"""mouthpiece skill eval — scores one candidate message against the voice rules.

usage: python3 check.py message.txt   (or pipe the message on stdin)

Exact information — fenced code, inline backtick spans, file paths, quoted
data — is masked out first, so the rules and the 500-char cap only see the
voice's own words, matching the skill's cap rule. Prints one pass/FAIL line per
rule and a final `score: passed/total (fraction)`; exits nonzero on any
failure. GEPA (Genetic-Pareto prompt evolution) runs read the score line
and use the FAIL lines as feedback. Stdlib only.

Adapted from local-harness/agents/mouthpiece/check.py, which graded transcript
invariants; this grades a single candidate message, which is what a skill eval
can see.
"""
import re
import sys

CAP = 500
MASK = "\x00"

EXACT_SPANS = [
    re.compile(r"```.*?(?:```|\Z)", re.S),          # fenced code / command output
    re.compile(r"`[^`\n]+`"),                       # inline code, paths, commands
    re.compile(r"(?:~|/)[\w.\-]+(?:/[\w.\-]+)+"),   # bare file paths
    re.compile(r'"[^"\n]+"'),                       # quoted data
    re.compile(r"“[^”\n]+”"),        # curly-quoted data
]

# transliterated duas keep their capitals even when they open a line
ALLOWED_CAPS = {"MashaAllah", "Barakallahu", "Feek", "Allahumma", "Barik",
                "Assalamu", "Alaikum"}


def mask_exact(text):
    """Overwrite exact-information spans with MASK. Newlines survive so
    line-based rules keep the line structure."""
    chars = list(text)
    for pat in EXACT_SPANS:
        for m in pat.finditer(text):
            for i in range(*m.span()):
                if chars[i] != "\n":
                    chars[i] = MASK
    return "".join(chars)


SENTENCE_START = re.compile(r"(?:^\s*|[.!?]\s+)([A-Z][a-z]+)", re.M)


def r_sentence_caps(t):
    return [m.group(1) for m in SENTENCE_START.finditer(t)
            if m.group(1) not in ALLOWED_CAPS]


def r_line_periods(t):
    return [ln.strip()[-30:] for ln in t.splitlines() if ln.rstrip().endswith(".")]


def r_dashes(t):
    # em/en dash anywhere; " - " between words (leading "- " bullets excluded)
    return re.findall(r"—|–|[^\S\n]--?[^\S\n]", t)


CONTR = re.compile(
    r"\b(?:\w+n['’]t|i['’](?:m|ve|d|ll)|\w+['’](?:re|ve|ll)"
    r"|(?:that|it|there|here|what|who|he|she|let)['’]s)\b", re.I)


def r_contractions(t):
    return CONTR.findall(t)


def _words(t, *ws):
    return re.findall(r"\b(?:%s)\b" % "|".join(ws), t, re.I)


def r_joiners(t):
    return _words(t, "however", "moreover", "furthermore", "in conclusion")


def r_shortforms(t):
    return _words(t, "because", "yeah", "yes", "okay")


def r_praise(t):
    return _words(t, "awesome", "excellent", "absolutely", "amazing",
                  "perfect", "great")


def r_genuine(t):
    return _words(t, "genuine", "genuinely")


def r_not_just_but(t):
    return re.findall(r"\bnot just\b[^.\n]{0,60}?,\s*but\b", t, re.I)


def r_plain_text(t):
    bad = re.findall(r"\*\*|__|^#+\s", t, re.M)                 # bold, headings
    bad += re.findall(r"(?<!\*)\*[^*\n]+\*(?!\*)", t)           # italics
    bad += [c for c in t if 0x1F000 <= ord(c) <= 0x1FAFF
            or 0x2600 <= ord(c) <= 0x27BF or ord(c) == 0xFE0F]  # emoji
    return bad


def r_timestamps(t):
    return re.findall(r"\b\d{1,2}:\d{2}\b", t)


def r_list_cap(t):
    run = worst = 0
    for ln in t.splitlines():
        if re.match(r"\s*\d+[.)]\s", ln):
            run += 1
            worst = max(worst, run)
        elif ln.strip():
            run = 0
    return ["%d items" % worst] if worst > 5 else []


def r_char_cap(t):
    n = len(t.replace(MASK, ""))
    return ["%d chars over a %d cap" % (n, CAP)] if n > CAP else []


RULES = [
    ("no capital opening a line or sentence", r_sentence_caps),
    ("no period at line end", r_line_periods),
    ("no dash between clauses", r_dashes),
    ("no apostrophe contractions", r_contractions),
    ("no however/moreover/furthermore/in conclusion", r_joiners),
    ("bc not because, yea not yeah/yes, ok not okay", r_shortforms),
    ("no awesome/excellent/absolutely/amazing/perfect/great", r_praise),
    ("never genuine/genuinely", r_genuine),
    ("never 'not just x, but y'", r_not_just_but),
    ("plain text: no emoji/bold/italics/headings", r_plain_text),
    ("relative time, no timestamps", r_timestamps),
    ("numbered lists capped at 5", r_list_cap),
    ("body <= %d chars excluding exact info" % CAP, r_char_cap),
]


def run(text):
    masked = mask_exact(text)
    passed = 0
    for name, rule in RULES:
        bad = rule(masked)
        if bad:
            shown = ", ".join(repr(b.strip()[:40]) for b in bad[:3])
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
