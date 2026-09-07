//! privacy-lint: block private network identifiers before they reach public artifacts.
//!
//! Scans text for IP addresses, SSH-target shapes, and machine-local names read from an
//! uncommitted config file, so an issue body, PR body, or commit never carries them.
//!
//!   privacy-lint <file>...            scan named files
//!   privacy-lint --stdin [--name L]   scan raw text on stdin, reported under label L
//!   privacy-lint --diff               scan the added lines of a unified diff on stdin
//!
//! Prints one line per hit — name:line: span [rule] — and exits nonzero on any hit.
//! No auto-fix: an identifier has no single safe rewrite, so this reports and blocks.

use std::io::Read;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::str::FromStr;

#[derive(Debug)]
pub struct Hit {
    pub line_number: usize,
    pub span: String,
    pub rule: &'static str,
}

pub fn run_cli() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let local_names = load_local_names();
    let mut hits = Vec::new();

    if args.iter().any(|a| a == "--diff") {
        let text = read_stdin();
        for (name, line_number, line) in added_diff_lines(&text) {
            for hit in scan_line(line, line_number, &local_names) {
                report(&name, &hit);
                hits.push(hit);
            }
        }
    } else if args.iter().any(|a| a == "--stdin") {
        let name = args
            .iter()
            .position(|a| a == "--name")
            .and_then(|i| args.get(i + 1))
            .cloned()
            .unwrap_or_else(|| "<stdin>".to_string());
        let text = read_stdin();
        hits.extend(scan_text(&name, &text, &local_names));
    } else if args.is_empty() {
        eprintln!("usage: privacy-lint <file>... | --stdin [--name <label>] | --diff");
        std::process::exit(2);
    } else {
        for path in &args {
            match std::fs::read(path) {
                Ok(bytes) => {
                    let text = String::from_utf8_lossy(&bytes);
                    hits.extend(scan_text(path, &text, &local_names));
                }
                Err(err) => {
                    eprintln!("privacy-lint: {path}: {err}");
                    std::process::exit(2);
                }
            }
        }
    }

    std::process::exit(if hits.is_empty() { 0 } else { 1 });
}

fn read_stdin() -> String {
    let mut raw = Vec::new();
    let _ = std::io::stdin().read_to_end(&mut raw);
    String::from_utf8_lossy(&raw).into_owned()
}

fn report(name: &str, hit: &Hit) {
    println!("{name}:{}: {} [{}]", hit.line_number, hit.span, hit.rule);
}

fn scan_text(name: &str, text: &str, local_names: &[String]) -> Vec<Hit> {
    let mut hits = Vec::new();
    for (index, line) in text.lines().enumerate() {
        for hit in scan_line(line, index + 1, local_names) {
            report(name, &hit);
            hits.push(hit);
        }
    }
    hits
}

/// Machine-local hostnames and usernames come from an uncommitted file — one
/// case-insensitive needle per line, `#` comments — so the machine's own names are
/// caught without hardcoding anyone's names in this repo.
fn load_local_names() -> Vec<String> {
    let path = std::env::var("PRIVACY_LINT_IDENTIFIERS").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_default();
        format!("{home}/.config/privacy-lint/identifiers")
    });
    read_local_names(std::path::Path::new(&path)).unwrap_or_default()
}

pub fn read_local_names(path: &std::path::Path) -> std::io::Result<Vec<String>> {
    let body = std::fs::read_to_string(path)?;
    Ok(body
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_lowercase)
        .collect())
}

pub fn scan_line(line: &str, line_number: usize, local_names: &[String]) -> Vec<Hit> {
    let mut hits = Vec::new();
    let mut claimed: Vec<(usize, usize)> = Vec::new();

    for (start, end, candidate) in
        candidates(line, |c| c.is_ascii_hexdigit() || c == ':' || c == '.')
    {
        let trimmed = candidate.trim_matches('.');
        if trimmed.matches(':').count() < 2 {
            continue;
        }
        let Ok(address) = Ipv6Addr::from_str(trimmed) else {
            continue;
        };
        if let Some(rule) = ipv6_rule(address) {
            claimed.push((start, end));
            hits.push(Hit {
                line_number,
                span: trimmed.to_string(),
                rule,
            });
        }
    }

    for (start, end, candidate) in candidates(line, |c| c.is_ascii_digit() || c == '.') {
        if claimed.iter().any(|&(s, e)| start < e && s < end) {
            continue;
        }
        let trimmed = candidate.trim_matches('.');
        let Ok(address) = Ipv4Addr::from_str(trimmed) else {
            continue;
        };
        if let Some(rule) = ipv4_rule(address) {
            hits.push(Hit {
                line_number,
                span: trimmed.to_string(),
                rule,
            });
        }
    }

    for token in line.split_whitespace() {
        let token = token.trim_matches(|c: char| !c.is_ascii_alphanumeric());
        if let Some(span) = ssh_target(token) {
            hits.push(Hit {
                line_number,
                span,
                rule: "ssh target",
            });
        }
    }

    let lowered = line.to_lowercase();
    for name in local_names {
        if lowered.contains(name.as_str()) {
            hits.push(Hit {
                line_number,
                span: name.clone(),
                rule: "machine-local identifier",
            });
        }
    }

    hits
}

fn candidates(line: &str, keep: fn(char) -> bool) -> Vec<(usize, usize, String)> {
    let chars: Vec<char> = line.chars().collect();
    let mut runs = Vec::new();
    let mut start = None;
    for index in 0..=chars.len() {
        let is_kept = index < chars.len() && keep(chars[index]);
        match (start, is_kept) {
            (None, true) => start = Some(index),
            (Some(from), false) => {
                let is_glued_left = from > 0 && chars[from - 1].is_ascii_alphanumeric();
                let is_glued_right = index < chars.len() && chars[index].is_ascii_alphanumeric();
                if !is_glued_left && !is_glued_right {
                    runs.push((from, index, chars[from..index].iter().collect()));
                }
                start = None;
            }
            _ => {}
        }
    }
    runs
}

fn ipv4_rule(address: Ipv4Addr) -> Option<&'static str> {
    let octets = address.octets();
    let is_test_net = matches!(
        octets,
        [192, 0, 2, _] | [198, 51, 100, _] | [203, 0, 113, _]
    );
    let is_example = matches!(octets, [8, 8, 8, 8] | [8, 8, 4, 4] | [1, 1, 1, 1]);
    if address.is_loopback()
        || address.is_unspecified()
        || address.is_broadcast()
        || is_test_net
        || is_example
    {
        return None;
    }
    if octets[0] == 100 && (64..128).contains(&octets[1]) {
        return Some("cgnat/tailnet address");
    }
    if address.is_private() {
        return Some("rfc 1918 address");
    }
    if address.is_link_local() {
        return Some("link-local address");
    }
    Some("ipv4 address")
}

fn ipv6_rule(address: Ipv6Addr) -> Option<&'static str> {
    let segments = address.segments();
    let is_documentation = segments[0] == 0x2001 && segments[1] == 0x0db8;
    if address.is_loopback() || address.is_unspecified() || is_documentation {
        return None;
    }
    Some("ipv6 address")
}

fn ssh_target(token: &str) -> Option<String> {
    let (before, after) = token.split_once('@')?;
    let is_name_char = |c: char| c.is_ascii_alphanumeric() || "._-".contains(c);
    let user = before
        .rfind(|c| !is_name_char(c))
        .map_or(before, |i| &before[i + 1..]);
    let host = after.find(|c: char| !(c.is_ascii_alphanumeric() || ".-".contains(c)));
    let host = host.map_or(after, |i| &after[..i]);
    let is_user = !user.is_empty() && user.chars().any(|c| c.is_ascii_alphabetic());
    let is_host = host.contains('.')
        && host.chars().any(|c| c.is_ascii_alphabetic())
        && !is_reserved_host(host);
    if is_user && is_host {
        Some(format!("{user}@{host}"))
    } else {
        None
    }
}

/// RFC 2606 reserved names and public git-host clone targets identify nobody; both were
/// observed as false positives across this repo's own tests and fixtures on 2026-09-05,
/// which is what licensed the narrowing. Dotless hosts (pkg@latest, plugin@marketplace)
/// were observed too; tailnet-style short hostnames stay covered by the local-names file.
fn is_reserved_host(host: &str) -> bool {
    let lowered = host.to_lowercase();
    [
        ".test",
        ".invalid",
        ".example",
        ".localhost",
        "example.com",
        "example.org",
        "example.net",
        "github.com",
    ]
    .iter()
    .any(|suffix| lowered.ends_with(suffix))
}

/// Yields (file name, new-file line number, content) for each added line of a unified
/// diff, so a pre-commit hook reports real positions in the staged files.
fn added_diff_lines(diff: &str) -> Vec<(String, usize, &str)> {
    let mut added = Vec::new();
    let mut file = String::new();
    let mut next_line = 0usize;
    for line in diff.lines() {
        if let Some(path) = line.strip_prefix("+++ ") {
            file = path.strip_prefix("b/").unwrap_or(path).to_string();
        } else if let Some(header) = line.strip_prefix("@@ ") {
            next_line = header
                .split_whitespace()
                .find_map(|part| part.strip_prefix('+'))
                .and_then(|range| range.split(',').next())
                .and_then(|n| n.parse().ok())
                .unwrap_or(0);
        } else if let Some(content) = line.strip_prefix('+') {
            if file != "/dev/null" {
                added.push((file.clone(), next_line, content));
            }
            next_line += 1;
        } else if !line.starts_with('-') && !line.starts_with('\\') {
            next_line += 1;
        }
    }
    added
}

#[cfg(test)]
mod tests {
    use super::{added_diff_lines, scan_line, ssh_target};
    use std::net::{Ipv4Addr, Ipv6Addr};

    fn rules(line: &str) -> Vec<&'static str> {
        scan_line(line, 1, &[])
            .into_iter()
            .map(|h| h.rule)
            .collect()
    }

    #[test]
    fn cgnat_and_private_ranges_are_flagged() {
        assert_eq!(
            rules(&format!(
                "relay at {} answered",
                Ipv4Addr::new(100, 64, 83, 19)
            )),
            ["cgnat/tailnet address"]
        );
        assert_eq!(
            rules(&Ipv4Addr::new(100, 127, 255, 255).to_string()),
            ["cgnat/tailnet address"]
        );
        assert_eq!(
            rules(&format!("bind to {}", Ipv4Addr::new(10, 83, 27, 19))),
            ["rfc 1918 address"]
        );
        assert_eq!(
            rules(&format!("{}/24 on eth0", Ipv4Addr::new(192, 168, 83, 19))),
            ["rfc 1918 address"]
        );
        assert_eq!(
            rules(&Ipv4Addr::new(172, 16, 83, 19).to_string()),
            ["rfc 1918 address"]
        );
        assert_eq!(
            rules(&Ipv4Addr::new(169, 254, 83, 19).to_string()),
            ["link-local address"]
        );
        assert_eq!(
            rules(&format!("saw {} in logs", Ipv4Addr::new(93, 83, 27, 19))),
            ["ipv4 address"]
        );
    }

    #[test]
    fn non_identifying_ipv4_is_allowed() {
        for line in [
            "127.0.0.1",
            "0.0.0.0",
            "255.255.255.255",
            "192.0.2.44",
            "198.51.100.7",
            "203.0.113.9",
            "8.8.8.8",
            "1.1.1.1",
        ] {
            assert!(rules(line).is_empty(), "{line} flagged");
        }
    }

    #[test]
    fn ipv6_is_flagged_outside_loopback_and_docs() {
        assert_eq!(
            rules(&format!(
                "addr {}",
                Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 83)
            )),
            ["ipv6 address"]
        );
        assert_eq!(
            rules(&Ipv6Addr::new(0xfd83, 0x2719, 0, 0, 0, 0, 0, 83).to_string()),
            ["ipv6 address"]
        );
        assert!(rules("::1").is_empty());
        assert!(rules("2001:db8::8a2e:370:7334").is_empty());
    }

    #[test]
    fn code_and_versions_do_not_read_as_addresses() {
        assert!(rules("use std::io::Read;").is_empty());
        assert!(rules("Vec::<i32>::new()").is_empty());
        assert!(rules("upgrade to v1.2.3.4 today").is_empty());
        assert!(rules("pi 3.14159 and 12:30:45 pm").is_empty());
        assert!(rules("release 1.2.3").is_empty());
    }

    #[test]
    fn ssh_targets_are_flagged() {
        let user = format!("fixture-user-{}", 83);
        let host = format!("fixture-host-{}", 27);
        assert_eq!(rules(&format!("ssh {user}@{host}.lan")), ["ssh target"]);
        assert_eq!(
            rules(&format!("scp x {user}@{host}.local:/tmp")),
            ["ssh target"]
        );
        assert!(ssh_target("pkg@1.2.3").is_none());
        assert!(ssh_target("pkg@latest").is_none());
        assert!(ssh_target("plugin@marketplace-name").is_none());
        assert!(ssh_target("user@example.com").is_none());
        assert!(ssh_target("fixture@example.invalid").is_none());
        assert!(ssh_target("git@github.com").is_none());
        assert!(ssh_target("@mentions").is_none());
        assert!(rules("array[@idx]").is_empty());
    }

    #[test]
    fn local_names_come_from_config() {
        let names = vec!["studio-mini".to_string()];
        let hits = scan_line("logs on Studio-Mini kept", 1, &names);
        assert_eq!(hits[0].rule, "machine-local identifier");
        assert!(scan_line("logs kept", 1, &names).is_empty());
    }

    #[test]
    fn diff_mode_reports_real_positions_in_added_lines_only() {
        let line = format!("relay is {}", Ipv4Addr::new(100, 83, 27, 19));
        let diff =
            format!("--- a/notes.md\n+++ b/notes.md\n@@ -4,0 +5,2 @@\n+{line}\n+fine line\n");
        let added = added_diff_lines(&diff);
        assert_eq!(added[0], ("notes.md".to_string(), 5, line.as_str()));
        assert_eq!(added[1].1, 6);
        let deletion = "--- a/old.md\n+++ /dev/null\n@@ -1,2 +0,0 @@\n-gone\n";
        assert!(added_diff_lines(deletion).is_empty());
    }
}
