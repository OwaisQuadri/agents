pub const MASK: char = '\u{0}';

fn is_boundary(c: char) -> bool {
    !(c.is_alphanumeric() || c == '_')
}

fn is_word_boundary_at(bytes: &[u8], index: usize) -> bool {
    match bytes.get(index) {
        None => true,
        Some(&b) => is_boundary(b as char),
    }
}

/// Case-insensitive whole-word search, one entry per occurrence.
/// Word boundaries follow Python's `\b`, so a hyphen separates.
pub fn find_words(text: &str, needles: &[&str]) -> Vec<String> {
    let lower = text.to_lowercase();
    let bytes = lower.as_bytes();
    let mut hits = Vec::new();
    for needle in needles {
        let mut from = 0;
        while let Some(offset) = lower[from..].find(needle) {
            let start = from + offset;
            let end = start + needle.len();
            let is_start_free = start == 0 || is_word_boundary_at(bytes, start - 1);
            if is_start_free && is_word_boundary_at(bytes, end) {
                hits.push((*needle).to_string());
            }
            from = start + 1;
        }
    }
    hits
}

/// Case-insensitive substring search, one entry per phrase that appears at all.
pub fn find_phrases(text: &str, phrases: &[&str]) -> Vec<String> {
    let lower = text.to_lowercase();
    phrases
        .iter()
        .filter(|p| lower.contains(**p))
        .map(|p| (*p).to_string())
        .collect()
}

pub fn alpha_words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_ascii_alphabetic())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_string())
        .collect()
}

pub fn tokens(text: &str) -> Vec<&str> {
    text.split(|c: char| !(c.is_alphanumeric() || c == '\'' || c == '-'))
        .filter(|t| t.chars().any(|c| c.is_alphanumeric()))
        .collect()
}

/// YAML frontmatter is a data header, not prose, so no prose rule sees it.
pub fn strip_frontmatter(text: &str) -> &str {
    let Some(rest) = text.strip_prefix("---\n") else {
        return text;
    };
    match rest.find("\n---") {
        Some(end) => &rest[end + 4..],
        None => text,
    }
}

pub fn is_structural_line(line: &str) -> bool {
    matches!(line.trim_start().chars().next(), Some('#') | Some('|') | Some('>'))
}

pub fn is_list_line(line: &str) -> bool {
    strip_list_marker(line) != line.trim_start()
}

/// A `JOB:` / `IN:` / `OUT:` header line is a labelled field, so it stands alone rather
/// than running into the line under it.
pub fn is_label_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    let label: String = trimmed.chars().take_while(|c| c.is_ascii_uppercase()).collect();
    !label.is_empty() && trimmed[label.len()..].starts_with(':')
}

/// One prose unit: a paragraph, a single list item, a heading, or a table row.
/// Markdown structure ends a block, so a bullet list never reads as one long sentence.
pub fn blocks(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    let mut end = 0;
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let is_break = line.trim().is_empty()
            || is_list_line(line)
            || is_structural_line(line)
            || is_label_line(line);
        if is_break {
            if let Some(from) = start.take() {
                out.push(text[from..end].trim());
            }
        }
        if !line.trim().is_empty() {
            if start.is_none() {
                start = Some(offset);
            }
            end = offset + line.trim_end().len();
        }
        offset += line.len();
    }
    if let Some(from) = start {
        out.push(text[from..end].trim());
    }
    out.into_iter().filter(|b| !b.is_empty()).collect()
}

/// Token runs bounded by punctuation, so no rule reads across a comma or a clause break.
pub fn chunks(text: &str) -> Vec<&str> {
    text.split(|c: char| ",;:.!?()[]{}\"\n".contains(c))
        .filter(|c| !c.trim().is_empty())
        .collect()
}

/// A period closing a `1.` or `2)` list marker is punctuation for the list, not a
/// sentence end, so the splitter has to see the whole line to rule it out.
fn is_list_marker_period(text: &str, index: usize) -> bool {
    let head = &text[..index];
    let line_start = head.rfind('\n').map(|i| i + 1).unwrap_or(0);
    let marker = head[line_start..].trim_start();
    !marker.is_empty() && marker.chars().all(|c| c.is_ascii_digit())
}

pub fn sentences(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let bytes = text.as_bytes();
    for (i, c) in text.char_indices() {
        if c != '.' && c != '!' && c != '?' {
            continue;
        }
        let next_is_space = match bytes.get(i + 1) {
            None => true,
            Some(&b) => (b as char).is_whitespace(),
        };
        if !next_is_space || (c == '.' && is_list_marker_period(text, i)) {
            continue;
        }
        let piece = text[start..=i].trim();
        if !piece.is_empty() {
            out.push(piece);
        }
        start = i + 1;
    }
    let tail = text[start..].trim();
    if !tail.is_empty() {
        out.push(tail);
    }
    out
}

pub fn strip_list_marker(line: &str) -> &str {
    let trimmed = line.trim_start();
    if let Some(rest) = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
        .or_else(|| trimmed.strip_prefix("+ "))
    {
        return rest.trim_start();
    }
    let digits: String = trimmed.chars().take_while(|c| c.is_ascii_digit()).collect();
    if !digits.is_empty() {
        let rest = &trimmed[digits.len()..];
        if let Some(rest) = rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") ")) {
            return rest.trim_start();
        }
    }
    trimmed
}

pub fn is_emoji(c: char) -> bool {
    let code = c as u32;
    (0x1F000..=0x1FAFF).contains(&code) || (0x2600..=0x27BF).contains(&code) || code == 0xFE0F
}

/// Em dash, en dash, and a spaced hyphen between words. A leading "- " bullet is
/// stripped first, so a bullet list never reads as a dash between clauses.
pub fn dashes(text: &str) -> Vec<String> {
    let mut hits: Vec<String> = text
        .chars()
        .filter(|c| *c == '\u{2014}' || *c == '\u{2013}')
        .map(|c| c.to_string())
        .collect();
    for line in text.lines() {
        let body = strip_list_marker(line);
        let chars: Vec<char> = body.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '-' {
                let run = if i + 1 < chars.len() && chars[i + 1] == '-' { 2 } else { 1 };
                let before = i > 0 && chars[i - 1] == ' ';
                let after = chars.get(i + run).map(|c| *c == ' ').unwrap_or(false);
                if before && after {
                    hits.push("-".repeat(run));
                }
                i += run;
                continue;
            }
            i += 1;
        }
    }
    hits
}
