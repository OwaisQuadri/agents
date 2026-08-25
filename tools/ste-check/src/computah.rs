use crate::mouthpiece::plain_text;
use crate::ste::Rule;
use crate::text::{find_words, strip_list_marker};

const MAX_SENTENCES: usize = 3;
const MAX_LINES: usize = 2;

const PRAISE: &[&str] = &[
    "awesome",
    "excellent",
    "absolutely",
    "amazing",
    "perfect",
    "great",
];

fn no_markdown(text: &str) -> Vec<String> {
    let mut hits = plain_text(text);
    for line in text.lines() {
        if strip_list_marker(line) != line.trim_start() {
            hits.push(line.chars().take(20).collect());
        }
    }
    hits
}

fn stacked_lines(text: &str) -> Vec<String> {
    let count = text.lines().filter(|l| !l.trim().is_empty()).count();
    if count > MAX_LINES {
        vec![format!("{count} lines")]
    } else {
        Vec::new()
    }
}

fn praise_words(text: &str) -> Vec<String> {
    find_words(text, PRAISE)
}

fn sentence_cap(text: &str) -> Vec<String> {
    let count = text
        .chars()
        .filter(|c| matches!(c, '.' | '!' | '?'))
        .count();
    if count > MAX_SENTENCES {
        vec![format!("{count} sentences over a {MAX_SENTENCES} guidance")]
    } else {
        Vec::new()
    }
}

pub const RULES: &[Rule] = &[
    (
        "no markdown: no bold/headings/bullets/numbered lists/emoji",
        no_markdown,
    ),
    (
        "not stacked into separate lines, one flowing bit of speech",
        stacked_lines,
    ),
    (
        "no awesome/excellent/absolutely/amazing/perfect/great",
        praise_words,
    ),
    ("roughly <= 3 sentences", sentence_cap),
];
