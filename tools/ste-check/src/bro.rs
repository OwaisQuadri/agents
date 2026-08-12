//! The bro register: re-explain the last message so a reader cannot misread it.
//!
//! Every other register caps its length, and this one deliberately does not. Upstream rule
//! 2 is "simpler, not necessarily shorter", so an idea that needs room gets the room. The
//! missing cap is the rule here, and not an omission.
//!
//! It also keeps the hedges byline bans. "Basically" and "essentially" are the casual
//! connectives upstream rule 4 asks for, so bro reuses byline's opener and consultant-speak
//! lists and never its hedge list.

use crate::byline::{openers, vague};
use crate::mouthpiece::plain_text;
use crate::ste::Rule;
use crate::text::find_words;

const MIN_ACRONYM: usize = 2;
const MAX_ACRONYM: usize = 5;

/// Seeded narrow on purpose. A wide list flags prose that already reads clearly, so this
/// grows on a logged miss and never on a guess. The mouthpiece register grades on this
/// list as well, so an entry added here also fails every message the end user reads.
const JARGON: &[&str] = &[
    "abstraction", "abstractions", "canonical", "canonicalize", "deterministic", "determinism",
    "dispatch", "dispatched", "dispatches", "heuristic", "heuristics", "idempotency",
    "idempotent", "instantiate", "instantiated", "instantiates", "invariant", "invariants",
    "monotonic", "orthogonal", "serialize", "serialized", "serializes", "topology",
];

/// GATE earns its place from a measured false positive, not a guess: the engineer skill
/// names its human gates GATE A through GATE E, so a status message quotes "GATE E" as a
/// fact and the rule read it as an unexpanded abbreviation.
const NOT_ACRONYMS: &[&str] = &["OK", "TV", "AM", "PM", "GATE"];

pub fn jargon(text: &str) -> Vec<String> {
    find_words(text, JARGON)
}

fn is_free(c: Option<&char>) -> bool {
    match c {
        None => true,
        Some(c) => !(c.is_alphanumeric() || *c == '_'),
    }
}

/// A hyphenated identifier, not an abbreviation: `CPU-0003`, `ABCD-1204`, `ASD-STE100`. No
/// in-prose form can satisfy the expansion test here, because the parenthesis would have to
/// follow the digits, so the rule flagged a ticket id and a branch name with no way out.
fn is_hyphenated_id(chars: &[char], end: usize) -> bool {
    chars.get(end) == Some(&'-')
        && chars
            .get(end + 1)
            .is_some_and(|c| c.is_ascii_digit() || c.is_ascii_uppercase())
}

pub fn bare_acronym(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut expanded = Vec::new();
    let mut hits = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if !chars[i].is_ascii_uppercase() {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && chars[i].is_ascii_uppercase() {
            i += 1;
        }
        let run: String = chars[start..i].iter().collect();
        let mut end = i;
        if chars.get(end) == Some(&'s') {
            end += 1;
        }
        if !(MIN_ACRONYM..=MAX_ACRONYM).contains(&run.len())
            || NOT_ACRONYMS.contains(&run.as_str())
            || (start > 0 && !is_free(chars.get(start - 1)))
            || !is_free(chars.get(end))
            || is_hyphenated_id(&chars, end)
        {
            continue;
        }
        // Either order counts as expanded. "MCP (Model Context Protocol)" reads acronym
        // first, and "Model Context Protocol (MCP)" reads expansion first, which is the
        // form a writer reaches for when a human reads the sentence.
        let is_wrapped = start > 0 && chars[start - 1] == '(' && chars.get(end) == Some(&')');
        let expansion = if chars.get(end) == Some(&' ') { end + 1 } else { end };
        if is_wrapped || chars.get(expansion) == Some(&'(') {
            expanded.push(run);
        } else {
            hits.push(run);
        }
    }
    // The rule name says "at first use", so one expansion anywhere covers every later short
    // form. Every register tells the writer to expand once and then use the short form.
    hits.retain(|run| !expanded.contains(run));
    hits
}

fn flat_prose(text: &str) -> Vec<String> {
    let mut hits = plain_text(text);
    for line in text.lines() {
        if line.trim_start().starts_with('|') {
            hits.push(line.chars().take(20).collect());
        }
    }
    hits
}

pub const RULES: &[Rule] = &[
    ("plain words, no term of art", jargon),
    ("every abbreviation expanded at first use", bare_acronym),
    ("flat prose: no headings, tables, bold, or emoji", flat_prose),
    ("no throat-clearing openers", openers),
    ("no consultant-speak", vague),
];
