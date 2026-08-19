//! Tails a log file for Codex/Claude usage-limit text, and once seen, schedules a
//! detached `sleep <wait> && <resume-cmd>` so the wrapped CLI resumes itself once the
//! limit resets. Secondary to the Pi extension in pi/extensions/usage-limit-continue.ts:
//! this reads plain text only (no HTTP headers, no model metadata), so it cannot exempt
//! local models on its own — pass --local for a log known to be a local model's.
//!
//! Usage: usage-limit-watch --log <path> --resume-cmd "<shell command>" [--local]
//!        [--poll-ms <n>] [--state-dir <path>]
//!
//! Detection, in order: an explicit "resets in <duration>" or "try again in <duration>";
//! an explicit "resets at <clock time>"; failing both, the 5-hour session window or the
//! 7-day weekly window named in the text. A usage-limit phrase with none of these present
//! is reported and left unscheduled.

use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const SESSION_WINDOW: Duration = Duration::from_secs(5 * 60 * 60);
const WEEKLY_WINDOW: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const USAGE_LIMIT_NEEDLES: &[&str] = &[
    "usage limit",
    "rate limit",
    "ratelimit",
    "too many requests",
    " 429",
    "quota exceeded",
    "weekly limit",
    "5-hour limit",
    "five-hour limit",
    "session limit",
];

#[derive(Debug, PartialEq, Eq)]
enum Plan {
    NotDetected,
    DetectedNoTime,
    DetectedAfter(Duration),
}

fn detect(line: &str) -> Plan {
    let lower = line.to_lowercase();
    let is_limit = USAGE_LIMIT_NEEDLES.iter().any(|needle| lower.contains(needle));
    if !is_limit {
        return Plan::NotDetected;
    }
    if let Some(wait) = parse_duration_after(&lower, "resets in").or_else(|| parse_duration_after(&lower, "try again in")) {
        return Plan::DetectedAfter(wait);
    }
    if let Some(wait) = parse_clock_after(&lower, "resets at") {
        return Plan::DetectedAfter(wait);
    }
    if lower.contains("5-hour") || lower.contains("five-hour") || lower.contains("session limit") {
        return Plan::DetectedAfter(SESSION_WINDOW);
    }
    if lower.contains("weekly") || lower.contains("7-day") || lower.contains("seven-day") {
        return Plan::DetectedAfter(WEEKLY_WINDOW);
    }
    Plan::DetectedNoTime
}

/// Parses "<marker> 2h 30m", "<marker> 45 seconds", etc. into a wait duration.
fn parse_duration_after(lower: &str, marker: &str) -> Option<Duration> {
    let start = lower.find(marker)? + marker.len();
    let window = &lower[start..lower.len().min(start + 40)];
    let mut chars = window.char_indices().peekable();
    let mut total = Duration::ZERO;
    let mut found_any = false;
    while let Some((i, ch)) = chars.peek().copied() {
        if ch.is_ascii_digit() {
            let digit_start = i;
            let mut digit_end = i;
            while let Some((j, d)) = chars.peek().copied() {
                if d.is_ascii_digit() {
                    digit_end = j + d.len_utf8();
                    chars.next();
                } else {
                    break;
                }
            }
            let number: u64 = window[digit_start..digit_end].parse().ok()?;
            while let Some((_, c)) = chars.peek().copied() {
                if c == ' ' {
                    chars.next();
                } else {
                    break;
                }
            }
            match chars.peek().copied() {
                Some((_, 'd')) => {
                    total += Duration::from_secs(number * 24 * 60 * 60);
                    found_any = true;
                }
                Some((_, 'h')) => {
                    total += Duration::from_secs(number * 60 * 60);
                    found_any = true;
                }
                Some((_, 'm')) => {
                    total += Duration::from_secs(number * 60);
                    found_any = true;
                }
                Some((_, 's')) => {
                    total += Duration::from_secs(number);
                    found_any = true;
                }
                _ => break,
            }
            chars.next();
        } else if ch == ' ' {
            chars.next();
        } else {
            break;
        }
    }
    if found_any {
        Some(total)
    } else {
        None
    }
}

/// Parses "<marker> 3:00pm", "<marker> 15:00" into a wait duration from `now`.
fn parse_clock_after(lower: &str, marker: &str) -> Option<Duration> {
    let start = lower.find(marker)? + marker.len();
    let window = lower[start..].trim_start();
    let mut chars = window.char_indices().peekable();

    let hour_start = 0;
    let mut hour_end = 0;
    while let Some((j, d)) = chars.peek().copied() {
        if d.is_ascii_digit() && j - hour_start < 2 {
            hour_end = j + d.len_utf8();
            chars.next();
        } else {
            break;
        }
    }
    if hour_end == hour_start {
        return None;
    }
    let mut hour: u32 = window[hour_start..hour_end].parse().ok()?;

    let mut minute: u32 = 0;
    if let Some((_, ':')) = chars.peek().copied() {
        chars.next();
        let minute_start = hour_end + 1;
        let mut minute_end = minute_start;
        while let Some((j, d)) = chars.peek().copied() {
            if d.is_ascii_digit() && j - minute_start < 2 {
                minute_end = j + d.len_utf8();
                chars.next();
            } else {
                break;
            }
        }
        minute = window.get(minute_start..minute_end)?.parse().ok()?;
    }

    while let Some((_, ' ')) = chars.peek().copied() {
        chars.next();
    }
    if window[chars.peek().map(|(i, _)| *i).unwrap_or(window.len())..].starts_with("pm") && hour < 12 {
        hour += 12;
    } else if window[chars.peek().map(|(i, _)| *i).unwrap_or(window.len())..].starts_with("am") && hour == 12 {
        hour = 0;
    }
    if hour > 23 || minute > 59 {
        return None;
    }

    let now = SystemTime::now();
    let now_secs = now.duration_since(UNIX_EPOCH).ok()?.as_secs();
    let seconds_since_midnight = now_secs % 86400;
    let target_seconds = u64::from(hour) * 3600 + u64::from(minute) * 60;
    let wait_secs = if target_seconds > seconds_since_midnight {
        target_seconds - seconds_since_midnight
    } else {
        86400 - seconds_since_midnight + target_seconds
    };
    Some(Duration::from_secs(wait_secs))
}

fn state_dir(override_dir: &Option<PathBuf>) -> PathBuf {
    if let Some(dir) = override_dir {
        return dir.clone();
    }
    let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
    Path::new(&home).join(".config").join("usage-limit-watch")
}

fn already_scheduled(state_dir: &Path, log_path: &Path) -> bool {
    let marker = state_dir.join(marker_file_name(log_path));
    marker.exists()
}

fn marker_file_name(log_path: &Path) -> String {
    let raw = log_path.to_string_lossy();
    let safe: String = raw
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("{safe}.json")
}

fn record_schedule(state_dir: &Path, log_path: &Path, wait: Duration, resume_cmd: &str) -> std::io::Result<()> {
    fs::create_dir_all(state_dir)?;
    let marker = state_dir.join(marker_file_name(log_path));
    let mut file = File::create(marker)?;
    let scheduled_at = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    writeln!(
        file,
        "{{\"logPath\":\"{}\",\"waitSeconds\":{},\"scheduledAtEpoch\":{},\"resumeCmd\":\"{}\"}}",
        log_path.display(),
        wait.as_secs(),
        scheduled_at,
        resume_cmd.replace('"', "'"),
    )
}

fn schedule_resume(wait: Duration, resume_cmd: &str) -> std::io::Result<()> {
    let shell_cmd = format!("sleep {} && {}", wait.as_secs(), resume_cmd);
    Command::new("/bin/sh")
        .arg("-c")
        .arg(shell_cmd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(())
}

struct Args {
    log_path: PathBuf,
    resume_cmd: String,
    is_local: bool,
    poll: Duration,
    state_dir: Option<PathBuf>,
}

fn parse_args() -> Result<Args, String> {
    let mut log_path = None;
    let mut resume_cmd = None;
    let mut is_local = false;
    let mut poll_ms = 2000u64;
    let mut state_dir = None;

    let mut raw = env::args().skip(1);
    while let Some(flag) = raw.next() {
        match flag.as_str() {
            "--log" => log_path = Some(PathBuf::from(raw.next().ok_or("--log needs a path")?)),
            "--resume-cmd" => resume_cmd = Some(raw.next().ok_or("--resume-cmd needs a command")?),
            "--local" => is_local = true,
            "--poll-ms" => poll_ms = raw.next().ok_or("--poll-ms needs a number")?.parse().map_err(|_| "--poll-ms must be a number")?,
            "--state-dir" => state_dir = Some(PathBuf::from(raw.next().ok_or("--state-dir needs a path")?)),
            other => return Err(format!("unknown argument {other}")),
        }
    }

    Ok(Args {
        log_path: log_path.ok_or("--log is required")?,
        resume_cmd: resume_cmd.ok_or("--resume-cmd is required")?,
        is_local,
        poll: Duration::from_millis(poll_ms),
        state_dir,
    })
}

fn main() {
    let args = match parse_args() {
        Ok(args) => args,
        Err(message) => {
            eprintln!("usage-limit-watch: {message}");
            std::process::exit(2);
        }
    };

    if args.is_local {
        println!("usage-limit-watch: --local set, this log is exempt, exiting");
        return;
    }

    let state_dir = state_dir(&args.state_dir);
    if already_scheduled(&state_dir, &args.log_path) {
        println!("usage-limit-watch: a resume is already scheduled for {}", args.log_path.display());
        return;
    }

    let mut file = match OpenOptions::new().read(true).open(&args.log_path) {
        Ok(file) => file,
        Err(error) => {
            eprintln!("usage-limit-watch: could not open {}: {error}", args.log_path.display());
            std::process::exit(1);
        }
    };
    let mut position = file.seek(SeekFrom::End(0)).unwrap_or(0);

    println!("usage-limit-watch: watching {}", args.log_path.display());
    loop {
        let metadata = fs::metadata(&args.log_path);
        let current_len = metadata.map(|m| m.len()).unwrap_or(position);
        if current_len < position {
            position = 0;
        }
        if current_len > position && file.seek(SeekFrom::Start(position)).is_ok() {
            let mut reader = BufReader::new(&mut file);
            let mut line = String::new();
            while reader.read_line(&mut line).unwrap_or(0) > 0 {
                match detect(&line) {
                    Plan::DetectedAfter(wait) => {
                        println!("usage-limit-watch: usage limit detected, resuming in {}s", wait.as_secs());
                        if let Err(error) = schedule_resume(wait, &args.resume_cmd) {
                            eprintln!("usage-limit-watch: could not schedule resume: {error}");
                            std::process::exit(1);
                        }
                        if let Err(error) = record_schedule(&state_dir, &args.log_path, wait, &args.resume_cmd) {
                            eprintln!("usage-limit-watch: could not record schedule: {error}");
                        }
                        return;
                    }
                    Plan::DetectedNoTime => {
                        println!("usage-limit-watch: usage limit detected, but no reset time was parseable; not scheduling");
                    }
                    Plan::NotDetected => {}
                }
                line.clear();
            }
            position = file.stream_position().unwrap_or(position);
        }
        thread::sleep(args.poll);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_duration_phrasing() {
        assert_eq!(detect("Rate limited. Resets in 2h 30m."), Plan::DetectedAfter(Duration::from_secs(2 * 3600 + 30 * 60)));
        assert_eq!(detect("429 too many requests, try again in 45 seconds"), Plan::DetectedAfter(Duration::from_secs(45)));
    }

    #[test]
    fn detects_clock_phrasing_within_a_day() {
        let plan = detect("5-hour limit reached, resets at 23:59");
        match plan {
            Plan::DetectedAfter(wait) => assert!(wait <= Duration::from_secs(86400)),
            other => panic!("expected DetectedAfter, got {other:?}"),
        }
    }

    #[test]
    fn falls_back_to_the_named_window_with_no_instant() {
        assert_eq!(detect("You've hit your 5-hour limit."), Plan::DetectedAfter(SESSION_WINDOW));
        assert_eq!(detect("You've hit your weekly limit."), Plan::DetectedAfter(WEEKLY_WINDOW));
    }

    #[test]
    fn detected_with_no_time_and_no_named_window() {
        assert_eq!(detect("usage limit reached"), Plan::DetectedNoTime);
    }

    #[test]
    fn unrelated_lines_are_not_detected() {
        assert_eq!(detect("connection refused"), Plan::NotDetected);
        assert_eq!(detect("build succeeded"), Plan::NotDetected);
    }

    #[test]
    fn parse_duration_after_reads_compound_units() {
        assert_eq!(parse_duration_after("resets in 1d 2h 3m 4s", "resets in"), Some(Duration::from_secs(86400 + 7200 + 180 + 4)));
        assert_eq!(parse_duration_after("nothing here", "resets in"), None);
    }

    #[test]
    fn already_scheduled_round_trips_through_the_state_dir() {
        let dir = env::temp_dir().join(format!("usage-limit-watch-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let log_path = dir.join("session.log");
        assert!(!already_scheduled(&dir, &log_path));
        record_schedule(&dir, &log_path, Duration::from_secs(10), "echo resume").unwrap();
        assert!(already_scheduled(&dir, &log_path));
        fs::remove_dir_all(&dir).unwrap();
    }
}
