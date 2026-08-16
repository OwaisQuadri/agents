use crate::text::{
    blocks, chunks, find_phrases, find_words, is_list_line, is_structural_line, sentences,
    strip_list_marker, tokens, MASK,
};

const INSTRUCTION_WORD_CAP: usize = 20;
const DESCRIPTIVE_WORD_CAP: usize = 25;
const PARAGRAPH_SENTENCE_CAP: usize = 6;

const IMPERATIVE_VERBS: &[&str] = &[
    "add", "always", "append", "apply", "ask", "assert", "assume", "avoid", "bound", "build",
    "call", "cap", "check", "choose", "cite", "close", "commit", "confirm", "copy", "count",
    "cut", "decide", "delete", "dispatch", "do", "drop", "emit", "ensure", "escalate",
    "exclude", "expand", "extend", "filter", "find", "fix", "flag", "follow", "get", "give",
    "grade", "grep", "hold", "include", "judge", "keep", "land", "leave", "limit", "link",
    "list", "log", "make", "mark", "merge", "move", "name", "never", "note", "open", "parse",
    "pick", "poll", "prefer", "print", "promote", "pull", "push", "put", "quote", "rank",
    "read", "reduce", "reject", "remove", "rename", "replace", "report", "resolve", "retry",
    "return", "run", "sample", "say", "scan", "score", "send", "set", "ship", "show", "skip",
    "sort", "spend", "split", "start", "state", "stash", "stop", "surface", "take", "tell",
    "trace", "treat", "tune", "use", "verify", "wait", "watch", "write",
];

const BE_VERBS: &[&str] = &["am", "is", "are", "was", "were", "be", "been", "being"];

const IRREGULAR_PARTICIPLES: &[&str] = &[
    "born", "brought", "built", "caught", "chosen", "done", "drawn", "driven", "eaten",
    "fallen", "flown", "forgotten", "found", "frozen", "given", "grown", "held", "hidden",
    "kept", "known", "left", "lost", "made", "met", "paid", "ridden", "risen", "seen", "sent",
    "shown", "sold", "sought", "spoken", "spent", "stolen", "taken", "taught", "thought",
    "thrown", "told", "torn", "woken", "worn", "written",
];

/// Words that follow a be-verb as adjectives, where the sentence is not passive. "the
/// migration is done" reports a state, and it names no hidden actor.
const PARTICIPLE_ADJECTIVES: &[&str] = &[
    "aged", "annoyed", "blessed", "bored", "confused", "crooked", "done", "excited", "gone",
    "interested", "learned", "naked", "pleased", "ragged", "rugged", "sacred", "scared",
    "stuck", "surprised", "tired", "wicked", "worried",
];

/// A transliterated dua opens in lowercase by convention, so sentence case skips it.
const DUA_OPENERS: &[&str] = &[
    "alhamdulillah", "assalamu", "bismillah", "inshallah", "jazak", "salam", "wa",
];

const TEXTING_SHORTFORMS: &[&str] = &[
    "bc", "idk", "imo", "kinda", "lmk", "nah", "ppl", "prob", "prolly", "rn", "tbh", "tho",
    "u", "ur", "wanna", "ya", "yea",
];

const DROPPED_APOSTROPHES: &[&str] = &[
    "arent", "cant", "couldnt", "didnt", "dont", "hadnt", "hasnt", "havent", "im", "isnt",
    "ive", "lets", "shouldnt", "thats", "theyre", "theyve", "wasnt", "werent", "whats", "wont",
    "wouldnt", "youre", "youve",
];

const COMPLEX_WORDS: &[&str] = &[
    "aforementioned", "approximately", "commence", "endeavor", "endeavour", "facilitate",
    "initiate", "subsequent", "subsequently", "terminate", "utilisation", "utilise",
    "utilization", "utilize", "whilst",
];

const COMPLEX_PHRASES: &[&str] = &["prior to", "in order to"];

// Owner's instruction, 2026-08-11: "pls take out genuinely from ur vocab. any
// version of that word." It sat in two registers and belongs in the base, so it
// binds the agent register too.
const BANNED_WORDS: &[&str] = &["genuine", "genuinely", "genuineness"];

const FUNCTION_WORDS: &[&str] = &[
    "a", "after", "all", "already", "also", "always", "an", "and", "any", "as", "at", "before",
    "both", "but", "by", "can", "each", "either", "every", "for", "from", "if", "in", "into",
    "her", "his", "it", "its", "just", "may", "must", "my", "never", "no", "nor", "not", "of",
    "off", "often", "on", "once", "one", "only", "or", "our", "out", "over", "own", "per",
    "since", "so", "some", "still", "than", "that", "the", "their", "then", "there", "these",
    "this", "those", "to", "two", "under", "until", "up", "when", "where", "which", "while",
    "who", "whose", "will", "with", "within", "without", "your",
];

const DETERMINERS: &[&str] = &[
    "a", "an", "each", "every", "her", "his", "its", "that", "the", "their", "this",
];

/// Verbs that legitimately take a bare -ing complement, so no relative pronoun is missing.
const CATENATIVE_VERBS: &[&str] = &[
    "avoid", "begin", "consider", "continue", "enjoy", "finish", "keep", "prefer", "risk",
    "start", "stop", "suggest", "try", "worth",
];

fn is_in(word: &str, list: &[&str]) -> bool {
    list.contains(&word.to_lowercase().as_str())
}

fn is_imperative(sentence: &str) -> bool {
    let body = strip_list_marker(sentence);
    match tokens(body).first() {
        Some(first) => is_in(first, IMPERATIVE_VERBS),
        None => false,
    }
}

/// The -thing pronouns end in "ing" without being gerunds.
fn is_gerund(word: &str) -> bool {
    let lower = word.to_lowercase();
    lower.ends_with("ing")
        && lower.len() > 5
        && !matches!(
            lower.as_str(),
            "anything" | "something" | "nothing" | "everything" | "during" | "ceiling"
        )
}

fn prose_blocks(text: &str) -> Vec<&str> {
    blocks(text)
        .into_iter()
        .filter(|b| !is_structural_line(b))
        .collect()
}

fn is_participle(word: &str) -> bool {
    let lower = word.to_lowercase();
    if is_in(&lower, PARTICIPLE_ADJECTIVES) {
        return false;
    }
    is_in(&lower, IRREGULAR_PARTICIPLES) || (lower.ends_with("ed") && lower.len() > 4)
}

pub fn sentence_length(text: &str) -> Vec<String> {
    let mut hits = Vec::new();
    for block in prose_blocks(text) {
        for sentence in sentences(block) {
            let count = sentence.split_whitespace().count();
            let cap = if is_imperative(sentence) {
                INSTRUCTION_WORD_CAP
            } else {
                DESCRIPTIVE_WORD_CAP
            };
            if count > cap {
                let head: String = sentence.chars().take(40).collect();
                hits.push(format!("{head} ({count} words, cap {cap})"));
            }
        }
    }
    hits
}

pub fn active_voice(text: &str) -> Vec<String> {
    let mut hits = Vec::new();
    for chunk in chunks(text) {
        let words = tokens(chunk);
        for pair in words.windows(2) {
            if is_in(pair[0], BE_VERBS) && is_participle(pair[1]) {
                hits.push(format!("{} {}", pair[0], pair[1]));
            }
        }
    }
    hits
}

pub fn texting_shortforms(text: &str) -> Vec<String> {
    find_words(text, TEXTING_SHORTFORMS)
}

pub fn contraction_apostrophes(text: &str) -> Vec<String> {
    find_words(text, DROPPED_APOSTROPHES)
}

pub fn sentence_case(text: &str) -> Vec<String> {
    let mut hits = Vec::new();
    for block in prose_blocks(text) {
        for sentence in sentences(block) {
            let body = strip_list_marker(sentence);
            let is_uncheckable = matches!(
                body.chars().next(),
                None | Some(MASK) | Some('0'..='9') | Some('`')
            ) || tokens(body).first().map(|t| is_in(t, DUA_OPENERS)).unwrap_or(false);
            if is_uncheckable {
                continue;
            }
            if let Some(first) = body.chars().find(|c| c.is_alphabetic()) {
                if first.is_lowercase() {
                    hits.push(sentence.chars().take(40).collect());
                }
            }
        }
    }
    let last_line = text.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("");
    if !is_structural_line(last_line) {
        let tail = text.trim_end_matches(|c: char| c.is_whitespace() || c == MASK);
        match tail.chars().last() {
            Some('.') | Some('!') | Some('?') | None => {}
            Some(other) => hits.push(format!("text ends on {other:?}, not . ! or ?")),
        }
    }
    hits
}

pub fn dropped_relative_pronoun(text: &str) -> Vec<String> {
    let mut hits = Vec::new();
    for chunk in chunks(text) {
        let words = tokens(chunk);
        for triple in words.windows(3) {
            let head = triple[0].to_lowercase();
            let tail = triple[2].to_lowercase();
            let is_bare_head = !is_in(&head, FUNCTION_WORDS)
                && !is_in(&head, BE_VERBS)
                && !is_in(&head, CATENATIVE_VERBS);
            if is_bare_head && is_gerund(triple[1]) && is_in(&tail, DETERMINERS) {
                hits.push(format!("{} {} {}", triple[0], triple[1], triple[2]));
            }
        }
    }
    hits
}

/// A modal inside parentheses is the reliable marker of a buried instruction. An
/// imperative-looking first word is not: "(Pull Request)" expands an abbreviation.
pub fn imperative_in_parenthetical(text: &str) -> Vec<String> {
    let mut hits = Vec::new();
    for line in text.lines() {
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] != '(' {
                i += 1;
                continue;
            }
            let mut j = i + 1;
            while j < chars.len() && chars[j] != ')' {
                j += 1;
            }
            if j >= chars.len() {
                break;
            }
            let inner: String = chars[i + 1..j].iter().collect();
            if !find_words(&inner, &["must", "never", "always"]).is_empty() {
                hits.push(inner.chars().take(40).collect());
            }
            i = j + 1;
        }
    }
    hits
}

pub fn simple_word(text: &str) -> Vec<String> {
    let mut hits = find_words(text, COMPLEX_WORDS);
    hits.extend(find_phrases(text, COMPLEX_PHRASES));
    hits
}

pub fn banned_word(text: &str) -> Vec<String> {
    find_words(text, BANNED_WORDS)
}

pub fn paragraph_length(text: &str) -> Vec<String> {
    let mut hits = Vec::new();
    for block in prose_blocks(text).into_iter().filter(|b| !is_list_line(b)) {
        let count = sentences(block).len();
        if count > PARAGRAPH_SENTENCE_CAP {
            let head: String = block.chars().take(40).collect();
            hits.push(format!("{head} ({count} sentences, cap {PARAGRAPH_SENTENCE_CAP})"));
        }
    }
    hits
}

pub type Rule = (&'static str, fn(&str) -> Vec<String>);

pub const RULES: &[Rule] = &[
    ("one instruction per sentence, within the word cap", sentence_length),
    ("active voice, actor named", active_voice),
    ("no texting shortforms", texting_shortforms),
    ("contractions keep their apostrophe", contraction_apostrophes),
    ("sentence case with terminal punctuation", sentence_case),
    ("articles and relative pronouns kept", dropped_relative_pronoun),
    ("a parenthetical holds examples only", imperative_in_parenthetical),
    ("the simplest word that carries the meaning", simple_word),
    ("never genuine/genuinely", banned_word),
    ("paragraph within the sentence cap", paragraph_length),
];
