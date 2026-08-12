use crate::bro::{bare_acronym, jargon};
use crate::ste::Rule;
use crate::text::{dashes, find_words, is_emoji, MASK};

/// 600, not 500. STE keeps the articles and spells every word out, so the same status
/// runs longer than the texting register it replaced. Two eval cases came back as clean
/// STE at 511 and 586 characters.
const CHAR_CAP: usize = 600;
const LIST_CAP: usize = 5;

const JOINERS: &[&str] = &["however", "moreover", "furthermore", "in conclusion"];
const PRAISE: &[&str] = &["awesome", "excellent", "absolutely", "amazing", "perfect", "great"];
const GENUINE: &[&str] = &["genuine", "genuinely"];

fn r_dashes(text: &str) -> Vec<String> {
    dashes(text)
}

fn r_joiners(text: &str) -> Vec<String> {
    find_words(text, JOINERS)
}

fn r_praise(text: &str) -> Vec<String> {
    find_words(text, PRAISE)
}

fn r_genuine(text: &str) -> Vec<String> {
    find_words(text, GENUINE)
}

pub fn not_just_but(text: &str) -> Vec<String> {
    let lower = text.to_lowercase();
    let mut hits = Vec::new();
    let mut from = 0;
    while let Some(offset) = lower[from..].find("not just") {
        let start = from + offset;
        let after = start + "not just".len();
        let window: String = lower[after..].chars().take(60).collect();
        let clipped = window
            .split(['.', '\n'])
            .next()
            .unwrap_or("")
            .to_string();
        if let Some(comma) = clipped.find(',') {
            let rest = clipped[comma + 1..].trim_start();
            if rest.starts_with("but")
                && rest[3..].chars().next().map(|c| !c.is_alphanumeric()).unwrap_or(true)
            {
                hits.push(format!("not just{clipped}"));
            }
        }
        from = after;
    }
    hits
}

pub fn plain_text(text: &str) -> Vec<String> {
    let mut hits = Vec::new();
    for line in text.lines() {
        if line.trim_start().starts_with('#') {
            let marker = line.trim_start();
            let hashes = marker.chars().take_while(|c| *c == '#').count();
            if marker[hashes..].starts_with(' ') {
                hits.push(marker.chars().take(20).collect());
            }
        }
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '*' && chars.get(i + 1) == Some(&'*') {
                hits.push("**".to_string());
                i += 2;
                continue;
            }
            if chars[i] == '_' && chars.get(i + 1) == Some(&'_') {
                hits.push("__".to_string());
                i += 2;
                continue;
            }
            if chars[i] == '*' {
                let mut j = i + 1;
                while j < chars.len() && chars[j] != '*' {
                    j += 1;
                }
                if j < chars.len() && j > i + 1 && chars.get(j + 1) != Some(&'*') {
                    hits.push(chars[i..=j].iter().collect());
                    i = j + 1;
                    continue;
                }
            }
            i += 1;
        }
    }
    hits.extend(text.chars().filter(|c| is_emoji(*c)).map(|c| c.to_string()));
    hits
}

pub fn timestamps(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut hits = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != ':' {
            i += 1;
            continue;
        }
        let head = chars[..i].iter().rev().take_while(|c| c.is_ascii_digit()).count();
        let tail = chars[i + 1..].iter().take_while(|c| c.is_ascii_digit()).count();
        let is_free_before = i - head == 0 || !chars[i - head - 1].is_alphanumeric();
        let is_free_after = chars.get(i + 1 + tail).map(|c| !c.is_alphanumeric()).unwrap_or(true);
        if (1..=2).contains(&head) && tail == 2 && is_free_before && is_free_after {
            hits.push(chars[i - head..=i + 2].iter().collect());
        }
        i += 1;
    }
    hits
}

fn is_numbered_item(line: &str) -> bool {
    let trimmed = line.trim_start();
    let digits = trimmed.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits == 0 {
        return false;
    }
    let rest = &trimmed[digits..];
    rest.starts_with(". ") || rest.starts_with(") ")
}

pub fn list_cap(text: &str) -> Vec<String> {
    let mut run = 0;
    let mut worst = 0;
    for line in text.lines() {
        if is_numbered_item(line) {
            run += 1;
            worst = worst.max(run);
        } else if !line.trim().is_empty() {
            run = 0;
        }
    }
    if worst > LIST_CAP {
        vec![format!("{worst} items")]
    } else {
        Vec::new()
    }
}

pub fn char_cap(text: &str) -> Vec<String> {
    let count = text.chars().filter(|c| *c != MASK).count();
    if count > CHAR_CAP {
        vec![format!("{count} chars over a {CHAR_CAP} cap")]
    } else {
        Vec::new()
    }
}

pub const RULES: &[Rule] = &[
    ("no dash between clauses", r_dashes),
    ("no however/moreover/furthermore/in conclusion", r_joiners),
    ("no awesome/excellent/absolutely/amazing/perfect/great", r_praise),
    ("never genuine/genuinely", r_genuine),
    ("never 'not just x, but y'", not_just_but),
    ("plain text: no emoji/bold/italics/headings", plain_text),
    ("relative time, no timestamps", timestamps),
    ("numbered lists capped at 5", list_cap),
    ("body <= 600 chars excluding exact info", char_cap),
    // Borrowed from the bro register: the end-user message is graded on them as well.
    ("plain words, no term of art", jargon),
    ("every abbreviation expanded at first use", bare_acronym),
];
