//! Blocking loopback review gate for skills/pr-review. Reads a GateInput JSON file (a
//! PR's diff, a blast-radius related-files list, and a drafted review), serves one HTML
//! page rendering all three, waits for an Approve or Decline-with-feedback POST from the
//! browser, prints that decision as one JSON line to stdout, and exits. No persistence,
//! no auth: loopback-only, single-shot, same shape a caller gets from
//! plannotator_submit_plan's own blocking approve/deny loop.

use std::env;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, ExitCode};

use serde::{Deserialize, Serialize};

const PAGE_TEMPLATE: &str = include_str!("page.html");
// A decision body is just {verdict, feedback}; it never approaches this. The cap exists
// so a claimed Content-Length can never force a large allocation before the body is read.
const MAX_DECISION_BODY_BYTES: usize = 64 * 1024;

#[derive(Deserialize)]
struct GateInput {
    pr: PrInfo,
    #[expect(
        dead_code,
        reason = "validates the field is present; the raw JSON string is what actually renders"
    )]
    diff: Vec<DiffEntry>,
    #[expect(
        dead_code,
        reason = "validates the field is present; the raw JSON string is what actually renders"
    )]
    draft: serde_json::Value,
}

#[derive(Deserialize)]
struct PrInfo {
    #[allow(dead_code)]
    number: u64,
    #[allow(dead_code)]
    title: String,
    #[allow(dead_code)]
    url: String,
}

#[derive(Deserialize)]
struct DiffEntry {
    #[allow(dead_code)]
    file: String,
    #[allow(dead_code)]
    is_related: bool,
    #[allow(dead_code)]
    patch_or_content: String,
    /// Present when `is_related` is true, copied from the matching `RelatedFile.reason`
    /// in the draft; absent on a changed file, which has no blast-radius reason.
    #[allow(dead_code)]
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug)]
struct GateDecision {
    verdict: String,
    feedback: Option<String>,
}

fn main() -> ExitCode {
    match run(env::args().skip(1)) {
        Ok(decision) => {
            println!(
                "{}",
                serde_json::to_string(&decision).expect("serialize decision")
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("pr-review-gate: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: impl Iterator<Item = String>) -> Result<GateDecision, String> {
    let cli = parse_arguments(args)?;
    let raw = fs::read_to_string(&cli.input_path)
        .map_err(|error| format!("cannot read {}: {error}", cli.input_path))?;
    let gate_input: GateInput = serde_json::from_str(&raw)
        .map_err(|error| format!("{} is not a valid GateInput: {error}", cli.input_path))?;
    let page = render_page(PAGE_TEMPLATE, &raw);

    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|error| format!("cannot bind loopback: {error}"))?;
    let port = listener
        .local_addr()
        .map_err(|error| format!("cannot read bound port: {error}"))?
        .port();
    let url = format!("http://127.0.0.1:{port}/");
    eprintln!(
        "pr-review-gate: reviewing PR #{} at {url}",
        gate_input.pr.number
    );

    if !cli.is_open_disabled {
        open_browser(&url);
    }

    // A per-connection failure (a stray reset, a half-open probe, a scanner) must never
    // take down the whole gate and lose the pending human decision. Log it and keep
    // accepting; only a fatal setup error above ends the process without a decision.
    for stream in listener.incoming() {
        let stream = match stream {
            Ok(stream) => stream,
            Err(error) => {
                eprintln!("pr-review-gate: dropped a connection: {error}");
                continue;
            }
        };
        match handle_connection(stream, &page) {
            Ok(Some(decision)) => return Ok(decision),
            Ok(None) => {}
            Err(error) => eprintln!("pr-review-gate: dropped a connection: {error}"),
        }
    }
    Err("listener closed without a decision".to_owned())
}

struct Cli {
    input_path: String,
    is_open_disabled: bool,
}

fn parse_arguments(mut args: impl Iterator<Item = String>) -> Result<Cli, String> {
    let mut input_path = None;
    let mut is_open_disabled = false;
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--input" => {
                input_path = Some(
                    args.next()
                        .ok_or_else(|| "--input requires a path".to_owned())?,
                );
            }
            "--no-open" => is_open_disabled = true,
            other => {
                return Err(format!(
                    "unknown argument {other}; usage: pr-review-gate --input <path> [--no-open]"
                ))
            }
        }
    }
    Ok(Cli {
        input_path: input_path.ok_or_else(|| "--input is required".to_owned())?,
        is_open_disabled,
    })
}

fn open_browser(url: &str) {
    let outcome = if cfg!(target_os = "macos") {
        Command::new("open").arg(url).spawn()
    } else {
        Command::new("xdg-open").arg(url).spawn()
    };
    if let Err(error) = outcome {
        eprintln!("pr-review-gate: could not auto-open a browser ({error}); open {url} manually");
    }
}

/// Embeds `data_json` into the page's `<script type="application/json">` block. Every
/// `</` in the payload becomes `<\/` first, so a value containing the literal text
/// `</script` can never terminate the tag early; `\/` is a valid JSON escape for `/`, so
/// `JSON.parse` on the browser side sees the exact original text back.
fn render_page(template: &str, data_json: &str) -> String {
    let escaped = data_json.replace("</", "<\\/");
    template.replace("__GATE_DATA__", &escaped)
}

fn handle_connection(mut stream: TcpStream, page: &str) -> Result<Option<GateDecision>, String> {
    let mut reader = BufReader::new(
        stream
            .try_clone()
            .map_err(|error| format!("cannot clone stream: {error}"))?,
    );
    let mut request_line = String::new();
    reader
        .read_line(&mut request_line)
        .map_err(|error| format!("cannot read request: {error}"))?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_owned();
    let path = parts.next().unwrap_or("").to_owned();

    let mut content_length = 0usize;
    loop {
        let mut header_line = String::new();
        reader
            .read_line(&mut header_line)
            .map_err(|error| format!("cannot read headers: {error}"))?;
        let trimmed = header_line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        if let Some((name, value)) = trimmed.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                content_length = value.trim().parse().unwrap_or(0);
            }
        }
    }

    match (method.as_str(), path.as_str()) {
        ("GET", "/") => {
            write_response(&mut stream, 200, "text/html", page)?;
            Ok(None)
        }
        ("POST", "/decision") => {
            if content_length > MAX_DECISION_BODY_BYTES {
                write_response(&mut stream, 400, "text/plain", "decision body too large")?;
                return Ok(None);
            }
            let mut body = vec![0u8; content_length];
            reader
                .read_exact(&mut body)
                .map_err(|error| format!("cannot read decision body: {error}"))?;
            let body_text = String::from_utf8_lossy(&body);
            match parse_decision(&body_text) {
                Ok(decision) => {
                    write_response(&mut stream, 200, "application/json", "{}")?;
                    Ok(Some(decision))
                }
                Err(error) => {
                    write_response(&mut stream, 400, "text/plain", &error)?;
                    Ok(None)
                }
            }
        }
        _ => {
            write_response(&mut stream, 404, "text/plain", "not found")?;
            Ok(None)
        }
    }
}

fn parse_decision(body: &str) -> Result<GateDecision, String> {
    let decision: GateDecision = serde_json::from_str(body)
        .map_err(|error| format!("decision body is not valid JSON: {error}"))?;
    match decision.verdict.as_str() {
        "approve" | "decline" => Ok(decision),
        other => Err(format!("verdict must be approve or decline, got {other}")),
    }
}

fn write_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &str,
) -> Result<(), String> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        _ => "Error",
    };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .map_err(|error| format!("cannot write response: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_page_and_escapes_closing_script_tags() {
        let template = "before __GATE_DATA__ after";
        let data = r#"{"text":"</script>steal me"}"#;
        let rendered = render_page(template, data);
        assert!(!rendered.contains("</script>steal me"));
        assert!(rendered.contains(r#"<\/script>steal me"#));
    }

    #[test]
    fn parses_approve_and_decline_decisions() {
        let approve =
            parse_decision(r#"{"verdict":"approve","feedback":null}"#).expect("approve parses");
        assert_eq!(approve.verdict, "approve");
        assert_eq!(approve.feedback, None);

        let decline = parse_decision(r#"{"verdict":"decline","feedback":"fix the null check"}"#)
            .expect("decline parses");
        assert_eq!(decline.verdict, "decline");
        assert_eq!(decline.feedback.as_deref(), Some("fix the null check"));
    }

    #[test]
    fn rejects_an_unknown_verdict() {
        let error =
            parse_decision(r#"{"verdict":"maybe","feedback":null}"#).expect_err("must reject");
        assert!(error.contains("maybe"), "{error}");
    }

    #[test]
    fn rejects_malformed_json() {
        let error = parse_decision("not json").expect_err("must reject");
        assert!(error.contains("not valid JSON"), "{error}");
    }

    #[test]
    fn parses_required_and_optional_arguments() {
        let cli = parse_arguments(["--input", "/tmp/gate.json"].into_iter().map(str::to_owned))
            .expect("parses");
        assert_eq!(cli.input_path, "/tmp/gate.json");
        assert!(!cli.is_open_disabled);

        let cli = parse_arguments(
            ["--input", "/tmp/gate.json", "--no-open"]
                .into_iter()
                .map(str::to_owned),
        )
        .expect("parses");
        assert!(cli.is_open_disabled);

        assert!(parse_arguments(std::iter::empty()).is_err());
        assert!(parse_arguments(["--bogus".to_owned()].into_iter()).is_err());
    }

    #[test]
    fn rejects_a_gate_input_missing_required_fields() {
        let broken = r#"{"pr":{"number":1}}"#;
        assert!(serde_json::from_str::<GateInput>(broken).is_err());
    }
}
