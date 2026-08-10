use crate::ste::Rule;
use crate::text::{alpha_words, dashes, find_phrases, find_words, sentences, MASK};

const ADVERB_PER_100: f64 = 2.0;
const RHYTHM_MIN_STDEV: f64 = 3.0;
const RHYTHM_MIN_SENTENCES: usize = 4;

const OPENERS: &[&str] = &[
    "in today's fast-paced", "it's worth noting", "it is worth noting", "at its core",
    "in this section", "let's dive in", "when it comes to", "it is important to understand",
    "first and foremost", "in order to better", "needless to say",
];

const HEDGE_WORDS: &[&str] = &[
    "arguably", "somewhat", "fairly", "quite", "rather", "perhaps", "essentially",
    "basically", "virtually", "relatively",
];

const HEDGE_PHRASES: &[&str] = &["generally speaking", "in some sense"];

const VAGUE: &[&str] = &[
    "game-changer", "powerful", "robust", "seamless", "leverage", "delve", "landscape",
    "realm", "tapestry", "underscore", "pivotal", "crucial", "cutting-edge", "best-in-class",
    "world-class", "revolutionize", "unlock", "elevate", "streamline",
];

const META: &[&str] = &[
    "as mentioned above", "as we will see", "this document will", "in this article",
    "the following section", "it should be noted", "for the purposes of this",
];

const WH_OPENERS: &[&str] = &[
    "what makes this", "why this matters", "how this works is", "what's interesting here",
];

const LY_EXCEPTIONS: &[&str] = &[
    "only", "early", "family", "reply", "apply", "supply", "likely", "daily", "weekly",
    "monthly", "yearly", "ugly", "italy", "assembly", "anomaly",
];

fn openers(text: &str) -> Vec<String> {
    find_phrases(text, OPENERS)
}

fn hedges(text: &str) -> Vec<String> {
    let mut hits = find_words(text, HEDGE_WORDS);
    hits.extend(find_phrases(text, HEDGE_PHRASES));
    hits
}

fn vague(text: &str) -> Vec<String> {
    find_words(text, VAGUE)
}

fn meta(text: &str) -> Vec<String> {
    find_phrases(text, META)
}

fn wh_openers(text: &str) -> Vec<String> {
    find_phrases(text, WH_OPENERS)
}

fn binary_contrast(text: &str) -> Vec<String> {
    let mut hits = crate::mouthpiece::not_just_but(text);
    let lower = text.to_lowercase();
    let mut from = 0;
    while let Some(offset) = lower[from..].find("not about") {
        let start = from + offset;
        let head = lower[..start].trim_end();
        let is_lead = head.ends_with("it's") || head.ends_with("its");
        let window: String = lower[start..].chars().take(80).collect();
        if is_lead && (window.contains("it's about") || window.contains("its about")) {
            hits.push(window.chars().take(60).collect());
        }
        from = start + "not about".len();
    }
    hits
}

fn adverbs(text: &str) -> Vec<String> {
    let words = alpha_words(text);
    if words.is_empty() {
        return Vec::new();
    }
    let ly: Vec<&String> = words
        .iter()
        .filter(|w| w.ends_with("ly") && w.len() > 4 && !LY_EXCEPTIONS.contains(&w.as_str()))
        .collect();
    let density = ly.len() as f64 * 100.0 / words.len() as f64;
    if density > ADVERB_PER_100 {
        let sample: Vec<String> = ly.iter().take(5).map(|w| (*w).clone()).collect();
        return vec![format!(
            "{density:.1} per 100 words (cap {ADVERB_PER_100:.0}): {}",
            sample.join(", ")
        )];
    }
    Vec::new()
}

fn rhythm(text: &str) -> Vec<String> {
    let plain: String = text.chars().map(|c| if c == MASK { ' ' } else { c }).collect();
    let lengths: Vec<usize> = sentences(&plain)
        .iter()
        .map(|s| s.split_whitespace().count())
        .filter(|n| *n > 1)
        .collect();
    if lengths.len() < RHYTHM_MIN_SENTENCES {
        return Vec::new();
    }
    let mean = lengths.iter().sum::<usize>() as f64 / lengths.len() as f64;
    let variance = lengths
        .iter()
        .map(|n| (*n as f64 - mean).powi(2))
        .sum::<f64>()
        / lengths.len() as f64;
    let stdev = variance.sqrt();
    if stdev < RHYTHM_MIN_STDEV {
        let head: Vec<usize> = lengths.iter().copied().take(6).collect();
        return vec![format!(
            "sentence lengths {head:?}, stdev {stdev:.1} (min {RHYTHM_MIN_STDEV:.0})"
        )];
    }
    Vec::new()
}

pub const RULES: &[Rule] = &[
    ("no throat-clearing openers", openers),
    ("no hedges", hedges),
    ("no vague declaratives", vague),
    ("no meta-commentary", meta),
    ("no Wh- openers", wh_openers),
    ("no binary contrast shape", binary_contrast),
    ("no dash between clauses", dashes),
    ("adverb density within cap", adverbs),
    ("sentence rhythm varied", rhythm),
];
