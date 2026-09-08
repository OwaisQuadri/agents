use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde_json::Value;

const TEMPLATE: &str = include_str!("template.html");

#[derive(Default)]
struct Agg {
    input: u64,
    output: u64,
    cache_read: u64,
    cache_create: u64,
    messages: u64,
    first_ts: String,
    last_ts: String,
    first_ctx: u64,
    last_ctx: u64,
}

struct Key {
    source: &'static str,
    project: String,
    session: String,
    model: String,
}

type Rows = HashMap<(String, String, String), (Key, Agg)>;

fn main() -> ExitCode {
    let mut pi_dir = home().join(".pi").join("agent").join("sessions");
    let mut out_path = PathBuf::from("session-stats.html");
    let mut json_path: Option<PathBuf> = None;
    let mut is_open_requested = false;
    let mut is_out_set = false;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--pi-dir" => match args.next() {
                Some(value) => pi_dir = PathBuf::from(value),
                None => return usage_error("--pi-dir needs a directory"),
            },
            "--out" => match args.next() {
                Some(value) => {
                    out_path = PathBuf::from(value);
                    is_out_set = true;
                }
                None => return usage_error("--out needs a file path"),
            },
            "--json" => match args.next() {
                Some(value) => json_path = Some(PathBuf::from(value)),
                None => return usage_error("--json needs a file path or -"),
            },
            "--open" => is_open_requested = true,
            "--help" | "-h" => {
                println!(
                    "session-stats [--pi-dir <dir>] [--out <file.html>] [--open]\n\
                     \x20             [--json <file.json | ->]\n\
                     Scans Pi session records. Writes an interactive token-usage graph as one\n\
                     self-contained HTML file, or the aggregated rows as one JSON array with\n\
                     --json (- for stdout; skips the HTML file unless --out or --open is also\n\
                     given)."
                );
                return ExitCode::SUCCESS;
            }
            other => return usage_error(&format!("unknown argument: {other}")),
        }
    }

    let mut aggs: Rows = HashMap::new();
    scan_pi(&pi_dir, &mut aggs);

    let mut rows: Vec<(Key, Agg)> = aggs.into_values().collect();
    rows.sort_by(|left, right| left.1.first_ts.cmp(&right.1.first_ts));
    if rows.is_empty() {
        eprintln!("session-stats: no usage data found");
        return ExitCode::FAILURE;
    }

    let data = render_rows(&rows);
    if let Some(json_path) = &json_path {
        if json_path.as_os_str() == "-" {
            println!("{data}");
        } else if let Err(error) = fs::write(json_path, &data) {
            eprintln!("session-stats: write {}: {error}", json_path.display());
            return ExitCode::FAILURE;
        } else {
            println!("{} rows -> {}", rows.len(), json_path.display());
        }
    }
    if json_path.is_none() || is_out_set || is_open_requested {
        let html = TEMPLATE.replace("[/*__DATA__*/]", &data);
        if let Err(error) = fs::write(&out_path, html) {
            eprintln!("session-stats: write {}: {error}", out_path.display());
            return ExitCode::FAILURE;
        }
        println!("{} rows -> {}", rows.len(), out_path.display());
    }

    if is_open_requested {
        let status = std::process::Command::new("open").arg(&out_path).status();
        if !status.is_ok_and(|result| result.success()) {
            eprintln!("session-stats: failed to open {}", out_path.display());
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}

fn usage_error(message: &str) -> ExitCode {
    eprintln!("session-stats: {message} (see --help)");
    ExitCode::FAILURE
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()))
}

fn entry<'a>(
    aggs: &'a mut Rows,
    source: &'static str,
    project: &str,
    session: &str,
    model: &str,
) -> &'a mut Agg {
    &mut aggs
        .entry((source.to_string(), session.to_string(), model.to_string()))
        .or_insert_with(|| {
            (
                Key {
                    source,
                    project: project.to_string(),
                    session: session.to_string(),
                    model: model.to_string(),
                },
                Agg::default(),
            )
        })
        .1
}

fn add(
    agg: &mut Agg,
    input: u64,
    output: u64,
    cache_read: u64,
    cache_create: u64,
    ts: Option<&str>,
) {
    let context = input + cache_read + cache_create;
    agg.input += input;
    agg.output += output;
    agg.cache_read += cache_read;
    agg.cache_create += cache_create;
    agg.messages += 1;
    if let Some(ts) = ts {
        if agg.first_ts.is_empty() || ts < agg.first_ts.as_str() {
            agg.first_ts = ts.to_string();
            agg.first_ctx = context;
        }
        if ts >= agg.last_ts.as_str() {
            agg.last_ts = ts.to_string();
            agg.last_ctx = context;
        }
    }
}

fn jsonl_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files = Vec::new();
    for path in entries.flatten().map(|e| e.path()) {
        if path.is_dir() {
            files.extend(jsonl_files(&path));
        } else if path.extension().map(|e| e == "jsonl") == Some(true) {
            files.push(path);
        }
    }
    files
}

fn each_line(file: &Path, mut handle: impl FnMut(&str)) {
    let Ok(open) = fs::File::open(file) else {
        return;
    };
    let mut reader = BufReader::new(open);
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => handle(&line),
        }
    }
}

fn file_stem(file: &Path) -> String {
    file.file_stem()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn parent_name(file: &Path) -> String {
    file.parent()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn scan_pi(dir: &Path, aggs: &mut Rows) {
    for file in jsonl_files(dir) {
        let project = parent_name(&file);
        let session = file_stem(&file);
        each_line(&file, |line| {
            if !line.contains("\"usage\"") {
                return;
            }
            let Ok(row) = serde_json::from_str::<Value>(line) else {
                return;
            };
            if row["type"].as_str() != Some("message") {
                return;
            }
            let message = &row["message"];
            let usage = &message["usage"];
            if message["role"].as_str() != Some("assistant") || !usage.is_object() {
                return;
            }
            let Some(model) = message["model"].as_str() else {
                return;
            };
            add(
                entry(aggs, "pi", &project, &session, model),
                tokens(usage, "input"),
                tokens(usage, "output"),
                tokens(usage, "cacheRead"),
                tokens(usage, "cacheWrite"),
                row["timestamp"].as_str(),
            );
        });
    }
}

fn tokens(usage: &Value, field: &str) -> u64 {
    usage[field].as_u64().unwrap_or(0)
}

fn render_rows(rows: &[(Key, Agg)]) -> String {
    let mut out = String::from("[");
    for (index, (key, agg)) in rows.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str(&format!(
            "{{\"src\":\"{}\",\"project\":{},\"session\":{},\"model\":{},\"input\":{},\
             \"output\":{},\"cacheRead\":{},\"cacheCreate\":{},\"messages\":{},\
             \"first\":{},\"last\":{},\"firstCtx\":{},\"lastCtx\":{}}}",
            key.source,
            Value::String(key.project.clone()),
            Value::String(key.session.clone()),
            Value::String(key.model.clone()),
            agg.input,
            agg.output,
            agg.cache_read,
            agg.cache_create,
            agg.messages,
            Value::String(agg.first_ts.clone()),
            Value::String(agg.last_ts.clone()),
            agg.first_ctx,
            agg.last_ctx,
        ));
    }
    out.push(']');
    out
}
