// TODO(AGNT-0002.T04): Verify Codex spare and removal safety.
use std::path::Path;

use toml_edit::{DocumentMut, Item, Table, TableLike, Value};

use crate::drift::{Change, ChangeKind, DriftRow, DriftState, Tool};
use crate::error::SyncError;
use crate::fsio;
use crate::manifest::{
    ManagedServer, Manifest, RemoteSpec, ServerEntry, StdioSpec, SyncState, ToolScope, Transport,
};

/// Renders one manifest entry as a Codex [mcp_servers.<name>] table.
/// Takes the entry; returns the TOML table.
pub fn render_table(entry: &ServerEntry) -> toml_edit::Table {
    let mut table = Table::new();
    match &entry.transport {
        Transport::Stdio(spec) => {
            table.insert("command", toml_edit::value(spec.command.as_str()));
            let mut args = toml_edit::Array::new();
            for arg in &spec.args {
                args.push(arg.as_str());
            }
            table.insert("args", toml_edit::value(args));
            if !spec.env.is_empty() {
                let mut env = toml_edit::InlineTable::new();
                for (key, value) in &spec.env {
                    env.insert(key.as_str(), value.as_str().into());
                }
                table.insert("env", toml_edit::value(env));
            }
            if let Some(cwd) = &spec.cwd {
                table.insert("cwd", toml_edit::value(cwd.as_str()));
            }
        }
        Transport::Remote(spec) => {
            table.insert("url", toml_edit::value(spec.url.as_str()));
            if let Some(name) = &spec.bearer_token_env_var {
                table.insert("bearer_token_env_var", toml_edit::value(name.as_str()));
            }
        }
    }
    table
}

pub fn managed_server(entry: &ServerEntry) -> ManagedServer {
    ManagedServer {
        name: entry.name.clone(),
        fingerprint: Some(fingerprint_live(&render_table(entry))),
    }
}

fn fingerprint_live(table: &dyn TableLike) -> String {
    fn push_len(bytes: &mut Vec<u8>, len: usize) {
        bytes.extend_from_slice(&(len as u64).to_be_bytes());
    }

    fn push_bytes(bytes: &mut Vec<u8>, value: &[u8]) {
        push_len(bytes, value.len());
        bytes.extend_from_slice(value);
    }

    fn canonicalize_table(table: &dyn TableLike, bytes: &mut Vec<u8>) {
        bytes.push(b't');
        let mut pairs: Vec<_> = table.iter().collect();
        pairs.sort_unstable_by_key(|(left, _)| *left);
        push_len(bytes, pairs.len());
        for (key, item) in pairs {
            push_bytes(bytes, key.as_bytes());
            canonicalize_item(item, bytes);
        }
    }

    fn canonicalize_value(value: &Value, bytes: &mut Vec<u8>) {
        match value {
            Value::String(value) => {
                bytes.push(b's');
                push_bytes(bytes, value.value().as_bytes());
            }
            Value::Integer(value) => {
                bytes.push(b'i');
                bytes.extend_from_slice(&value.value().to_be_bytes());
            }
            Value::Float(value) => {
                bytes.push(b'f');
                bytes.extend_from_slice(&value.value().to_bits().to_be_bytes());
            }
            Value::Boolean(value) => {
                bytes.push(b'b');
                bytes.push(u8::from(*value.value()));
            }
            Value::Datetime(value) => {
                bytes.push(b'd');
                push_bytes(bytes, value.value().to_string().as_bytes());
            }
            Value::Array(values) => {
                bytes.push(b'a');
                push_len(bytes, values.len());
                for value in values.iter() {
                    canonicalize_value(value, bytes);
                }
            }
            Value::InlineTable(table) => canonicalize_table(table, bytes),
        }
    }

    fn canonicalize_item(item: &Item, bytes: &mut Vec<u8>) {
        match item {
            Item::None => bytes.push(b'n'),
            Item::Value(value) => canonicalize_value(value, bytes),
            Item::Table(table) => canonicalize_table(table, bytes),
            Item::ArrayOfTables(tables) => {
                bytes.push(b'a');
                push_len(bytes, tables.len());
                for table in tables.iter() {
                    canonicalize_table(table, bytes);
                }
            }
        }
    }

    let mut canonical = Vec::new();
    canonicalize_table(table, &mut canonical);
    crate::fingerprint::sha256_hex(&canonical)
}

/// Validates that a live Codex config parses, before any write touches it.
/// Takes the config path; returns Ok when the file is absent or holds valid
/// TOML.
///
/// # Errors
/// ParseToml on invalid TOML; Io on a read failure other than not-found.
pub fn validate(path: &Path) -> Result<(), SyncError> {
    let text = fsio::read_opt(path)?;
    let doc = parse_doc(path, text.as_deref())?;
    match doc.get("mcp_servers") {
        None | Some(toml_edit::Item::Table(_)) => Ok(()),
        Some(_) => Err(SyncError::ParseToml(
            path.to_path_buf(),
            "mcp_servers is not a table".to_string(),
        )),
    }
}

/// Converges the [mcp_servers.*] tables of config.toml onto the manifest.
/// Touches only manifest-scoped names plus state-listed names to remove;
/// [projects.*], [plugins.*], comments, and unmanaged server tables survive
/// via toml_edit decor preservation.
/// Takes the config path, manifest, state, and dry-run flag; returns the
/// changes made or planned.
///
/// # Errors
/// Io, ParseToml, ChangedSinceRead, BackupFailed, VerifyFailed.
pub fn sync(
    path: &Path,
    manifest: &Manifest,
    state: &SyncState,
    is_dry_run: bool,
) -> Result<Vec<Change>, SyncError> {
    let snapshot = fsio::read_opt(path)?;
    let mut doc = parse_doc(path, snapshot.as_deref())?;
    let mut is_config_changed = false;
    let mut changes = Vec::new();
    for entry in manifest
        .servers
        .iter()
        .filter(|entry| is_codex_scoped(entry))
    {
        let kind = match live_server(&doc, &entry.name) {
            None => ChangeKind::Add,
            Some(live) if is_matching(live, entry) => continue,
            Some(_) => ChangeKind::Update,
        };
        let mut table = render_table(entry);
        let parent = parent_table_mut(&mut doc, path)?;
        if let Some(old) = parent.get(&entry.name).and_then(Item::as_table) {
            if let Some(position) = old.position() {
                table.set_position(Some(position));
            }
            if let Some(prefix) = old.decor().prefix() {
                table.decor_mut().set_prefix(prefix.clone());
            }
        }
        parent.insert(&entry.name, Item::Table(table));
        is_config_changed = true;
        changes.push(Change {
            tool: Tool::Codex,
            server: entry.name.clone(),
            kind,
        });
    }
    for managed in &state.codex_managed {
        if manifest
            .servers
            .iter()
            .any(|entry| is_codex_scoped(entry) && entry.name == managed.name)
        {
            continue;
        }
        let Some(live) = doc
            .get("mcp_servers")
            .and_then(Item::as_table_like)
            .and_then(|servers| servers.get(&managed.name))
        else {
            continue;
        };
        let is_fingerprint_match = live.as_table_like().is_some_and(|table| {
            managed.fingerprint.as_deref() == Some(fingerprint_live(table).as_str())
        });
        if !is_fingerprint_match {
            changes.push(Change {
                tool: Tool::Codex,
                server: managed.name.clone(),
                kind: ChangeKind::Spare,
            });
            continue;
        }
        let Some(parent) = doc.get_mut("mcp_servers").and_then(Item::as_table_mut) else {
            break;
        };
        if parent.remove(&managed.name).is_some() {
            is_config_changed = true;
            changes.push(Change {
                tool: Tool::Codex,
                server: managed.name.clone(),
                kind: ChangeKind::Remove,
            });
        }
    }
    if !is_config_changed {
        return Ok(changes);
    }
    let rendered = doc.to_string();
    let is_converged = snapshot.as_deref() == Some(rendered.as_str())
        || (snapshot.is_none() && rendered.is_empty());
    if !is_converged {
        fsio::write_verified(path, &rendered, snapshot.as_deref(), is_dry_run)?;
    }
    Ok(changes)
}

/// Reports per-server drift between the manifest and config.toml.
/// Takes the config path, manifest, and state; returns one DriftRow per
/// codex-scoped manifest server plus one per live server outside them —
/// Drifted when state lists it as a pending removal, Unmanaged otherwise.
///
/// # Errors
/// Io, ParseToml.
pub fn check(
    path: &Path,
    manifest: &Manifest,
    state: &SyncState,
) -> Result<Vec<DriftRow>, SyncError> {
    let text = fsio::read_opt(path)?;
    let doc = parse_doc(path, text.as_deref())?;
    let servers = doc.get("mcp_servers").and_then(Item::as_table_like);
    let mut rows = Vec::new();
    for entry in manifest
        .servers
        .iter()
        .filter(|entry| is_codex_scoped(entry))
    {
        let live = servers
            .and_then(|table| table.get(&entry.name))
            .and_then(Item::as_table_like);
        let drift = match live {
            None => DriftState::Missing,
            Some(table) if is_matching(table, entry) => DriftState::Ok,
            Some(_) => DriftState::Drifted,
        };
        rows.push(DriftRow {
            server: entry.name.clone(),
            tool: Tool::Codex,
            state: drift,
        });
    }
    if let Some(servers) = servers {
        for (name, item) in servers.iter() {
            if manifest
                .servers
                .iter()
                .any(|entry| is_codex_scoped(entry) && entry.name == name)
            {
                continue;
            }
            let drift = match state
                .codex_managed
                .iter()
                .find(|managed| managed.name == name)
            {
                Some(managed)
                    if item.as_table_like().is_some_and(|table| {
                        managed.fingerprint.as_deref() == Some(fingerprint_live(table).as_str())
                    }) =>
                {
                    DriftState::Drifted
                }
                Some(_) => DriftState::Spared,
                None => DriftState::Unmanaged,
            };
            rows.push(DriftRow {
                server: name.to_string(),
                tool: Tool::Codex,
                state: drift,
            });
        }
    }
    Ok(rows)
}

/// Parses live Codex servers absent from the manifest into manifest entries.
/// Takes the config path and manifest; returns the adoptable entries.
///
/// # Errors
/// Io, ParseToml.
pub fn unmanaged(path: &Path, manifest: &Manifest) -> Result<Vec<ServerEntry>, SyncError> {
    let text = fsio::read_opt(path)?;
    let doc = parse_doc(path, text.as_deref())?;
    let Some(servers) = doc.get("mcp_servers").and_then(Item::as_table_like) else {
        return Ok(Vec::new());
    };
    let mut entries = Vec::new();
    for (name, item) in servers.iter() {
        if manifest.servers.iter().any(|entry| entry.name == name) {
            continue;
        }
        let Some(live) = item.as_table_like() else {
            eprintln!("skip codex server {name}: entry is not a table");
            continue;
        };
        match adopt(name, live) {
            Ok(entry) => entries.push(entry),
            Err(reason) => eprintln!("skip codex server {name}: {reason}"),
        }
    }
    Ok(entries)
}

fn parse_doc(path: &Path, text: Option<&str>) -> Result<DocumentMut, SyncError> {
    let Some(text) = text else {
        return Ok(DocumentMut::new());
    };
    text.parse::<DocumentMut>()
        .map_err(|err| SyncError::ParseToml(path.to_path_buf(), err.to_string()))
}

fn is_codex_scoped(entry: &ServerEntry) -> bool {
    matches!(entry.scope, ToolScope::Both | ToolScope::CodexOnly)
        && entry.is_for_this_platform()
}

fn live_server<'a>(doc: &'a DocumentMut, name: &str) -> Option<&'a dyn TableLike> {
    doc.get("mcp_servers")?
        .as_table_like()?
        .get(name)?
        .as_table_like()
}

fn parent_table_mut<'a>(doc: &'a mut DocumentMut, path: &Path) -> Result<&'a mut Table, SyncError> {
    if !doc.contains_key("mcp_servers") {
        let mut parent = Table::new();
        parent.set_implicit(true);
        doc.insert("mcp_servers", Item::Table(parent));
    }
    doc.get_mut("mcp_servers")
        .and_then(Item::as_table_mut)
        .ok_or_else(|| {
            SyncError::ParseToml(
                path.to_path_buf(),
                String::from("mcp_servers is not a table"),
            )
        })
}

fn is_matching(live: &dyn TableLike, entry: &ServerEntry) -> bool {
    match &entry.transport {
        Transport::Stdio(spec) => {
            live.get("command").and_then(Item::as_str) == Some(spec.command.as_str())
                && str_values(live, "args").as_deref() == Some(spec.args.as_slice())
                && is_env_matching(live, &spec.env)
                && live.get("cwd").and_then(Item::as_str) == spec.cwd.as_deref()
        }
        Transport::Remote(spec) => {
            live.get("url").and_then(Item::as_str) == Some(spec.url.as_str())
                && live.get("bearer_token_env_var").and_then(Item::as_str)
                    == spec.bearer_token_env_var.as_deref()
        }
    }
}

fn is_env_matching(live: &dyn TableLike, want: &[(String, String)]) -> bool {
    let Some(mut live_pairs) = env_pairs(live) else {
        return false;
    };
    let mut want_pairs = want.to_vec();
    live_pairs.sort();
    want_pairs.sort();
    live_pairs == want_pairs
}

fn env_pairs(live: &dyn TableLike) -> Option<Vec<(String, String)>> {
    let Some(item) = live.get("env") else {
        return Some(Vec::new());
    };
    let table = item.as_table_like()?;
    let mut pairs = Vec::new();
    for (key, value) in table.iter() {
        pairs.push((key.to_string(), value.as_str()?.to_string()));
    }
    Some(pairs)
}

fn str_values(live: &dyn TableLike, key: &str) -> Option<Vec<String>> {
    let Some(item) = live.get(key) else {
        return Some(Vec::new());
    };
    let values = item.as_array()?;
    values
        .iter()
        .map(|value| value.as_str().map(str::to_string))
        .collect()
}

fn adopt(name: &str, live: &dyn TableLike) -> Result<ServerEntry, String> {
    let is_stdio = live.contains_key("command");
    if !is_stdio && !live.contains_key("url") {
        return Err(String::from("no command or url key"));
    }
    let known: &[&str] = if is_stdio {
        &["command", "args", "env", "cwd"]
    } else {
        &["url", "bearer_token_env_var"]
    };
    for (key, _) in live.iter() {
        if !known.contains(&key) {
            return Err(format!("key {key} has no manifest shape"));
        }
    }
    let transport = if is_stdio {
        Transport::Stdio(StdioSpec {
            command: str_value(live, "command")?,
            args: str_values(live, "args").ok_or("key args has no manifest shape")?,
            env: env_pairs(live).ok_or("key env has no manifest shape")?,
            cwd: opt_str_value(live, "cwd")?,
        })
    } else {
        Transport::Remote(RemoteSpec {
            url: str_value(live, "url")?,
            bearer_token_env_var: opt_str_value(live, "bearer_token_env_var")?,
        })
    };
    Ok(ServerEntry {
        name: name.to_string(),
        transport,
        scope: ToolScope::Both,
        platforms: Vec::new(),
    })
}

fn str_value(live: &dyn TableLike, key: &str) -> Result<String, String> {
    live.get(key)
        .and_then(Item::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("key {key} has no manifest shape"))
}

fn opt_str_value(live: &dyn TableLike, key: &str) -> Result<Option<String>, String> {
    match live.get(key) {
        None => Ok(None),
        Some(item) => match item.as_str() {
            Some(text) => Ok(Some(text.to_string())),
            None => Err(format!("key {key} has no manifest shape")),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    const REAL_SHAPE: &str = r#"[projects."/Users/owaisquadri/Documents/Pillars"]
trust_level = "trusted"

[projects."/Users/owaisquadri/Documents/Flint"]
trust_level = "trusted"

[projects."/Users/owaisquadri/Documents/Claude/Projects/strava for golf"]
trust_level = "trusted"

[projects."/Users/owaisquadri/Library/Application Support/com.conductor.app/bin"]
trust_level = "trusted"

[projects."/Users/owaisquadri/operator-hq"]
trust_level = "trusted"

[plugins."github@openai-curated"]
enabled = true

[mcp_servers.mobbin]
url = "https://api.mobbin.com/mcp"

[mcp_servers.XcodeBuildMCP]
command = "npx"
args = ["-y", "xcodebuildmcp@latest", "mcp"]

[mcp_servers.playwright]
command = "npx"
args = ["@playwright/mcp@latest"]

[mcp_servers.shadcn]
command = "npx"
args = ["shadcn@latest", "mcp"]

[mcp_servers.lottiefiles-creator]
command = "npx"
args = ["-y", "@lottiefiles/creator-mcp@latest"]

[mcp_servers.lottiefiles-search]
command = "npx"
args = ["-y", "mcp-server-lottiefiles"]

[plugins."vercel-plugin@plugins-cli"]
enabled = true
"#;

    fn fixture(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("mcp-sync-codex-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create fixture dir");
        dir
    }

    fn stdio_entry(
        name: &str,
        command: &str,
        args: &[&str],
        env: &[(&str, &str)],
        cwd: Option<&str>,
        scope: ToolScope,
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
                cwd: cwd.map(str::to_string),
            }),
            scope,
            platforms: Vec::new(),
        }
    }

    fn remote_entry(name: &str, url: &str, bearer: Option<&str>, scope: ToolScope) -> ServerEntry {
        ServerEntry {
            name: name.to_string(),
            transport: Transport::Remote(RemoteSpec {
                url: url.to_string(),
                bearer_token_env_var: bearer.map(str::to_string),
            }),
            scope,
            platforms: Vec::new(),
        }
    }

    fn state_of(codex_managed: &[&str]) -> SyncState {
        SyncState {
            claude_managed: Vec::new(),
            codex_managed: codex_managed
                .iter()
                .map(|name| ManagedServer {
                    name: name.to_string(),
                    fingerprint: None,
                })
                .collect(),
        }
    }

    fn state_with(codex_managed: Vec<ManagedServer>) -> SyncState {
        SyncState {
            claude_managed: Vec::new(),
            codex_managed,
        }
    }

    fn managed_live(name: &str, table: &dyn TableLike) -> ManagedServer {
        ManagedServer {
            name: name.to_string(),
            fingerprint: Some(fingerprint_live(table)),
        }
    }

    #[test]
    fn stdio_table_carries_command_args_cwd_and_only_nonempty_env() {
        let bare = render_table(&stdio_entry(
            "alpha",
            "npx",
            &["-y", "alpha-mcp"],
            &[],
            None,
            ToolScope::Both,
        ));
        assert_eq!(bare.get("command").and_then(Item::as_str), Some("npx"));
        assert_eq!(
            str_values(&bare, "args").as_deref(),
            Some(&["-y".to_string(), "alpha-mcp".to_string()][..])
        );
        assert!(!bare.contains_key("env"));
        assert!(!bare.contains_key("cwd"));
        assert_eq!(bare.len(), 2);
        let with_env = render_table(&stdio_entry(
            "beta",
            "npx",
            &[],
            &[("PORT", "8080")],
            Some("/srv"),
            ToolScope::Both,
        ));
        assert_eq!(
            env_pairs(&with_env),
            Some(vec![("PORT".to_string(), "8080".to_string())])
        );
        assert_eq!(with_env.get("cwd").and_then(Item::as_str), Some("/srv"));
        assert_eq!(with_env.len(), 4);
    }

    #[test]
    fn cwd_entry_converges_on_the_second_apply() {
        let dir = fixture("cwd");
        let path = dir.join("config.toml");
        let manifest = Manifest {
            servers: vec![stdio_entry(
                "srvd",
                "npx",
                &["srv-mcp"],
                &[],
                Some("/srv"),
                ToolScope::Both,
            )],
        };
        let state = state_of(&[]);
        let first = sync(&path, &manifest, &state, false).expect("first sync");
        assert_eq!(first.len(), 1);
        let second = sync(&path, &manifest, &state, false).expect("second sync");
        assert!(second.is_empty(), "cwd entry never converges");
        let rows = check(&path, &manifest, &state).expect("check");
        assert!(matches!(
            &rows[0],
            DriftRow { tool: Tool::Codex, state: DriftState::Ok, server } if server == "srvd"
        ));
    }

    #[test]
    fn update_keeps_the_comment_above_a_managed_table() {
        let dir = fixture("update-comment");
        let path = dir.join("config.toml");
        let commented = "# keep me\n\
            [mcp_servers.tended]\n\
            command = \"npx\"\n\
            args = [\"old-args\"]\n";
        fs::write(&path, commented).expect("seed config");
        let manifest = Manifest {
            servers: vec![stdio_entry(
                "tended",
                "npx",
                &["new-args"],
                &[],
                None,
                ToolScope::Both,
            )],
        };
        let changes = sync(&path, &manifest, &state_of(&[]), false).expect("sync");
        assert_eq!(changes.len(), 1);
        assert!(matches!(changes[0].kind, ChangeKind::Update));
        let written = fs::read_to_string(&path).expect("read result");
        assert!(
            written.contains("# keep me\n[mcp_servers.tended]"),
            "{written}"
        );
        assert!(written.contains("args = [\"new-args\"]"), "{written}");
    }

    #[test]
    fn remote_table_carries_url_and_env_var_name_only() {
        let bare = render_table(&remote_entry(
            "mobbin",
            "https://api.mobbin.com/mcp",
            None,
            ToolScope::Both,
        ));
        assert_eq!(
            bare.get("url").and_then(Item::as_str),
            Some("https://api.mobbin.com/mcp")
        );
        assert_eq!(bare.len(), 1);
        let with_var = render_table(&remote_entry(
            "gated",
            "https://api.example.com/mcp",
            Some("MY_TOKEN"),
            ToolScope::Both,
        ));
        assert_eq!(
            with_var.get("bearer_token_env_var").and_then(Item::as_str),
            Some("MY_TOKEN")
        );
        assert_eq!(with_var.len(), 2);
    }

    #[test]
    fn managed_record_fingerprints_every_codex_managed_field() {
        let stdio = stdio_entry(
            "tool",
            "npx",
            &["-y", "tool-mcp"],
            &[("MODE", "fast")],
            Some("/srv/tool"),
            ToolScope::Both,
        );
        let managed = managed_server(&stdio);
        assert_eq!(managed.name, "tool");
        assert_eq!(
            managed.fingerprint.as_deref(),
            Some(fingerprint_live(&render_table(&stdio)).as_str())
        );
        for changed in [
            stdio_entry(
                "tool",
                "bunx",
                &["-y", "tool-mcp"],
                &[("MODE", "fast")],
                Some("/srv/tool"),
                ToolScope::Both,
            ),
            stdio_entry(
                "tool",
                "npx",
                &["tool-mcp"],
                &[("MODE", "fast")],
                Some("/srv/tool"),
                ToolScope::Both,
            ),
            stdio_entry(
                "tool",
                "npx",
                &["-y", "tool-mcp"],
                &[("MODE", "slow")],
                Some("/srv/tool"),
                ToolScope::Both,
            ),
            stdio_entry(
                "tool",
                "npx",
                &["-y", "tool-mcp"],
                &[("MODE", "fast")],
                Some("/srv/other"),
                ToolScope::Both,
            ),
        ] {
            assert_ne!(managed.fingerprint, managed_server(&changed).fingerprint);
        }

        let remote = remote_entry(
            "remote",
            "https://api.example.com/mcp",
            Some("API_TOKEN"),
            ToolScope::Both,
        );
        let remote_fingerprint = managed_server(&remote).fingerprint;
        assert_ne!(
            remote_fingerprint,
            managed_server(&remote_entry(
                "remote",
                "https://other.example.com/mcp",
                Some("API_TOKEN"),
                ToolScope::Both,
            ))
            .fingerprint
        );
        assert_ne!(
            remote_fingerprint,
            managed_server(&remote_entry(
                "remote",
                "https://api.example.com/mcp",
                Some("OTHER_TOKEN"),
                ToolScope::Both,
            ))
            .fingerprint
        );
    }

    #[test]
    fn fingerprint_is_semantic_and_keeps_unknown_or_malformed_values() {
        let compact = "[mcp_servers.tool]\ncommand='npx'\nargs=['-y','tool-mcp']\ncwd='/srv'\nenv={B='2',A='1'}\n";
        let decorated = "# decoration is irrelevant\n\
            [mcp_servers.tool] # table comment\n\
            args = [ \"-y\", \"tool-mcp\" ] # args comment\n\
            command = \"npx\"\n\
            cwd = \"/srv\"\n\
            [mcp_servers.tool.env]\n\
            A = \"1\"\n\
            B = \"2\"\n";
        let compact_doc = compact.parse::<DocumentMut>().expect("compact parses");
        let decorated_doc = decorated.parse::<DocumentMut>().expect("decorated parses");
        assert_eq!(
            fingerprint_live(live_server(&compact_doc, "tool").expect("compact table")),
            fingerprint_live(live_server(&decorated_doc, "tool").expect("decorated table"))
        );

        let malformed = "[mcp_servers.tool]\ncommand='npx'\nargs=42\n";
        let differently_malformed = "[mcp_servers.tool]\ncommand='npx'\nargs=true\n";
        let with_unknown = "[mcp_servers.tool]\ncommand='npx'\nargs=42\nunknown='kept'\n";
        let malformed_doc = malformed
            .parse::<DocumentMut>()
            .expect("malformed shape parses");
        let differently_malformed_doc = differently_malformed
            .parse::<DocumentMut>()
            .expect("different malformed shape parses");
        let with_unknown_doc = with_unknown
            .parse::<DocumentMut>()
            .expect("unknown field shape parses");
        let malformed_fingerprint =
            fingerprint_live(live_server(&malformed_doc, "tool").expect("malformed table"));
        assert_ne!(
            malformed_fingerprint,
            fingerprint_live(
                live_server(&differently_malformed_doc, "tool").expect("different malformed table")
            )
        );
        assert_ne!(
            malformed_fingerprint,
            fingerprint_live(live_server(&with_unknown_doc, "tool").expect("unknown table"))
        );
    }

    #[test]
    fn sync_preserves_foreign_sections_byte_for_byte() {
        let dir = fixture("real-shape");
        let path = dir.join("config.toml");
        fs::write(&path, REAL_SHAPE).expect("seed config");
        let manifest = Manifest {
            servers: vec![
                stdio_entry(
                    "XcodeBuildMCP",
                    "npx",
                    &["-y", "xcodebuildmcp@latest", "mcp"],
                    &[],
                    None,
                    ToolScope::Both,
                ),
                stdio_entry(
                    "playwright",
                    "npx",
                    &["@playwright/mcp@latest", "--headless"],
                    &[],
                    None,
                    ToolScope::Both,
                ),
                stdio_entry(
                    "supermemory",
                    "npx",
                    &["-y", "supermemory-mcp"],
                    &[("SUPERMEMORY_DIR", "/tmp/sm")],
                    None,
                    ToolScope::CodexOnly,
                ),
                stdio_entry(
                    "gmail",
                    "npx",
                    &["gmail-mcp"],
                    &[],
                    None,
                    ToolScope::ClaudeOnly,
                ),
            ],
        };
        let original = REAL_SHAPE.parse::<DocumentMut>().expect("seed parses");
        let shadcn = live_server(&original, "shadcn").expect("shadcn table");
        let state = state_with(vec![managed_live("shadcn", shadcn)]);
        let changes = sync(&path, &manifest, &state, false).expect("sync");
        assert_eq!(changes.len(), 3);
        assert!(matches!(
            &changes[0],
            Change { tool: Tool::Codex, kind: ChangeKind::Update, server } if server == "playwright"
        ));
        assert!(matches!(
            &changes[1],
            Change { tool: Tool::Codex, kind: ChangeKind::Add, server } if server == "supermemory"
        ));
        assert!(matches!(
            &changes[2],
            Change { tool: Tool::Codex, kind: ChangeKind::Remove, server } if server == "shadcn"
        ));
        let written = fs::read_to_string(&path).expect("read result");
        let foreign_sections = [
            "[projects.\"/Users/owaisquadri/Documents/Pillars\"]\ntrust_level = \"trusted\"\n",
            "[projects.\"/Users/owaisquadri/Documents/Flint\"]\ntrust_level = \"trusted\"\n",
            "[projects.\"/Users/owaisquadri/Documents/Claude/Projects/strava for golf\"]\ntrust_level = \"trusted\"\n",
            "[projects.\"/Users/owaisquadri/Library/Application Support/com.conductor.app/bin\"]\ntrust_level = \"trusted\"\n",
            "[projects.\"/Users/owaisquadri/operator-hq\"]\ntrust_level = \"trusted\"\n",
            "[plugins.\"github@openai-curated\"]\nenabled = true\n",
            "[plugins.\"vercel-plugin@plugins-cli\"]\nenabled = true\n",
            "[mcp_servers.mobbin]\nurl = \"https://api.mobbin.com/mcp\"\n",
            "[mcp_servers.lottiefiles-creator]\ncommand = \"npx\"\nargs = [\"-y\", \"@lottiefiles/creator-mcp@latest\"]\n",
            "[mcp_servers.lottiefiles-search]\ncommand = \"npx\"\nargs = [\"-y\", \"mcp-server-lottiefiles\"]\n",
            "[mcp_servers.XcodeBuildMCP]\ncommand = \"npx\"\nargs = [\"-y\", \"xcodebuildmcp@latest\", \"mcp\"]\n",
        ];
        for section in foreign_sections {
            assert!(
                written.contains(section),
                "lost section:\n{section}\nin:\n{written}"
            );
        }
        assert!(!written.contains("shadcn"), "{written}");
        assert!(!written.contains("gmail"), "{written}");
        let doc = written.parse::<DocumentMut>().expect("result parses");
        assert!(doc["mcp_servers"]["supermemory"]["env"]
            .as_value()
            .and_then(toml_edit::Value::as_inline_table)
            .is_some());
        assert_eq!(
            doc["mcp_servers"]["supermemory"]["env"]["SUPERMEMORY_DIR"].as_str(),
            Some("/tmp/sm")
        );
        assert_eq!(
            str_values(
                doc["mcp_servers"]["playwright"].as_table_like().unwrap(),
                "args"
            )
            .as_deref(),
            Some(
                &[
                    "@playwright/mcp@latest".to_string(),
                    "--headless".to_string()
                ][..]
            )
        );
    }

    #[test]
    fn converged_sync_makes_no_changes_and_no_write() {
        let dir = fixture("idempotent");
        let path = dir.join("config.toml");
        fs::write(&path, REAL_SHAPE).expect("seed config");
        let manifest = Manifest {
            servers: vec![stdio_entry(
                "supermemory",
                "npx",
                &["-y", "supermemory-mcp"],
                &[("SUPERMEMORY_DIR", "/tmp/sm")],
                None,
                ToolScope::Both,
            )],
        };
        let state = state_of(&[]);
        let first = sync(&path, &manifest, &state, false).expect("first sync");
        assert_eq!(first.len(), 1);
        let entries_after_first = fs::read_dir(&dir).unwrap().count();
        assert_eq!(entries_after_first, 2);
        let bytes_after_first = fs::read_to_string(&path).unwrap();
        let second = sync(&path, &manifest, &state, false).expect("second sync");
        assert!(second.is_empty());
        assert_eq!(fs::read_to_string(&path).unwrap(), bytes_after_first);
        assert_eq!(fs::read_dir(&dir).unwrap().count(), entries_after_first);
    }

    #[test]
    fn sync_keeps_comments_on_foreign_and_converged_tables() {
        let dir = fixture("comments");
        let path = dir.join("config.toml");
        let commented = "# codex config, hand-tended\n\
            [projects.\"/Users/o/x\"]\n\
            trust_level = \"trusted\"\n\
            \n\
            # foreign server, do not touch\n\
            [mcp_servers.foreign]\n\
            command = \"deno\"\n\
            args = []\n\
            \n\
            # managed but converged\n\
            [mcp_servers.kept]\n\
            command = \"npx\"\n\
            args = [\"kept-mcp\"]\n";
        fs::write(&path, commented).expect("seed config");
        let manifest = Manifest {
            servers: vec![
                stdio_entry("kept", "npx", &["kept-mcp"], &[], None, ToolScope::Both),
                stdio_entry("newbie", "npx", &["newbie-mcp"], &[], None, ToolScope::Both),
            ],
        };
        let changes = sync(&path, &manifest, &state_of(&[]), false).expect("sync");
        assert_eq!(changes.len(), 1);
        let written = fs::read_to_string(&path).expect("read result");
        assert!(
            written.contains("# codex config, hand-tended\n"),
            "{written}"
        );
        assert!(written.contains(
            "# foreign server, do not touch\n[mcp_servers.foreign]\ncommand = \"deno\"\nargs = []\n"
        ), "{written}");
        assert!(written.contains(
            "# managed but converged\n[mcp_servers.kept]\ncommand = \"npx\"\nargs = [\"kept-mcp\"]\n"
        ), "{written}");
        assert!(written.contains("[mcp_servers.newbie]"), "{written}");
    }

    #[test]
    fn absent_config_is_created_with_only_managed_tables() {
        let dir = fixture("absent");
        let path = dir.join("config.toml");
        let manifest = Manifest {
            servers: vec![
                stdio_entry(
                    "alpha",
                    "npx",
                    &["-y", "alpha-mcp"],
                    &[],
                    None,
                    ToolScope::Both,
                ),
                remote_entry(
                    "beta",
                    "https://api.example.com/mcp",
                    Some("MY_TOKEN"),
                    ToolScope::CodexOnly,
                ),
                stdio_entry(
                    "gamma",
                    "npx",
                    &["gamma-mcp"],
                    &[],
                    None,
                    ToolScope::ClaudeOnly,
                ),
            ],
        };
        let changes = sync(&path, &manifest, &state_of(&[]), false).expect("sync");
        assert_eq!(changes.len(), 2);
        let written = fs::read_to_string(&path).expect("file created");
        assert!(written.contains("[mcp_servers.alpha]"), "{written}");
        assert!(written.contains("[mcp_servers.beta]"), "{written}");
        assert!(!written.contains("[mcp_servers]\n"), "{written}");
        assert!(!written.contains("gamma"), "{written}");
        let doc = written.parse::<DocumentMut>().expect("result parses");
        let beta = doc["mcp_servers"]["beta"].as_table().expect("beta table");
        assert_eq!(beta.len(), 2);
        assert_eq!(
            beta.get("url").and_then(Item::as_str),
            Some("https://api.example.com/mcp")
        );
        assert_eq!(
            beta.get("bearer_token_env_var").and_then(Item::as_str),
            Some("MY_TOKEN")
        );
    }

    #[test]
    fn dry_run_plans_changes_and_writes_nothing() {
        let dir = fixture("dry");
        let path = dir.join("config.toml");
        fs::write(&path, REAL_SHAPE).expect("seed config");
        let manifest = Manifest {
            servers: vec![stdio_entry(
                "newbie",
                "npx",
                &["newbie-mcp"],
                &[],
                None,
                ToolScope::Both,
            )],
        };
        let changes = sync(&path, &manifest, &state_of(&[]), true).expect("dry sync");
        assert_eq!(changes.len(), 1);
        assert!(matches!(
            &changes[0],
            Change { tool: Tool::Codex, kind: ChangeKind::Add, server } if server == "newbie"
        ));
        assert_eq!(fs::read_to_string(&path).unwrap(), REAL_SHAPE);
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
    }

    #[test]
    fn sync_removes_matching_previously_managed_server() {
        let dir = fixture("exact-fingerprint-removal");
        let path = dir.join("config.toml");
        let seeded = "# top-level comment\n\
            unrelated = 17\n\
            \n\
            # foreign stdio comment\n\
            [mcp_servers.foreign-stdio]\n\
            command = \"deno\"\n\
            args = [\"run\", \"server.ts\"]\n\
            \n\
            # stale managed comment\n\
            [mcp_servers.stale]\n\
            command = \"npx\"\n\
            args = [\"stale-mcp\"]\n\
            \n\
            # foreign remote comment\n\
            [mcp_servers.foreign-remote]\n\
            url = \"https://foreign.example/mcp\"\n";
        fs::write(&path, seeded).expect("seed config");
        let doc = seeded.parse::<DocumentMut>().expect("seed parses");
        let state = state_with(vec![managed_live(
            "stale",
            live_server(&doc, "stale").expect("stale table"),
        )]);

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
            [Change { tool: Tool::Codex, server, kind: ChangeKind::Remove }] if server == "stale"
        ));
        let written = fs::read_to_string(&path).expect("read result");
        for foreign in [
            "# top-level comment\nunrelated = 17\n",
            "# foreign stdio comment\n[mcp_servers.foreign-stdio]\ncommand = \"deno\"\nargs = [\"run\", \"server.ts\"]\n",
            "# foreign remote comment\n[mcp_servers.foreign-remote]\nurl = \"https://foreign.example/mcp\"\n",
        ] {
            assert!(written.contains(foreign), "lost foreign content:\n{foreign}\nin:\n{written}");
        }
        assert!(!written.contains("stale"), "{written}");
    }

    #[test]
    fn sync_spares_changed_previously_managed_server() {
        let dir = fixture("changed-replacement");
        let path = dir.join("config.toml");
        let original = "[mcp_servers.stale]\ncommand = \"old-command\"\nargs = []\n";
        let original_doc = original.parse::<DocumentMut>().expect("original parses");
        let replacement =
            "# hand replacement\n[mcp_servers.stale]\ncommand=\"new-command\"\nargs=[]";
        fs::write(&path, replacement).expect("seed config");
        let state = state_with(vec![managed_live(
            "stale",
            live_server(&original_doc, "stale").expect("original table"),
        )]);

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
            [Change { tool: Tool::Codex, server, kind: ChangeKind::Spare }] if server == "stale"
        ));
        assert_eq!(fs::read_to_string(&path).unwrap(), replacement);
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
    }

    #[test]
    fn sync_spares_legacy_record_without_deletion_authority() {
        let dir = fixture("legacy-record");
        let path = dir.join("config.toml");
        let seeded = "[mcp_servers.stale]\ncommand = \"npx\"\nargs = []\n";
        fs::write(&path, seeded).expect("seed config");

        let changes = sync(
            &path,
            &Manifest {
                servers: Vec::new(),
            },
            &state_of(&["stale"]),
            false,
        )
        .expect("sync succeeds");

        assert!(matches!(
            changes.as_slice(),
            [Change { tool: Tool::Codex, server, kind: ChangeKind::Spare }] if server == "stale"
        ));
        assert_eq!(fs::read_to_string(&path).unwrap(), seeded);
    }

    #[test]
    fn fingerprint_ignores_formatting_and_decor() {
        let dir = fixture("formatting-only");
        let path = dir.join("config.toml");
        let original =
            "[mcp_servers.stale]\ncommand='npx'\nargs=['-y','stale-mcp']\nenv={B='2',A='1'}\n";
        let original_doc = original.parse::<DocumentMut>().expect("original parses");
        let reformatted = "# reformatted by hand\n\
            [mcp_servers.stale]\n\
            args = [ \"-y\", \"stale-mcp\" ]\n\
            command = \"npx\"\n\
            [mcp_servers.stale.env]\n\
            A = \"1\"\n\
            B = \"2\"\n";
        fs::write(&path, reformatted).expect("seed config");
        let state = state_with(vec![managed_live(
            "stale",
            live_server(&original_doc, "stale").expect("original table"),
        )]);

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
            [Change { tool: Tool::Codex, server, kind: ChangeKind::Remove }] if server == "stale"
        ));
        assert!(!fs::read_to_string(&path).unwrap().contains("stale"));
    }

    #[test]
    fn dry_run_exact_removal_reports_change_and_writes_nothing() {
        let dir = fixture("dry-removal");
        let path = dir.join("config.toml");
        let seeded = "[mcp_servers.stale]\ncommand = \"npx\"\nargs = []\n";
        fs::write(&path, seeded).expect("seed config");
        let doc = seeded.parse::<DocumentMut>().expect("seed parses");
        let state = state_with(vec![managed_live(
            "stale",
            live_server(&doc, "stale").expect("stale table"),
        )]);

        let changes = sync(
            &path,
            &Manifest {
                servers: Vec::new(),
            },
            &state,
            true,
        )
        .expect("dry-run sync succeeds");

        assert!(matches!(
            changes.as_slice(),
            [Change { tool: Tool::Codex, server, kind: ChangeKind::Remove }] if server == "stale"
        ));
        assert_eq!(fs::read_to_string(&path).unwrap(), seeded);
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
    }

    #[test]
    fn check_reports_ok_missing_drifted_and_unmanaged() {
        let dir = fixture("check");
        let path = dir.join("config.toml");
        let live = "[mcp_servers.okie]\n\
            command = \"npx\"\n\
            args = [\"okie-mcp\"]\n\
            \n\
            [mcp_servers.drifty]\n\
            command = \"npx\"\n\
            args = [\"old-args\"]\n\
            \n\
            [mcp_servers.remote-ok]\n\
            url = \"https://x/mcp\"\n\
            \n\
            [mcp_servers.remote-drift]\n\
            url = \"https://x/mcp\"\n\
            \n\
            [mcp_servers.foreign]\n\
            command = \"deno\"\n\
            args = []\n\
            \n\
            [mcp_servers.pending]\n\
            command = \"npx\"\n\
            args = [\"pending-mcp\"]\n";
        fs::write(&path, live).expect("seed config");
        let manifest = Manifest {
            servers: vec![
                stdio_entry("okie", "npx", &["okie-mcp"], &[], None, ToolScope::Both),
                stdio_entry("drifty", "npx", &["new-args"], &[], None, ToolScope::Both),
                stdio_entry(
                    "missing-one",
                    "npx",
                    &["missing-mcp"],
                    &[],
                    None,
                    ToolScope::CodexOnly,
                ),
                remote_entry("remote-ok", "https://x/mcp", None, ToolScope::Both),
                remote_entry(
                    "remote-drift",
                    "https://x/mcp",
                    Some("X_TOKEN"),
                    ToolScope::Both,
                ),
                stdio_entry(
                    "claude-side",
                    "npx",
                    &["c-mcp"],
                    &[],
                    None,
                    ToolScope::ClaudeOnly,
                ),
            ],
        };
        let pending = stdio_entry(
            "pending",
            "npx",
            &["pending-mcp"],
            &[],
            None,
            ToolScope::Both,
        );
        let state = state_with(vec![managed_server(&pending)]);
        let rows = check(&path, &manifest, &state).expect("check");
        assert_eq!(rows.len(), 7);
        assert!(matches!(
            &rows[0],
            DriftRow { tool: Tool::Codex, state: DriftState::Ok, server } if server == "okie"
        ));
        assert!(matches!(
            &rows[1],
            DriftRow { tool: Tool::Codex, state: DriftState::Drifted, server } if server == "drifty"
        ));
        assert!(matches!(
            &rows[2],
            DriftRow { tool: Tool::Codex, state: DriftState::Missing, server } if server == "missing-one"
        ));
        assert!(matches!(
            &rows[3],
            DriftRow { tool: Tool::Codex, state: DriftState::Ok, server } if server == "remote-ok"
        ));
        assert!(matches!(
            &rows[4],
            DriftRow { tool: Tool::Codex, state: DriftState::Drifted, server } if server == "remote-drift"
        ));
        assert!(matches!(
            &rows[5],
            DriftRow { tool: Tool::Codex, state: DriftState::Unmanaged, server } if server == "foreign"
        ));
        assert!(matches!(
            &rows[6],
            DriftRow { tool: Tool::Codex, state: DriftState::Drifted, server } if server == "pending"
        ));
    }

    #[test]
    fn check_classifies_exact_changed_legacy_and_unmanaged_servers() {
        let dir = fixture("check-protected-removals");
        let path = dir.join("config.toml");
        let live = "[mcp_servers.exact]\n\
            command = \"npx\"\n\
            args = [\"exact-mcp\"]\n\
            \n\
            [mcp_servers.changed]\n\
            command = \"replacement\"\n\
            args = []\n\
            \n\
            [mcp_servers.legacy]\n\
            command = \"legacy\"\n\
            args = []\n\
            \n\
            [mcp_servers.foreign]\n\
            url = \"https://foreign.example/mcp\"\n";
        fs::write(&path, live).expect("seed config");
        let live_doc = live.parse::<DocumentMut>().expect("live parses");
        let original_changed = "[mcp_servers.changed]\ncommand = \"original\"\nargs = []\n"
            .parse::<DocumentMut>()
            .expect("original parses");
        let state = state_with(vec![
            managed_live(
                "exact",
                live_server(&live_doc, "exact").expect("exact table"),
            ),
            managed_live(
                "changed",
                live_server(&original_changed, "changed").expect("original changed table"),
            ),
            ManagedServer {
                name: "legacy".to_string(),
                fingerprint: None,
            },
        ]);

        let rows = check(
            &path,
            &Manifest {
                servers: Vec::new(),
            },
            &state,
        )
        .expect("check succeeds");

        assert_eq!(rows.len(), 4);
        assert!(matches!(
            &rows[0],
            DriftRow { tool: Tool::Codex, server, state: DriftState::Drifted } if server == "exact"
        ));
        assert!(matches!(
            &rows[1],
            DriftRow { tool: Tool::Codex, server, state: DriftState::Spared } if server == "changed"
        ));
        assert!(matches!(
            &rows[2],
            DriftRow { tool: Tool::Codex, server, state: DriftState::Spared } if server == "legacy"
        ));
        assert!(matches!(
            &rows[3],
            DriftRow { tool: Tool::Codex, server, state: DriftState::Unmanaged } if server == "foreign"
        ));
    }

    #[test]
    fn unmanaged_translates_live_tables_and_skips_inexpressible_ones() {
        let dir = fixture("unmanaged");
        let path = dir.join("config.toml");
        let live = "[mcp_servers.managed-one]\n\
            command = \"npx\"\n\
            args = [\"managed-mcp\"]\n\
            \n\
            [mcp_servers.native-stdio]\n\
            command = \"deno\"\n\
            args = [\"run\", \"server.ts\"]\n\
            cwd = \"/srv\"\n\
            \n\
            [mcp_servers.native-stdio.env]\n\
            PORT = \"8080\"\n\
            \n\
            [mcp_servers.native-remote]\n\
            url = \"https://api.example.com/mcp\"\n\
            bearer_token_env_var = \"EXAMPLE_TOKEN\"\n\
            \n\
            [mcp_servers.rich]\n\
            command = \"npx\"\n\
            args = [\"rich-mcp\"]\n\
            startup_timeout_ms = 5000\n\
            \n\
            [mcp_servers.odd]\n\
            enabled = true\n";
        fs::write(&path, live).expect("seed config");
        let manifest = Manifest {
            servers: vec![stdio_entry(
                "managed-one",
                "npx",
                &["managed-mcp"],
                &[],
                None,
                ToolScope::Both,
            )],
        };
        let entries = unmanaged(&path, &manifest).expect("unmanaged");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "native-stdio");
        assert!(matches!(
            &entries[0].transport,
            Transport::Stdio(spec)
                if spec.command == "deno"
                    && spec.args == ["run", "server.ts"]
                    && spec.env == [("PORT".to_string(), "8080".to_string())]
                    && spec.cwd.as_deref() == Some("/srv")
        ));
        assert!(matches!(entries[0].scope, ToolScope::Both));
        assert_eq!(entries[1].name, "native-remote");
        assert!(matches!(
            &entries[1].transport,
            Transport::Remote(spec)
                if spec.url == "https://api.example.com/mcp"
                    && spec.bearer_token_env_var.as_deref() == Some("EXAMPLE_TOKEN")
        ));
    }

    #[test]
    fn invalid_toml_reports_parse_error() {
        let dir = fixture("invalid");
        let path = dir.join("config.toml");
        fs::write(&path, "[mcp_servers.broken\ncommand = \"npx\"\n").expect("seed config");
        let manifest = Manifest {
            servers: Vec::new(),
        };
        let result = sync(&path, &manifest, &state_of(&[]), false);
        assert!(matches!(result, Err(SyncError::ParseToml(_, _))));
    }
}
