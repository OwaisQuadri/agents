use std::path::Path;

use toml_edit::{DocumentMut, Item, Table, TableLike};

use crate::drift::{Change, ChangeKind, DriftRow, DriftState, Tool};
use crate::error::SyncError;
use crate::fsio;
use crate::manifest::{
    Manifest, RemoteSpec, ServerEntry, StdioSpec, SyncState, ToolScope, Transport,
};

/// Renders one manifest entry as a Codex [mcp_servers.<name>] table.
/// Takes the entry; returns the TOML table.
///
/// # Errors
/// none
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
    let mut changes = Vec::new();
    for entry in manifest.servers.iter().filter(|entry| is_codex_scoped(entry)) {
        let kind = match live_server(&doc, &entry.name) {
            None => ChangeKind::Add,
            Some(live) if is_matching(live, entry) => continue,
            Some(_) => ChangeKind::Update,
        };
        let mut table = render_table(entry);
        let parent = parent_table_mut(&mut doc, path)?;
        if let Some(position) = parent
            .get(&entry.name)
            .and_then(Item::as_table)
            .and_then(Table::position)
        {
            table.set_position(Some(position));
        }
        parent.insert(&entry.name, Item::Table(table));
        changes.push(Change { tool: Tool::Codex, server: entry.name.clone(), kind });
    }
    for name in &state.codex_managed {
        if manifest
            .servers
            .iter()
            .any(|entry| is_codex_scoped(entry) && entry.name == *name)
        {
            continue;
        }
        let Some(parent) = doc.get_mut("mcp_servers").and_then(Item::as_table_mut) else {
            break;
        };
        if parent.remove(name).is_some() {
            changes.push(Change {
                tool: Tool::Codex,
                server: name.clone(),
                kind: ChangeKind::Remove,
            });
        }
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
/// codex-scoped manifest server plus one per unmanaged live server.
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
    for entry in manifest.servers.iter().filter(|entry| is_codex_scoped(entry)) {
        let live = servers
            .and_then(|table| table.get(&entry.name))
            .and_then(Item::as_table_like);
        let drift = match live {
            None => DriftState::Missing,
            Some(table) if is_matching(table, entry) => DriftState::Ok,
            Some(_) => DriftState::Drifted,
        };
        rows.push(DriftRow { server: entry.name.clone(), tool: Tool::Codex, state: drift });
    }
    if let Some(servers) = servers {
        for (name, _) in servers.iter() {
            let is_known = manifest.servers.iter().any(|entry| entry.name == name)
                || state.codex_managed.iter().any(|managed| managed.as_str() == name);
            if !is_known {
                rows.push(DriftRow {
                    server: name.to_string(),
                    tool: Tool::Codex,
                    state: DriftState::Unmanaged,
                });
            }
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
            SyncError::ParseToml(path.to_path_buf(), String::from("mcp_servers is not a table"))
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
    Ok(ServerEntry { name: name.to_string(), transport, scope: ToolScope::Both })
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
        let dir = std::env::temp_dir().join(format!(
            "mcp-sync-codex-{name}-{}",
            std::process::id()
        ));
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
        }
    }

    fn state_of(codex_managed: &[&str]) -> SyncState {
        SyncState {
            claude_managed: Vec::new(),
            codex_managed: codex_managed.iter().map(|name| name.to_string()).collect(),
        }
    }

    #[test]
    fn stdio_table_carries_command_args_and_only_nonempty_env() {
        let bare = render_table(&stdio_entry(
            "alpha",
            "npx",
            &["-y", "alpha-mcp"],
            &[],
            Some("/srv"),
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
            None,
            ToolScope::Both,
        ));
        assert_eq!(
            env_pairs(&with_env),
            Some(vec![("PORT".to_string(), "8080".to_string())])
        );
        assert_eq!(with_env.len(), 3);
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
                stdio_entry("gmail", "npx", &["gmail-mcp"], &[], None, ToolScope::ClaudeOnly),
            ],
        };
        let state = state_of(&["XcodeBuildMCP", "playwright", "shadcn"]);
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
            assert!(written.contains(section), "lost section:\n{section}\nin:\n{written}");
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
            str_values(doc["mcp_servers"]["playwright"].as_table_like().unwrap(), "args")
                .as_deref(),
            Some(&["@playwright/mcp@latest".to_string(), "--headless".to_string()][..])
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
        assert!(written.contains("# codex config, hand-tended\n"), "{written}");
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
                stdio_entry("alpha", "npx", &["-y", "alpha-mcp"], &[], None, ToolScope::Both),
                remote_entry(
                    "beta",
                    "https://api.example.com/mcp",
                    Some("MY_TOKEN"),
                    ToolScope::CodexOnly,
                ),
                stdio_entry("gamma", "npx", &["gamma-mcp"], &[], None, ToolScope::ClaudeOnly),
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
        assert_eq!(beta.get("url").and_then(Item::as_str), Some("https://api.example.com/mcp"));
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
            servers: vec![stdio_entry("newbie", "npx", &["newbie-mcp"], &[], None, ToolScope::Both)],
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
                stdio_entry("missing-one", "npx", &["missing-mcp"], &[], None, ToolScope::CodexOnly),
                remote_entry("remote-ok", "https://x/mcp", None, ToolScope::Both),
                remote_entry("remote-drift", "https://x/mcp", Some("X_TOKEN"), ToolScope::Both),
                stdio_entry("claude-side", "npx", &["c-mcp"], &[], None, ToolScope::ClaudeOnly),
            ],
        };
        let state = state_of(&["okie", "drifty", "pending"]);
        let rows = check(&path, &manifest, &state).expect("check");
        assert_eq!(rows.len(), 6);
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
        let manifest = Manifest { servers: Vec::new() };
        let result = sync(&path, &manifest, &state_of(&[]), false);
        assert!(matches!(result, Err(SyncError::ParseToml(_, _))));
    }
}
