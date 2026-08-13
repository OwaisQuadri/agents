use std::path::Path;

use serde_json::{Map, Value};

use crate::drift::{Change, ChangeKind, DriftRow, DriftState, Tool};
use crate::error::SyncError;
use crate::fsio;
use crate::manifest::{
    ManagedServer, Manifest, RemoteSpec, ServerEntry, StdioSpec, SyncState, ToolScope, Transport,
};

/// Renders one manifest entry as a Claude mcpServers JSON value.
/// Takes the entry; returns the JSON object for that server.
pub fn render_entry(entry: &ServerEntry) -> serde_json::Value {
    match &entry.transport {
        Transport::Stdio(spec) => {
            let mut object = Map::new();
            object.insert("command".to_string(), Value::String(spec.command.clone()));
            object.insert(
                "args".to_string(),
                Value::Array(spec.args.iter().cloned().map(Value::String).collect()),
            );
            if !spec.env.is_empty() {
                let env = spec
                    .env
                    .iter()
                    .map(|(key, value)| (key.clone(), Value::String(value.clone())))
                    .collect();
                object.insert("env".to_string(), Value::Object(env));
            }
            Value::Object(object)
        }
        Transport::Remote(spec) => {
            let mut object = Map::new();
            object.insert("type".to_string(), Value::String("http".to_string()));
            object.insert("url".to_string(), Value::String(spec.url.clone()));
            Value::Object(object)
        }
    }
}

// TODO(AGNT-0002.T03): Verify Claude fingerprint deletion-safety coverage.
pub fn managed_server(entry: &ServerEntry) -> ManagedServer {
    ManagedServer {
        name: entry.name.clone(),
        fingerprint: Some(fingerprint_live(&render_entry(entry))),
    }
}

fn fingerprint_live(value: &Value) -> String {
    fn canonicalize(value: &Value) -> Value {
        match value {
            Value::Array(items) => Value::Array(items.iter().map(canonicalize).collect()),
            Value::Object(object) => {
                let mut pairs: Vec<_> = object.iter().collect();
                pairs.sort_unstable_by(|(left, _), (right, _)| left.cmp(right));
                Value::Object(
                    pairs
                        .into_iter()
                        .map(|(key, value)| (key.clone(), canonicalize(value)))
                        .collect(),
                )
            }
            scalar => scalar.clone(),
        }
    }

    let canonical = canonicalize(value);
    let bytes = serde_json::to_vec(&canonical).expect("a serde_json Value always serializes");
    crate::fingerprint::sha256_hex(&bytes)
}

/// Validates that the live Claude config parses, before any write touches it.
/// Takes the config path; returns Ok when the file is absent or holds a
/// valid JSON object.
///
/// # Errors
/// Io when the file is absent (Claude creates its own config); ParseJson on
/// invalid JSON or a non-object top level.
pub fn validate(path: &Path) -> Result<(), SyncError> {
    read_doc(path).map(|_| ())
}

/// Converges the mcpServers key of the Claude config onto the manifest.
/// Touches only manifest-scoped names plus state-listed names to remove;
/// every other key in the file survives byte-for-byte modulo reserialization.
/// Takes the config path, manifest, state, and dry-run flag; returns the
/// changes made or planned.
///
/// # Errors
/// Io, ParseJson, ChangedSinceRead, BackupFailed, VerifyFailed.
pub fn sync(
    path: &Path,
    manifest: &Manifest,
    state: &SyncState,
    is_dry_run: bool,
) -> Result<Vec<Change>, SyncError> {
    let (snapshot, mut doc) = read_doc(path)?;
    let mut servers = live_servers(path, &doc)?;
    let is_key_present = doc.contains_key("mcpServers");
    let mut is_config_changed = false;
    let mut changes = Vec::new();
    for entry in manifest
        .servers
        .iter()
        .filter(|entry| is_claude_scoped(entry))
    {
        let rendered = render_entry(entry);
        let kind = match servers.get(&entry.name) {
            Some(live) if *live == rendered => continue,
            Some(_) => ChangeKind::Update,
            None => ChangeKind::Add,
        };
        servers.insert(entry.name.clone(), rendered);
        is_config_changed = true;
        changes.push(Change {
            tool: Tool::Claude,
            server: entry.name.clone(),
            kind,
        });
    }
    for managed in &state.claude_managed {
        if manifest
            .servers
            .iter()
            .any(|entry| is_claude_scoped(entry) && entry.name == managed.name)
        {
            continue;
        }
        let Some(live) = servers.get(&managed.name) else {
            continue;
        };
        let live_fingerprint = fingerprint_live(live);
        let is_fingerprint_match =
            managed.fingerprint.as_deref() == Some(live_fingerprint.as_str());
        let kind = if is_fingerprint_match {
            servers.shift_remove(&managed.name);
            is_config_changed = true;
            ChangeKind::Remove
        } else {
            ChangeKind::Spare
        };
        changes.push(Change {
            tool: Tool::Claude,
            server: managed.name.clone(),
            kind,
        });
    }
    if !is_config_changed {
        return Ok(changes);
    }
    if is_key_present || !servers.is_empty() {
        doc.insert("mcpServers".to_string(), Value::Object(servers));
    }
    let mut rendered_doc = serde_json::to_string_pretty(&Value::Object(doc))
        .expect("a string-keyed Value always serializes");
    if snapshot.ends_with('\n') {
        rendered_doc.push('\n');
    }
    if rendered_doc == snapshot {
        return Ok(changes);
    }
    fsio::write_verified(path, &rendered_doc, Some(&snapshot), is_dry_run)?;
    Ok(changes)
}

/// Reports per-server drift between the manifest and the Claude config.
/// Takes the config path, manifest, and state; returns one DriftRow per
/// claude-scoped manifest server plus one per unmanaged live server.
///
/// # Errors
/// Io, ParseJson.
pub fn check(
    path: &Path,
    manifest: &Manifest,
    state: &SyncState,
) -> Result<Vec<DriftRow>, SyncError> {
    let (_, doc) = read_doc(path)?;
    let live = live_servers(path, &doc)?;
    let mut rows = Vec::new();
    for entry in manifest
        .servers
        .iter()
        .filter(|entry| is_claude_scoped(entry))
    {
        let drift_state = match live.get(&entry.name) {
            None => DriftState::Missing,
            Some(value) if *value == render_entry(entry) => DriftState::Ok,
            Some(_) => DriftState::Drifted,
        };
        rows.push(DriftRow {
            server: entry.name.clone(),
            tool: Tool::Claude,
            state: drift_state,
        });
    }
    for (name, value) in &live {
        if manifest
            .servers
            .iter()
            .any(|entry| is_claude_scoped(entry) && entry.name == *name)
        {
            continue;
        }
        let drift_state = match state
            .claude_managed
            .iter()
            .find(|managed| managed.name == *name)
        {
            Some(managed)
                if managed.fingerprint.as_deref() == Some(fingerprint_live(value).as_str()) =>
            {
                DriftState::Drifted
            }
            Some(_) => DriftState::Spared,
            None => DriftState::Unmanaged,
        };
        rows.push(DriftRow {
            server: name.clone(),
            tool: Tool::Claude,
            state: drift_state,
        });
    }
    Ok(rows)
}

/// Parses live Claude servers absent from the manifest into manifest entries.
/// Takes the config path and manifest; returns the adoptable entries.
///
/// # Errors
/// Io, ParseJson.
pub fn unmanaged(path: &Path, manifest: &Manifest) -> Result<Vec<ServerEntry>, SyncError> {
    let (_, doc) = read_doc(path)?;
    let live = live_servers(path, &doc)?;
    let mut entries = Vec::new();
    for (name, value) in &live {
        if manifest.servers.iter().any(|entry| entry.name == *name) {
            continue;
        }
        match translate(name, value) {
            Ok(entry) => entries.push(entry),
            Err(reason) => eprintln!("skip: claude server {name}: {reason}"),
        }
    }
    Ok(entries)
}

fn is_claude_scoped(entry: &ServerEntry) -> bool {
    matches!(entry.scope, ToolScope::Both | ToolScope::ClaudeOnly)
        && entry.is_for_this_platform()
}

fn read_doc(path: &Path) -> Result<(String, Map<String, Value>), SyncError> {
    let Some(text) = fsio::read_opt(path)? else {
        return Err(SyncError::Io(
            path.to_path_buf(),
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "file does not exist; Claude creates its own config",
            ),
        ));
    };
    let value: Value = serde_json::from_str(&text)
        .map_err(|err| SyncError::ParseJson(path.to_path_buf(), err.to_string()))?;
    match value {
        Value::Object(doc) => Ok((text, doc)),
        _ => Err(SyncError::ParseJson(
            path.to_path_buf(),
            "top-level value is not an object".to_string(),
        )),
    }
}

fn live_servers(path: &Path, doc: &Map<String, Value>) -> Result<Map<String, Value>, SyncError> {
    match doc.get("mcpServers") {
        Some(Value::Object(map)) => Ok(map.clone()),
        Some(_) => Err(SyncError::ParseJson(
            path.to_path_buf(),
            "mcpServers is not an object".to_string(),
        )),
        None => Ok(Map::new()),
    }
}

fn translate(name: &str, value: &Value) -> Result<ServerEntry, String> {
    let Value::Object(object) = value else {
        return Err("entry is not an object".to_string());
    };
    if object.contains_key("headers") {
        return Err("headers have no manifest shape".to_string());
    }
    if object.contains_key("oauth") {
        return Err("oauth has no manifest shape".to_string());
    }
    if object.contains_key("command") {
        return translate_stdio(name, object);
    }
    match object.get("type").and_then(Value::as_str) {
        Some("http") | Some("streamable-http") => {
            let Some(url) = object.get("url").and_then(Value::as_str) else {
                return Err("remote entry has no url string".to_string());
            };
            Ok(ServerEntry {
                name: name.to_string(),
                transport: Transport::Remote(RemoteSpec {
                    url: url.to_string(),
                    bearer_token_env_var: None,
                }),
                scope: ToolScope::Both,
                platforms: Vec::new(),
            })
        }
        Some(other) => Err(format!("transport {other} has no manifest shape")),
        None => Err("entry has neither command nor a known type".to_string()),
    }
}

fn translate_stdio(name: &str, object: &Map<String, Value>) -> Result<ServerEntry, String> {
    let Some(command) = object.get("command").and_then(Value::as_str) else {
        return Err("command is not a string".to_string());
    };
    let args = match object.get("args") {
        None => Vec::new(),
        Some(Value::Array(items)) => {
            let mut args = Vec::new();
            for item in items {
                let Some(arg) = item.as_str() else {
                    return Err("args holds a non-string".to_string());
                };
                args.push(arg.to_string());
            }
            args
        }
        Some(_) => return Err("args is not an array".to_string()),
    };
    let env = match object.get("env") {
        None => Vec::new(),
        Some(Value::Object(pairs)) => {
            let mut env = Vec::new();
            for (key, item) in pairs {
                let Some(text) = item.as_str() else {
                    return Err("env holds a non-string".to_string());
                };
                env.push((key.clone(), text.to_string()));
            }
            env
        }
        Some(_) => return Err("env is not an object".to_string()),
    };
    Ok(ServerEntry {
        name: name.to_string(),
        transport: Transport::Stdio(StdioSpec {
            command: command.to_string(),
            args,
            env,
            cwd: None,
        }),
        scope: ToolScope::Both,
        platforms: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use serde_json::json;

    use super::*;

    fn fixture_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("mcp-sync-claude-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create fixture dir");
        dir
    }

    fn pretty(value: &Value) -> String {
        let mut text = serde_json::to_string_pretty(value).expect("serialize fixture");
        text.push('\n');
        text
    }

    fn stdio_entry(
        name: &str,
        scope: ToolScope,
        command: &str,
        args: &[&str],
        env: &[(&str, &str)],
    ) -> ServerEntry {
        ServerEntry {
            name: name.to_string(),
            transport: Transport::Stdio(StdioSpec {
                command: command.to_string(),
                args: args.iter().map(|arg| arg.to_string()).collect(),
                env: env
                    .iter()
                    .map(|(key, value)| (key.to_string(), value.to_string()))
                    .collect(),
                cwd: None,
            }),
            scope,
            platforms: Vec::new(),
        }
    }

    fn remote_entry(
        name: &str,
        scope: ToolScope,
        url: &str,
        token_var: Option<&str>,
    ) -> ServerEntry {
        ServerEntry {
            name: name.to_string(),
            transport: Transport::Remote(RemoteSpec {
                url: url.to_string(),
                bearer_token_env_var: token_var.map(str::to_string),
            }),
            scope,
            platforms: Vec::new(),
        }
    }

    fn empty_state() -> SyncState {
        SyncState {
            claude_managed: Vec::new(),
            codex_managed: Vec::new(),
        }
    }

    fn managed(name: &str, fingerprint: Option<String>) -> ManagedServer {
        ManagedServer {
            name: name.to_string(),
            fingerprint,
        }
    }

    fn managed_live(name: &str, value: &Value) -> ManagedServer {
        managed(name, Some(fingerprint_live(value)))
    }

    fn seeded_config() -> Value {
        json!({
            "installMethod": "brew",
            "projects": {
                "/Users/owais/repo": {
                    "allowedTools": ["Bash", "Read"],
                    "history": ["run tests", "fix the parser"]
                }
            },
            "mcpServers": {
                "drifty": { "command": "old-cmd", "args": [] },
                "stale": { "command": "stale-cmd", "args": ["--x"] },
                "foreign": { "type": "http", "url": "https://foreign.example/mcp" }
            },
            "tipsHistory": { "memory-command": 5 }
        })
    }

    #[test]
    fn render_stdio_without_env_omits_the_env_key() {
        let rendered = render_entry(&stdio_entry("s", ToolScope::Both, "cmd", &["--flag"], &[]));
        assert_eq!(rendered, json!({ "command": "cmd", "args": ["--flag"] }));
    }

    #[test]
    fn render_stdio_with_env_carries_the_env_object() {
        let rendered = render_entry(&stdio_entry(
            "s",
            ToolScope::Both,
            "cmd",
            &[],
            &[("MODE", "fast")],
        ));
        assert_eq!(
            rendered,
            json!({ "command": "cmd", "args": [], "env": { "MODE": "fast" } })
        );
    }

    #[test]
    fn render_remote_is_http_url_and_never_a_token() {
        let rendered = render_entry(&remote_entry(
            "r",
            ToolScope::Both,
            "https://x.example/mcp",
            Some("X_TOKEN"),
        ));
        assert_eq!(
            rendered,
            json!({ "type": "http", "url": "https://x.example/mcp" })
        );
    }

    #[test]
    fn managed_record_fingerprints_the_rendered_value_canonically() {
        let entry = stdio_entry(
            "tool",
            ToolScope::Both,
            "npx",
            &["-y", "tool-mcp"],
            &[("MODE", "fast")],
        );
        let managed = managed_server(&entry);
        let equal_live = json!({
            "env": { "MODE": "fast" },
            "args": ["-y", "tool-mcp"],
            "command": "npx"
        });
        assert_eq!(managed.name, "tool");
        assert_eq!(
            managed.fingerprint.as_deref(),
            Some(fingerprint_live(&equal_live).as_str())
        );
    }

    #[test]
    fn sync_on_an_absent_file_is_an_io_error() {
        let dir = fixture_dir("absent");
        let missing = dir.join("claude.json");
        let manifest = Manifest {
            servers: Vec::new(),
        };
        let result = sync(&missing, &manifest, &empty_state(), false);
        assert!(matches!(result, Err(SyncError::Io(_, _))));
    }

    #[test]
    fn sync_converges_and_preserves_foreign_keys_byte_identically() {
        let dir = fixture_dir("sync");
        let path = dir.join("claude.json");
        fs::write(&path, pretty(&seeded_config())).expect("seed config");
        let manifest = Manifest {
            servers: vec![
                stdio_entry(
                    "drifty",
                    ToolScope::Both,
                    "new-cmd",
                    &["--serve"],
                    &[("MODE", "fast")],
                ),
                remote_entry(
                    "fresh",
                    ToolScope::ClaudeOnly,
                    "https://fresh.example/mcp",
                    Some("FRESH_TOKEN"),
                ),
                stdio_entry(
                    "codex-side",
                    ToolScope::CodexOnly,
                    "codex-only-cmd",
                    &[],
                    &[],
                ),
            ],
        };
        let state = SyncState {
            claude_managed: vec![
                managed("drifty", None),
                managed_live("stale", &json!({ "command": "stale-cmd", "args": ["--x"] })),
            ],
            codex_managed: Vec::new(),
        };
        let changes = sync(&path, &manifest, &state, false).expect("sync succeeds");
        assert_eq!(changes.len(), 3);
        assert!(matches!(
            &changes[0],
            Change { tool: Tool::Claude, kind: ChangeKind::Update, server } if server == "drifty"
        ));
        assert!(matches!(
            &changes[1],
            Change { tool: Tool::Claude, kind: ChangeKind::Add, server } if server == "fresh"
        ));
        assert!(matches!(
            &changes[2],
            Change { tool: Tool::Claude, kind: ChangeKind::Remove, server } if server == "stale"
        ));
        let expected = pretty(&json!({
            "installMethod": "brew",
            "projects": {
                "/Users/owais/repo": {
                    "allowedTools": ["Bash", "Read"],
                    "history": ["run tests", "fix the parser"]
                }
            },
            "mcpServers": {
                "drifty": { "command": "new-cmd", "args": ["--serve"], "env": { "MODE": "fast" } },
                "foreign": { "type": "http", "url": "https://foreign.example/mcp" },
                "fresh": { "type": "http", "url": "https://fresh.example/mcp" }
            },
            "tipsHistory": { "memory-command": 5 }
        }));
        assert_eq!(fs::read_to_string(&path).unwrap(), expected);
    }

    #[test]
    fn sync_removes_a_stale_server_only_on_an_exact_fingerprint_match() {
        let dir = fixture_dir("exact-fingerprint-removal");
        let path = dir.join("claude.json");
        let stale = json!({ "command": "stale-cmd", "args": ["--x"] });
        fs::write(
            &path,
            pretty(&json!({
                "unrelated": { "keep": true },
                "mcpServers": { "stale": stale.clone() }
            })),
        )
        .expect("seed config");
        let state = SyncState {
            claude_managed: vec![managed_live("stale", &stale)],
            codex_managed: Vec::new(),
        };

        let changes = sync(
            &path,
            &Manifest {
                servers: Vec::new(),
            },
            &state,
            false,
        )
        .expect("sync succeeds");

        assert!(matches!(
            changes.as_slice(),
            [Change { tool: Tool::Claude, server, kind: ChangeKind::Remove }] if server == "stale"
        ));
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            pretty(&json!({ "unrelated": { "keep": true }, "mcpServers": {} }))
        );
    }

    #[test]
    fn sync_spares_a_changed_replacement_without_rewriting_the_file() {
        let dir = fixture_dir("changed-replacement");
        let path = dir.join("claude.json");
        let original = json!({ "command": "old-cmd", "args": [] });
        let bytes = "{\"unrelated\":17,\"mcpServers\":{\"stale\":{\"command\":\"replacement\",\"args\":[]}}}";
        fs::write(&path, bytes).expect("seed config");
        let state = SyncState {
            claude_managed: vec![managed_live("stale", &original)],
            codex_managed: Vec::new(),
        };

        let changes = sync(
            &path,
            &Manifest {
                servers: Vec::new(),
            },
            &state,
            false,
        )
        .expect("sync succeeds");

        assert!(matches!(
            changes.as_slice(),
            [Change { tool: Tool::Claude, server, kind: ChangeKind::Spare }] if server == "stale"
        ));
        assert_eq!(fs::read_to_string(&path).unwrap(), bytes);
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
    }

    #[test]
    fn sync_spares_a_legacy_record_without_deletion_authority() {
        let dir = fixture_dir("legacy-record");
        let path = dir.join("claude.json");
        let bytes = pretty(&json!({
            "mcpServers": { "stale": { "command": "stale-cmd", "args": [] } }
        }));
        fs::write(&path, &bytes).expect("seed config");
        let state = SyncState {
            claude_managed: vec![managed("stale", None)],
            codex_managed: Vec::new(),
        };

        let changes = sync(
            &path,
            &Manifest {
                servers: Vec::new(),
            },
            &state,
            false,
        )
        .expect("sync succeeds");

        assert!(matches!(
            changes.as_slice(),
            [Change { tool: Tool::Claude, server, kind: ChangeKind::Spare }] if server == "stale"
        ));
        assert_eq!(fs::read_to_string(&path).unwrap(), bytes);
    }

    #[test]
    fn sync_creates_the_mcp_servers_key_when_absent() {
        let dir = fixture_dir("createkey");
        let path = dir.join("claude.json");
        fs::write(&path, pretty(&json!({ "installMethod": "brew" }))).expect("seed config");
        let manifest = Manifest {
            servers: vec![remote_entry(
                "fresh",
                ToolScope::Both,
                "https://fresh.example/mcp",
                None,
            )],
        };
        let changes = sync(&path, &manifest, &empty_state(), false).expect("sync succeeds");
        assert_eq!(changes.len(), 1);
        let expected = pretty(&json!({
            "installMethod": "brew",
            "mcpServers": {
                "fresh": { "type": "http", "url": "https://fresh.example/mcp" }
            }
        }));
        assert_eq!(fs::read_to_string(&path).unwrap(), expected);
    }

    #[test]
    fn sync_round_trips_float_literals_byte_exact() {
        let dir = fixture_dir("floats");
        let path = dir.join("claude.json");
        let seeded = "{\n  \"growthMetrics\": {\n    \"ratio\": 0.16220400000000001,\n    \"ceiling\": 1e+308,\n    \"tiny\": 5e-324\n  },\n  \"mcpServers\": {}\n}\n";
        fs::write(&path, seeded).expect("seed config");
        let manifest = Manifest {
            servers: vec![remote_entry(
                "fresh",
                ToolScope::Both,
                "https://fresh.example/mcp",
                None,
            )],
        };
        let changes = sync(&path, &manifest, &empty_state(), false).expect("sync succeeds");
        assert_eq!(changes.len(), 1);
        let expected = "{\n  \"growthMetrics\": {\n    \"ratio\": 0.16220400000000001,\n    \"ceiling\": 1e+308,\n    \"tiny\": 5e-324\n  },\n  \"mcpServers\": {\n    \"fresh\": {\n      \"type\": \"http\",\n      \"url\": \"https://fresh.example/mcp\"\n    }\n  }\n}\n";
        assert_eq!(fs::read_to_string(&path).unwrap(), expected);
    }

    #[test]
    fn second_sync_returns_no_changes_and_writes_nothing() {
        let dir = fixture_dir("idempotent");
        let path = dir.join("claude.json");
        fs::write(&path, pretty(&seeded_config())).expect("seed config");
        let manifest = Manifest {
            servers: vec![stdio_entry("drifty", ToolScope::Both, "new-cmd", &[], &[])],
        };
        let state = empty_state();
        let first = sync(&path, &manifest, &state, false).expect("first sync");
        assert_eq!(first.len(), 1);
        let bytes = fs::read_to_string(&path).unwrap();
        let files_before = fs::read_dir(&dir).unwrap().count();
        let second = sync(&path, &manifest, &state, false).expect("second sync");
        assert!(second.is_empty());
        assert_eq!(fs::read_to_string(&path).unwrap(), bytes);
        assert_eq!(fs::read_dir(&dir).unwrap().count(), files_before);
    }

    #[test]
    fn a_file_without_a_trailing_newline_converges() {
        let dir = fixture_dir("no-trailing-newline");
        let path = dir.join("claude.json");
        let seeded = pretty(&seeded_config());
        fs::write(&path, seeded.trim_end_matches('\n')).expect("seed config");
        let manifest = Manifest {
            servers: vec![stdio_entry("drifty", ToolScope::Both, "new-cmd", &[], &[])],
        };
        let state = empty_state();
        sync(&path, &manifest, &state, false).expect("first sync");
        let bytes = fs::read_to_string(&path).unwrap();
        assert!(!bytes.ends_with('\n'));
        let files_before = fs::read_dir(&dir).unwrap().count();
        let second = sync(&path, &manifest, &state, false).expect("second sync");
        assert!(second.is_empty());
        assert_eq!(fs::read_to_string(&path).unwrap(), bytes);
        assert_eq!(fs::read_dir(&dir).unwrap().count(), files_before);
    }

    #[test]
    fn dry_run_sync_reports_changes_and_touches_nothing() {
        let dir = fixture_dir("dry");
        let path = dir.join("claude.json");
        fs::write(&path, pretty(&seeded_config())).expect("seed config");
        let manifest = Manifest {
            servers: vec![stdio_entry("drifty", ToolScope::Both, "new-cmd", &[], &[])],
        };
        let state = SyncState {
            claude_managed: vec![managed_live(
                "stale",
                &json!({ "command": "stale-cmd", "args": ["--x"] }),
            )],
            codex_managed: Vec::new(),
        };
        let before = fs::read_to_string(&path).unwrap();
        let changes = sync(&path, &manifest, &state, true).expect("dry-run sync");
        assert_eq!(changes.len(), 2);
        assert_eq!(fs::read_to_string(&path).unwrap(), before);
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
    }

    #[test]
    fn check_rows_cover_ok_missing_drifted_pending_remove_and_unmanaged() {
        let dir = fixture_dir("check");
        let path = dir.join("claude.json");
        let live = json!({
            "mcpServers": {
                "steady": { "command": "cmd", "args": [] },
                "drifty": { "command": "old-cmd", "args": [] },
                "stale": { "command": "stale-cmd", "args": [] },
                "foreign": { "type": "http", "url": "https://foreign.example/mcp" }
            }
        });
        fs::write(&path, pretty(&live)).expect("seed config");
        let manifest = Manifest {
            servers: vec![
                stdio_entry("steady", ToolScope::Both, "cmd", &[], &[]),
                stdio_entry("drifty", ToolScope::ClaudeOnly, "new-cmd", &[], &[]),
                stdio_entry("gone", ToolScope::Both, "cmd", &[], &[]),
                stdio_entry("codex-side", ToolScope::CodexOnly, "cmd", &[], &[]),
            ],
        };
        let state = SyncState {
            claude_managed: vec![managed_live(
                "stale",
                &json!({ "command": "stale-cmd", "args": [] }),
            )],
            codex_managed: Vec::new(),
        };
        let rows = check(&path, &manifest, &state).expect("check succeeds");
        assert_eq!(rows.len(), 5);
        assert!(matches!(
            &rows[0],
            DriftRow { tool: Tool::Claude, state: DriftState::Ok, server } if server == "steady"
        ));
        assert!(matches!(
            &rows[1],
            DriftRow { tool: Tool::Claude, state: DriftState::Drifted, server } if server == "drifty"
        ));
        assert!(matches!(
            &rows[2],
            DriftRow { tool: Tool::Claude, state: DriftState::Missing, server } if server == "gone"
        ));
        assert!(matches!(
            &rows[3],
            DriftRow { tool: Tool::Claude, state: DriftState::Drifted, server } if server == "stale"
        ));
        assert!(matches!(
            &rows[4],
            DriftRow { tool: Tool::Claude, state: DriftState::Unmanaged, server } if server == "foreign"
        ));
    }

    #[test]
    fn check_classifies_changed_and_legacy_managed_servers_as_spared() {
        let dir = fixture_dir("check-spared");
        let path = dir.join("claude.json");
        let original = json!({ "command": "original", "args": [] });
        fs::write(
            &path,
            pretty(&json!({
                "mcpServers": {
                    "changed": { "command": "replacement", "args": [] },
                    "legacy": { "command": "legacy", "args": [] }
                }
            })),
        )
        .expect("seed config");
        let state = SyncState {
            claude_managed: vec![managed_live("changed", &original), managed("legacy", None)],
            codex_managed: Vec::new(),
        };

        let rows = check(
            &path,
            &Manifest {
                servers: Vec::new(),
            },
            &state,
        )
        .expect("check succeeds");

        assert_eq!(rows.len(), 2);
        assert!(rows
            .iter()
            .all(|row| matches!(row.state, DriftState::Spared)));
    }

    #[test]
    fn unmanaged_translates_expressible_entries_and_skips_the_rest() {
        let dir = fixture_dir("unmanaged");
        let path = dir.join("claude.json");
        let live = json!({
            "mcpServers": {
                "known": { "command": "cmd", "args": [] },
                "tool": { "command": "npx", "args": ["-y", "tool-mcp"], "env": { "MODE": "fast" } },
                "site": { "type": "http", "url": "https://site.example/mcp" },
                "stream": { "type": "streamable-http", "url": "https://stream.example/mcp" },
                "pushy": { "type": "sse", "url": "https://pushy.example/mcp" },
                "sockety": { "type": "ws", "url": "wss://sockety.example/mcp" },
                "headed": { "type": "http", "url": "https://headed.example/mcp", "headers": { "X-Key": "v" } }
            }
        });
        fs::write(&path, pretty(&live)).expect("seed config");
        let manifest = Manifest {
            servers: vec![stdio_entry("known", ToolScope::CodexOnly, "cmd", &[], &[])],
        };
        let entries = unmanaged(&path, &manifest).expect("unmanaged succeeds");
        let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, ["tool", "site", "stream"]);
        assert!(matches!(
            &entries[0].transport,
            Transport::Stdio(spec)
                if spec.command == "npx"
                    && spec.args == ["-y", "tool-mcp"]
                    && spec.env == [("MODE".to_string(), "fast".to_string())]
        ));
        assert!(matches!(
            &entries[1].transport,
            Transport::Remote(spec)
                if spec.url == "https://site.example/mcp" && spec.bearer_token_env_var.is_none()
        ));
        assert!(matches!(
            &entries[2].transport,
            Transport::Remote(spec) if spec.url == "https://stream.example/mcp"
        ));
    }
}
