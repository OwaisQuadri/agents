// Parses SRT and WebVTT caption files into a flat list of timed cues. Both formats
// share the same cue shape (a time range plus one or more lines of text); they only
// differ in decimal separator (SRT uses a comma, VTT a dot) and in VTT's optional
// leading "WEBVTT" header line, which this parser skips rather than rejects.

#[derive(Debug, Clone, PartialEq)]
pub struct Cue {
    pub start_s: f64,
    pub end_s: f64,
    pub text: String,
}

pub fn parse(input: &str) -> Result<Vec<Cue>, String> {
    let mut cues = Vec::new();
    let mut lines = input.lines().peekable();

    if let Some(first) = lines.peek() {
        if first.trim_start().starts_with("WEBVTT") {
            lines.next();
        }
    }

    let mut block: Vec<&str> = Vec::new();
    loop {
        match lines.next() {
            Some(line) if !line.trim().is_empty() => block.push(line),
            _ => {
                if !block.is_empty() {
                    if let Some(cue) = parse_block(&block)? {
                        cues.push(cue);
                    }
                    block.clear();
                }
                if lines.peek().is_none() {
                    break;
                }
            }
        }
    }

    Ok(cues)
}

fn parse_block(block: &[&str]) -> Result<Option<Cue>, String> {
    // A block is either:
    //   <sequence-number>        (SRT only)
    //   <start> --> <end>
    //   <text line 1>
    //   <text line 2...>
    // or the same without the leading sequence number (VTT, or SRT with it stripped).
    let mut idx = 0;
    if idx < block.len() && !block[idx].contains("-->") {
        idx += 1; // skip the SRT sequence-number line
    }
    let timing_line = block
        .get(idx)
        .ok_or_else(|| "cue block missing a timing line".to_string())?;
    let (start_s, end_s) = parse_timing_line(timing_line)?;
    let text = block[idx + 1..].join("\n");
    Ok(Some(Cue {
        start_s,
        end_s,
        text,
    }))
}

fn parse_timing_line(line: &str) -> Result<(f64, f64), String> {
    let mut parts = line.splitn(2, "-->");
    let start = parts
        .next()
        .ok_or_else(|| format!("malformed timing line: {line}"))?;
    let end = parts
        .next()
        .ok_or_else(|| format!("malformed timing line (no '-->'): {line}"))?;
    // The end side may carry trailing VTT cue settings (e.g. "align:middle") after
    // the timestamp; only the first whitespace-delimited token is the timestamp.
    let end = end.split_whitespace().next().unwrap_or(end.trim());
    Ok((parse_timestamp(start.trim())?, parse_timestamp(end)?))
}

fn parse_timestamp(raw: &str) -> Result<f64, String> {
    let normalized = raw.replace(',', ".");
    let parts: Vec<&str> = normalized.split(':').collect();
    let (h, m, s) = match parts.as_slice() {
        [h, m, s] => (*h, *m, *s),
        [m, s] => ("0", *m, *s),
        _ => return Err(format!("malformed timestamp: {raw}")),
    };
    let h: f64 = h.parse().map_err(|_| format!("bad hours in timestamp: {raw}"))?;
    let m: f64 = m.parse().map_err(|_| format!("bad minutes in timestamp: {raw}"))?;
    let s: f64 = s.parse().map_err(|_| format!("bad seconds in timestamp: {raw}"))?;
    Ok(h * 3600.0 + m * 60.0 + s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_basic_srt() {
        let input = "1\n00:00:02,000 --> 00:00:04,000\nHello world\n\n2\n00:00:04,500 --> 00:00:06,000\nSecond cue\n";
        let cues = parse(input).unwrap();
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].start_s, 2.0);
        assert_eq!(cues[0].end_s, 4.0);
        assert_eq!(cues[0].text, "Hello world");
        assert_eq!(cues[1].start_s, 4.5);
        assert_eq!(cues[1].text, "Second cue");
    }

    #[test]
    fn parses_basic_vtt_with_header() {
        let input = "WEBVTT\n\n00:00:02.000 --> 00:00:04.000\nHello world\n";
        let cues = parse(input).unwrap();
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].start_s, 2.0);
        assert_eq!(cues[0].end_s, 4.0);
    }

    #[test]
    fn parses_multiline_cue_text() {
        let input = "1\n00:00:02,000 --> 00:00:04,000\nLine one\nLine two\n";
        let cues = parse(input).unwrap();
        assert_eq!(cues[0].text, "Line one\nLine two");
    }

    #[test]
    fn parses_vtt_cue_settings_after_end_timestamp() {
        let input = "WEBVTT\n\n00:00:02.000 --> 00:00:04.000 align:middle line:90%\nHello\n";
        let cues = parse(input).unwrap();
        assert_eq!(cues[0].end_s, 4.0);
    }

    #[test]
    fn parses_short_mm_ss_timestamps() {
        let input = "1\n00:02,000 --> 00:04,000\nShort form\n";
        let cues = parse(input).unwrap();
        assert_eq!(cues[0].start_s, 2.0);
    }

    #[test]
    fn malformed_timing_line_is_an_error_not_a_panic() {
        let input = "1\nnot a timing line\ntext\n";
        assert!(parse(input).is_err());
    }

    #[test]
    fn empty_input_yields_no_cues() {
        assert_eq!(parse("").unwrap(), vec![]);
    }
}
