use std::collections::{HashMap, HashSet};
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
    let mut claude_dir = home().join(".claude").join("projects");
    let mut pi_dir = home().join(".pi").join("agent").join("sessions");
    let mut codex_dir = home().join(".codex").join("sessions");
    let mut cursor_db =
        home().join("Library/Application Support/Cursor/User/globalStorage/state.vscdb");
    let mut out_path = PathBuf::from("session-stats.html");
    let mut json_path: Option<PathBuf> = None;
    let mut is_open_requested = false;
    let mut is_out_set = false;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--claude-dir" => match args.next() {
                Some(v) => claude_dir = PathBuf::from(v),
                None => return usage_error("--claude-dir needs a directory"),
            },
            "--pi-dir" => match args.next() {
                Some(v) => pi_dir = PathBuf::from(v),
                None => return usage_error("--pi-dir needs a directory"),
            },
            "--codex-dir" => match args.next() {
                Some(v) => codex_dir = PathBuf::from(v),
                None => return usage_error("--codex-dir needs a directory"),
            },
            "--cursor-db" => match args.next() {
                Some(v) => cursor_db = PathBuf::from(v),
                None => return usage_error("--cursor-db needs a file path"),
            },
            "--out" => match args.next() {
                Some(v) => {
                    out_path = PathBuf::from(v);
                    is_out_set = true;
                }
                None => return usage_error("--out needs a file path"),
            },
            "--json" => match args.next() {
                Some(v) => json_path = Some(PathBuf::from(v)),
                None => return usage_error("--json needs a file path or -"),
            },
            "--open" => is_open_requested = true,
            "--help" | "-h" => {
                println!(
                    "session-stats [--claude-dir <dir>] [--pi-dir <dir>] [--codex-dir <dir>]\n\
                     \x20             [--cursor-db <state.vscdb>] [--out <file.html>] [--open]\n\
                     \x20             [--json <file.json | ->]\n\
                     Scans Claude Code, Pi, Codex, and Cursor session records. Writes an\n\
                     interactive token-usage graph as one self-contained HTML file, or the\n\
                     aggregated rows as one JSON array with --json (- for stdout; skips\n\
                     the HTML file unless --out or --open is also given)."
                );
                return ExitCode::SUCCESS;
            }
            other => return usage_error(&format!("unknown argument: {other}")),
        }
    }

    let mut aggs: Rows = HashMap::new();
    scan_claude(&claude_dir, &mut aggs);
    scan_pi(&pi_dir, &mut aggs);
    scan_codex(&codex_dir, &mut aggs);
    scan_cursor(&cursor_db, &mut aggs);

    let mut rows: Vec<(Key, Agg)> = aggs.into_values().collect();
    rows.sort_by(|a, b| a.1.first_ts.cmp(&b.1.first_ts));
    if rows.is_empty() {
        eprintln!("session-stats: no usage data found");
        return ExitCode::FAILURE;
    }

    let data = render_rows(&rows);
    if let Some(json_path) = &json_path {
        if json_path.as_os_str() == "-" {
            println!("{data}");
        } else if let Err(err) = fs::write(json_path, &data) {
            eprintln!("session-stats: write {}: {err}", json_path.display());
            return ExitCode::FAILURE;
        } else {
            println!("{} rows -> {}", rows.len(), json_path.display());
        }
    }
    if json_path.is_none() || is_out_set || is_open_requested {
        let html = TEMPLATE.replace("[/*__DATA__*/]", &data);
        if let Err(err) = fs::write(&out_path, html) {
            eprintln!("session-stats: write {}: {err}", out_path.display());
            return ExitCode::FAILURE;
        }
        println!("{} rows -> {}", rows.len(), out_path.display());
    }

    if is_open_requested {
        let status = std::process::Command::new("open").arg(&out_path).status();
        if !status.map(|s| s.success()).unwrap_or(false) {
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

fn scan_claude(dir: &Path, aggs: &mut Rows) {
    for file in jsonl_files(dir) {
        let project = parent_name(&file);
        let session = file_stem(&file);
        // A message id can repeat across lines as the transcript re-emits the
        // same API response; only its first occurrence carries new tokens.
        let mut seen: HashSet<String> = HashSet::new();
        each_line(&file, |line| {
            if !line.contains("\"usage\"") {
                return;
            }
            let Ok(row) = serde_json::from_str::<Value>(line) else {
                return;
            };
            let message = &row["message"];
            let usage = &message["usage"];
            if !usage.is_object() {
                return;
            }
            let model = match message["model"].as_str() {
                Some(m) if !m.starts_with('<') => m,
                _ => return,
            };
            if let Some(id) = message["id"].as_str() {
                let request = row["requestId"].as_str().unwrap_or("");
                if !seen.insert(format!("{id}/{request}")) {
                    return;
                }
            }
            add(
                entry(aggs, "claude", &project, &session, model),
                tokens(usage, "input_tokens"),
                tokens(usage, "output_tokens"),
                tokens(usage, "cache_read_input_tokens"),
                tokens(usage, "cache_creation_input_tokens"),
                row["timestamp"].as_str(),
            );
        });
    }
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

fn scan_codex(dir: &Path, aggs: &mut Rows) {
    for file in jsonl_files(dir) {
        let session = file_stem(&file);
        let mut project = String::new();
        let mut model = String::from("unknown");
        // Codex re-emits token_count with unchanged totals on rate-limit
        // refreshes; only a growing running total carries new tokens.
        let mut prev_total = 0u64;
        each_line(&file, |line| {
            let is_relevant = line.contains("\"token_count\"")
                || line.contains("\"turn_context\"")
                || line.contains("\"session_meta\"");
            if !is_relevant {
                return;
            }
            let Ok(row) = serde_json::from_str::<Value>(line) else {
                return;
            };
            let payload = &row["payload"];
            match row["type"].as_str() {
                Some("session_meta") => {
                    if let Some(cwd) = payload["cwd"].as_str() {
                        project = cwd.to_string();
                    }
                }
                Some("turn_context") => {
                    if let Some(m) = payload["model"].as_str() {
                        model = m.to_string();
                    }
                }
                Some("event_msg") if payload["type"].as_str() == Some("token_count") => {
                    let info = &payload["info"];
                    let last = &info["last_token_usage"];
                    if !last.is_object() {
                        return;
                    }
                    let total = info["total_token_usage"]["total_tokens"]
                        .as_u64()
                        .unwrap_or(0);
                    if total == prev_total {
                        return;
                    }
                    prev_total = total;
                    let cached = tokens(last, "cached_input_tokens");
                    add(
                        entry(aggs, "codex", &project, &session, &model),
                        tokens(last, "input_tokens").saturating_sub(cached),
                        tokens(last, "output_tokens"),
                        cached,
                        tokens(last, "cache_write_input_tokens"),
                        row["timestamp"].as_str(),
                    );
                }
                _ => {}
            }
        });
    }
}

fn scan_cursor(db_path: &Path, aggs: &mut Rows) {
    // Cursor holds the db open with a write lock, so read a snapshot copy.
    let copy = std::env::temp_dir().join("session-stats-cursor.vscdb");
    if fs::copy(db_path, &copy).is_err() {
        return;
    }
    for suffix in ["-wal", "-shm"] {
        let mut side = db_path.as_os_str().to_owned();
        side.push(suffix);
        let mut side_copy = copy.as_os_str().to_owned();
        side_copy.push(suffix);
        let _ = fs::copy(PathBuf::from(side), PathBuf::from(side_copy));
    }
    let Ok(conn) =
        rusqlite::Connection::open_with_flags(&copy, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
    else {
        return;
    };

    struct Meta {
        model: String,
        name: String,
        first_ms: i64,
        last_ms: i64,
    }
    let mut metas: HashMap<String, Meta> = HashMap::new();
    let read = |sql: &str, handle: &mut dyn FnMut(&str, &Value)| {
        let Ok(mut stmt) = conn.prepare(sql) else {
            return;
        };
        let Ok(mut rows) = stmt.query([]) else { return };
        while let Ok(Some(row)) = rows.next() {
            let (Ok(key), Ok(value)) = (row.get::<_, String>(0), row.get::<_, String>(1)) else {
                continue;
            };
            let Ok(json) = serde_json::from_str::<Value>(&value) else {
                continue;
            };
            handle(&key, &json);
        }
    };

    read(
        "SELECT key, value FROM cursorDiskKV WHERE key LIKE 'composerData:%'",
        &mut |key, json| {
            let id = key.trim_start_matches("composerData:").to_string();
            let created = json["createdAt"].as_i64().unwrap_or(0);
            metas.insert(
                id,
                Meta {
                    model: json["modelConfig"]["modelName"]
                        .as_str()
                        .unwrap_or("unknown")
                        .to_string(),
                    name: json["name"].as_str().unwrap_or("").to_string(),
                    first_ms: created,
                    last_ms: json["lastUpdatedAt"].as_i64().unwrap_or(created),
                },
            );
        },
    );
    read(
        "SELECT key, value FROM cursorDiskKV WHERE key LIKE 'bubbleId:%'",
        &mut |key, json| {
            if json["type"].as_i64() != Some(2) {
                return;
            }
            let Some(composer) = key.split(':').nth(1) else {
                return;
            };
            let Some(meta) = metas.get(composer) else {
                return;
            };
            let usage = &json["tokenCount"];
            add(
                entry(aggs, "cursor", &meta.name, composer, &meta.model),
                tokens(usage, "inputTokens"),
                tokens(usage, "outputTokens"),
                0,
                0,
                None,
            );
        },
    );
    for (composer, meta) in &metas {
        let key = ("cursor".to_string(), composer.clone(), meta.model.clone());
        if let Some((_, agg)) = aggs.get_mut(&key) {
            agg.first_ts = iso_from_ms(meta.first_ms);
            agg.last_ts = iso_from_ms(meta.last_ms.max(meta.first_ms));
        }
    }
}

fn iso_from_ms(ms: i64) -> String {
    if ms <= 0 {
        return String::new();
    }
    let secs = ms / 1000;
    let (days, rem) = (secs / 86_400, secs % 86_400);
    // civil_from_days, Howard Hinnant's date algorithms
    // (howardhinnant.github.io/date_algorithms.html)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.000Z",
        year,
        month,
        day,
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
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
