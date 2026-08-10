use crate::text::MASK;

/// Both profiles mask quoted spans, because a quotation is someone else's words and
/// the rules grade the writer's own.
#[derive(Clone, Copy, PartialEq)]
pub enum Profile {
    /// Fenced code, inline backticks, absolute paths, and quoted spans.
    Spoken,
    /// The Spoken set plus URLs, relative paths, and FLAG lines.
    Shipped,
}

fn is_path_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '.' || c == '-'
}

fn find_fenced(chars: &[char], spans: &mut Vec<(usize, usize)>) {
    let mut i = 0;
    while i + 2 < chars.len() {
        if chars[i] == '`' && chars[i + 1] == '`' && chars[i + 2] == '`' {
            let mut j = i + 3;
            while j + 2 < chars.len() && !(chars[j] == '`' && chars[j + 1] == '`' && chars[j + 2] == '`') {
                j += 1;
            }
            let end = if j + 2 < chars.len() { j + 3 } else { chars.len() };
            spans.push((i, end));
            i = end;
            continue;
        }
        i += 1;
    }
}

fn find_delimited(chars: &[char], open: char, close: char, spans: &mut Vec<(usize, usize)>) {
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == open {
            let mut j = i + 1;
            while j < chars.len() && chars[j] != close && chars[j] != '\n' {
                j += 1;
            }
            if j < chars.len() && chars[j] == close && j > i + 1 {
                spans.push((i, j + 1));
                i = j + 1;
                continue;
            }
        }
        i += 1;
    }
}

/// A path starts at `/`, `~/`, or `./`. A bare `~` does not, so "~2KB" stays prose and
/// keeps the sentence period that follows it.
fn find_paths(chars: &[char], is_segment_required: bool, spans: &mut Vec<(usize, usize)>) {
    let mut i = 0;
    while i < chars.len() {
        let is_slash = chars[i] == '/';
        let is_prefixed = matches!(chars[i], '~' | '.') && chars.get(i + 1) == Some(&'/');
        if !is_slash && !is_prefixed {
            i += 1;
            continue;
        }
        let mut j = if is_slash { i + 1 } else { i + 2 };
        let head = j;
        while j < chars.len() && is_path_char(chars[j]) {
            j += 1;
        }
        if j == head {
            i += 1;
            continue;
        }
        let mut segments = 0;
        loop {
            if j >= chars.len() || chars[j] != '/' {
                break;
            }
            let mut k = j + 1;
            while k < chars.len() && is_path_char(chars[k]) {
                k += 1;
            }
            if k == j + 1 {
                break;
            }
            segments += 1;
            j = k;
        }
        if is_segment_required && segments == 0 {
            i += 1;
            continue;
        }
        while j > head && matches!(chars[j - 1], '.' | '-') {
            j -= 1;
        }
        spans.push((i, j));
        i = j;
    }
}

fn find_urls(chars: &[char], spans: &mut Vec<(usize, usize)>) {
    let text: String = chars.iter().collect();
    let mut from = 0;
    while let Some(offset) = text[from..].find("http") {
        let start_byte = from + offset;
        let rest = &text[start_byte..];
        if rest.starts_with("http://") || rest.starts_with("https://") {
            let len = rest.find(char::is_whitespace).unwrap_or(rest.len());
            let start_char = text[..start_byte].chars().count();
            let end_char = start_char + text[start_byte..start_byte + len].chars().count();
            spans.push((start_char, end_char));
            from = start_byte + len;
            continue;
        }
        from = start_byte + 4;
    }
}

fn find_flag_lines(chars: &[char], spans: &mut Vec<(usize, usize)>) {
    let mut line_start = 0;
    for i in 0..=chars.len() {
        if i == chars.len() || chars[i] == '\n' {
            let line: String = chars[line_start..i].iter().collect();
            if line.starts_with("FLAG:") {
                spans.push((line_start, i));
            }
            line_start = i + 1;
        }
    }
}

pub fn exact(text: &str, profile: Profile) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut spans = Vec::new();
    find_fenced(&chars, &mut spans);
    find_delimited(&chars, '`', '`', &mut spans);
    find_delimited(&chars, '"', '"', &mut spans);
    find_delimited(&chars, '\u{201c}', '\u{201d}', &mut spans);
    match profile {
        Profile::Spoken => find_paths(&chars, true, &mut spans),
        Profile::Shipped => {
            find_urls(&chars, &mut spans);
            find_paths(&chars, false, &mut spans);
            find_flag_lines(&chars, &mut spans);
        }
    }
    let mut out = chars;
    for (start, end) in spans {
        for c in out.iter_mut().take(end).skip(start) {
            if *c != '\n' {
                *c = MASK;
            }
        }
    }
    out.into_iter().collect()
}
