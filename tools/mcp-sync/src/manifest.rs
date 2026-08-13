use std::path::Path;

use toml_edit::{value, Array, DocumentMut, InlineTable, Item, Table, TableLike};

use crate::error::SyncError;
use crate::fsio;

pub struct Manifest {
    pub servers: Vec<ServerEntry>,
}

pub struct ServerEntry {
    pub name: String,
    pub transport: Transport,
    pub scope: ToolScope,
    pub platforms: Vec<String>,
}

pub enum Transport {
    Stdio(StdioSpec),
    Remote(RemoteSpec),
}

pub struct StdioSpec {
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub cwd: Option<String>,
}

pub struct RemoteSpec {
    pub url: String,
    pub bearer_token_env_var: Option<String>,
}

pub enum ToolScope {
    Both,
    ClaudeOnly,
    CodexOnly,
}

pub struct SyncState {
    pub claude_managed: Vec<ManagedServer>,
    pub codex_managed: Vec<ManagedServer>,
}

pub struct ManagedServer {
    pub name: String,
    pub fingerprint: Option<String>,
}

fn managed_servers_of(
    _doc: &DocumentMut,
    _key: &str,
    _path: &Path,
) -> Result<Vec<ManagedServer>, SyncError> {
    unimplemented!()
}

fn managed_array(_servers: &[ManagedServer]) -> Array {
    unimplemented!()
}

/// Loads and validates the manifest file.
/// Takes the manifest path; returns the parsed Manifest in file order.
///
/// # Errors
/// Io when the file is unreadable; ParseToml on invalid TOML;
/// ManifestInvalid on a table with both command and url, neither, or an
/// unknown tools entry.
pub fn load_manifest(path: &Path) -> Result<Manifest, SyncError> {
    let text = match fsio::read_opt(path)? {
        Some(text) => text,
        None => {
            return Err(SyncError::Io(
                path.to_path_buf(),
                std::io::ErrorKind::NotFound.into(),
            ))
        }
    };
    let doc: DocumentMut = text
        .parse()
        .map_err(|err: toml_edit::TomlError| SyncError::ParseToml(path.to_path_buf(), err.to_string()))?;
    let mut servers = Vec::new();
    if let Some(item) = doc.get("servers") {
        let table = item.as_table_like().ok_or_else(|| {
            SyncError::ManifestInvalid("servers is not a table of server tables".to_string())
        })?;
        for (name, item) in table.iter() {
            servers.push(server_of(name, item)?);
        }
    }
    Ok(Manifest { servers })
}

/// Loads the managed-names state file, treating an absent file as empty state.
/// Takes the state path; returns SyncState.
///
/// # Errors
/// Io on an unreadable existing file; ParseToml on invalid TOML.
pub fn load_state(path: &Path) -> Result<SyncState, SyncError> {
    let text = match fsio::read_opt(path)? {
        Some(text) => text,
        None => {
            return Ok(SyncState {
                claude_managed: Vec::new(),
                codex_managed: Vec::new(),
            })
        }
    };
    let doc: DocumentMut = text
        .parse()
        .map_err(|err: toml_edit::TomlError| SyncError::ParseToml(path.to_path_buf(), err.to_string()))?;
    Ok(SyncState {
        claude_managed: names_of(&doc, "claude_managed", path)?,
        codex_managed: names_of(&doc, "codex_managed", path)?,
    })
}

/// Writes the managed-names state file; a rendered state byte-equal to the
/// current file is not written at all — no backup, no temp file, no dry: line.
/// Takes the state path, the state, and the dry-run flag; returns unit.
///
/// # Errors
/// Io on write failure; VerifyFailed when the re-read does not match.
pub fn save_state(path: &Path, state: &SyncState, is_dry_run: bool) -> Result<(), SyncError> {
    let snapshot = fsio::read_opt(path)?;
    let mut doc = DocumentMut::new();
    doc.insert("claude_managed", value(sorted_names(&state.claude_managed)));
    doc.insert("codex_managed", value(sorted_names(&state.codex_managed)));
    let rendered = doc.to_string();
    if snapshot.as_deref() == Some(rendered.as_str()) {
        return Ok(());
    }
    fsio::write_verified(path, &rendered, snapshot.as_deref(), is_dry_run)
}

/// Appends adopted servers to the manifest file, format-preserving.
/// Takes the manifest path, the entries, and the dry-run flag; returns unit.
///
/// # Errors
/// Io, ParseToml, VerifyFailed, or ChangedSinceRead from the underlying
/// verified write.
pub fn append_servers(
    path: &Path,
    entries: &[ServerEntry],
    is_dry_run: bool,
) -> Result<(), SyncError> {
    let text = match fsio::read_opt(path)? {
        Some(text) => text,
        None => {
            return Err(SyncError::Io(
                path.to_path_buf(),
                std::io::ErrorKind::NotFound.into(),
            ))
        }
    };
    let mut doc: DocumentMut = text
        .parse()
        .map_err(|err: toml_edit::TomlError| SyncError::ParseToml(path.to_path_buf(), err.to_string()))?;
    if doc.get("servers").is_none() {
        let mut implicit = Table::new();
        implicit.set_implicit(true);
        doc.insert("servers", Item::Table(implicit));
    }
    let servers = doc["servers"].as_table_mut().ok_or_else(|| {
        SyncError::ManifestInvalid("servers is not a table of server tables".to_string())
    })?;
    for entry in entries {
        if servers.contains_key(&entry.name) {
            return Err(invalid(&entry.name, "is already in the manifest"));
        }
        servers.insert(&entry.name, Item::Table(table_of(entry)));
    }
    fsio::write_verified(path, &doc.to_string(), Some(&text), is_dry_run)
}

fn invalid(name: &str, offense: &str) -> SyncError {
    SyncError::ManifestInvalid(format!("server {name} {offense}"))
}

fn server_of(name: &str, item: &Item) -> Result<ServerEntry, SyncError> {
    let table = item
        .as_table_like()
        .ok_or_else(|| invalid(name, "is not a table"))?;
    let transport = match (table.get("command"), table.get("url")) {
        (Some(_), Some(_)) => return Err(invalid(name, "has both command and url")),
        (None, None) => return Err(invalid(name, "has neither command nor url")),
        (Some(command), None) => Transport::Stdio(stdio_of(name, command, table)?),
        (None, Some(url)) => Transport::Remote(RemoteSpec {
            url: str_of(name, "url", url)?,
            bearer_token_env_var: opt_str_of(name, "bearer_token_env_var", table)?,
        }),
    };
    Ok(ServerEntry {
        name: name.to_string(),
        transport,
        scope: scope_of(name, table.get("tools"))?,
        platforms: platforms_of(name, table.get("platforms"))?,
    })
}

/// Reads the optional `platforms` key. An empty result means every platform,
/// so a server that omits the key keeps its current behaviour.
///
/// # Errors
/// ManifestInvalid when platforms is not an array of strings.
fn platforms_of(name: &str, item: Option<&Item>) -> Result<Vec<String>, SyncError> {
    let Some(item) = item else {
        return Ok(Vec::new());
    };
    let array = item
        .as_array()
        .ok_or_else(|| invalid(name, "platforms is not an array"))?;
    let mut out = Vec::new();
    for entry in array.iter() {
        let text = entry
            .as_str()
            .ok_or_else(|| invalid(name, "platforms holds a non-string entry"))?;
        out.push(text.to_string());
    }
    Ok(out)
}

impl ServerEntry {
    /// Reports whether this server should be installed on the running platform.
    /// Takes nothing; returns true when the manifest names this platform or
    /// names none at all.
    pub fn is_for_this_platform(&self) -> bool {
        self.platforms.is_empty() || self.platforms.iter().any(|name| name == std::env::consts::OS)
    }
}

fn stdio_of(name: &str, command: &Item, table: &dyn TableLike) -> Result<StdioSpec, SyncError> {
    let mut args = Vec::new();
    if let Some(item) = table.get("args") {
        let array = item
            .as_array()
            .ok_or_else(|| invalid(name, "args is not an array"))?;
        for entry in array.iter() {
            let text = entry
                .as_str()
                .ok_or_else(|| invalid(name, "args holds a non-string entry"))?;
            args.push(text.to_string());
        }
    }
    let mut env = Vec::new();
    if let Some(item) = table.get("env") {
        let pairs = item
            .as_table_like()
            .ok_or_else(|| invalid(name, "env is not a table"))?;
        for (key, entry) in pairs.iter() {
            let text = entry
                .as_str()
                .ok_or_else(|| invalid(name, &format!("env {key} is not a string")))?;
            env.push((key.to_string(), text.to_string()));
        }
    }
    Ok(StdioSpec {
        command: str_of(name, "command", command)?,
        args,
        env,
        cwd: opt_str_of(name, "cwd", table)?,
    })
}

fn scope_of(name: &str, tools: Option<&Item>) -> Result<ToolScope, SyncError> {
    let Some(tools) = tools else {
        return Ok(ToolScope::Both);
    };
    let array = tools
        .as_array()
        .ok_or_else(|| invalid(name, "tools is not an array"))?;
    let mut entries = Vec::new();
    for entry in array.iter() {
        let text = entry
            .as_str()
            .ok_or_else(|| invalid(name, "tools holds a non-string entry"))?;
        entries.push(text);
    }
    match entries.as_slice() {
        ["claude"] => Ok(ToolScope::ClaudeOnly),
        ["codex"] => Ok(ToolScope::CodexOnly),
        ["claude", "codex"] | ["codex", "claude"] => Ok(ToolScope::Both),
        _ => Err(invalid(
            name,
            &format!("tools names no known tool set: [{}]", entries.join(", ")),
        )),
    }
}

fn str_of(name: &str, key: &str, item: &Item) -> Result<String, SyncError> {
    item.as_str()
        .map(str::to_string)
        .ok_or_else(|| invalid(name, &format!("{key} is not a string")))
}

fn opt_str_of(name: &str, key: &str, table: &dyn TableLike) -> Result<Option<String>, SyncError> {
    match table.get(key) {
        Some(item) => Ok(Some(str_of(name, key, item)?)),
        None => Ok(None),
    }
}

fn names_of(doc: &DocumentMut, key: &str, path: &Path) -> Result<Vec<String>, SyncError> {
    let Some(item) = doc.get(key) else {
        return Ok(Vec::new());
    };
    let array = item.as_array().ok_or_else(|| {
        SyncError::ParseToml(path.to_path_buf(), format!("{key} is not an array"))
    })?;
    let mut names = Vec::new();
    for entry in array.iter() {
        let text = entry.as_str().ok_or_else(|| {
            SyncError::ParseToml(path.to_path_buf(), format!("{key} holds a non-string entry"))
        })?;
        names.push(text.to_string());
    }
    Ok(names)
}

fn sorted_names(names: &[String]) -> Array {
    let mut sorted = names.to_vec();
    sorted.sort();
    Array::from_iter(sorted)
}

fn table_of(entry: &ServerEntry) -> Table {
    let mut table = Table::new();
    table.decor_mut().set_prefix("\n");
    match &entry.transport {
        Transport::Stdio(spec) => {
            table.insert("command", value(spec.command.as_str()));
            table.insert(
                "args",
                value(Array::from_iter(spec.args.iter().map(String::as_str))),
            );
            if !spec.env.is_empty() {
                let mut env = InlineTable::new();
                for (key, val) in &spec.env {
                    env.insert(key.as_str(), val.as_str().into());
                }
                table.insert("env", value(env));
            }
            if let Some(cwd) = &spec.cwd {
                table.insert("cwd", value(cwd.as_str()));
            }
        }
        Transport::Remote(spec) => {
            table.insert("url", value(spec.url.as_str()));
            if let Some(var) = &spec.bearer_token_env_var {
                table.insert("bearer_token_env_var", value(var.as_str()));
            }
        }
    }
    match entry.scope {
        ToolScope::Both => {}
        ToolScope::ClaudeOnly => {
            table.insert("tools", value(Array::from_iter(["claude"])));
        }
        ToolScope::CodexOnly => {
            table.insert("tools", value(Array::from_iter(["codex"])));
        }
    }
    table
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn fixture(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "mcp-sync-manifest-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create fixture dir");
        dir
    }

    fn manifest_from(dir: &Path, text: &str) -> Result<Manifest, SyncError> {
        let path = dir.join("mcp-servers.toml");
        fs::write(&path, text).expect("seed manifest");
        load_manifest(&path)
    }

    fn load_err(dir: &Path, text: &str) -> SyncError {
        match manifest_from(dir, text) {
            Err(err) => err,
            Ok(_) => panic!("manifest unexpectedly loaded"),
        }
    }

    #[test]
    fn a_server_without_platforms_runs_everywhere() {
        let dir = fixture("platforms-absent");
        let manifest = manifest_from(&dir, "[servers.zulu]\ncommand = \"npx\"\n")
            .expect("manifest loads");
        assert!(manifest.servers[0].platforms.is_empty());
        assert!(manifest.servers[0].is_for_this_platform());
    }

    #[test]
    fn platforms_gates_a_server_to_the_named_os() {
        let dir = fixture("platforms-named");
        let manifest = manifest_from(
            &dir,
            "[servers.mac_only]\ncommand = \"n\"\nplatforms = [\"macos\"]\n\
             \n[servers.linux_only]\ncommand = \"n\"\nplatforms = [\"linux\"]\n\
             \n[servers.both]\ncommand = \"n\"\nplatforms = [\"macos\", \"linux\"]\n",
        )
        .expect("manifest loads");
        let by_name = |want: &str| {
            manifest
                .servers
                .iter()
                .find(|entry| entry.name == want)
                .expect("server present")
                .is_for_this_platform()
        };
        assert_eq!(by_name("mac_only"), cfg!(target_os = "macos"));
        assert_eq!(by_name("linux_only"), cfg!(target_os = "linux"));
        assert!(by_name("both"));
    }

    #[test]
    fn platforms_must_be_an_array_of_strings() {
        let dir = fixture("platforms-invalid");
        let err = load_err(&dir, "[servers.zulu]\ncommand = \"n\"\nplatforms = \"macos\"\n");
        assert!(matches!(err, SyncError::ManifestInvalid(_)));
        let err = load_err(&dir, "[servers.zulu]\ncommand = \"n\"\nplatforms = [7]\n");
        assert!(matches!(err, SyncError::ManifestInvalid(_)));
    }

    const MIXED: &str = "\
[servers.zulu]
command = \"npx\"
args = [\"-y\", \"zulu-mcp\"]
env = { FIRST = \"1\", SECOND = \"2\" }
cwd = \"/tmp/zulu\"

[servers.alpha]
url = \"https://alpha.example/mcp\"
bearer_token_env_var = \"ALPHA_TOKEN\"
tools = [\"claude\"]

# a comment between tables stays invisible to the parser
[servers.mike]
command = \"uv\"
args = []
tools = [\"codex\"]

[servers.mike.env]
A = \"1\"
B = \"2\"

[servers.both]
url = \"https://both.example/mcp\"
tools = [\"claude\", \"codex\"]
";

    #[test]
    fn manifest_loads_mixed_servers_in_file_order() {
        let dir = fixture("mixed");
        let manifest = manifest_from(&dir, MIXED).expect("mixed manifest loads");
        let names: Vec<&str> = manifest
            .servers
            .iter()
            .map(|server| server.name.as_str())
            .collect();
        assert_eq!(names, ["zulu", "alpha", "mike", "both"]);

        let zulu = &manifest.servers[0];
        assert!(matches!(zulu.scope, ToolScope::Both));
        let Transport::Stdio(spec) = &zulu.transport else {
            panic!("zulu is stdio");
        };
        assert_eq!(spec.command, "npx");
        assert_eq!(spec.args, ["-y", "zulu-mcp"]);
        assert_eq!(
            spec.env,
            [
                ("FIRST".to_string(), "1".to_string()),
                ("SECOND".to_string(), "2".to_string())
            ]
        );
        assert_eq!(spec.cwd.as_deref(), Some("/tmp/zulu"));

        let alpha = &manifest.servers[1];
        assert!(matches!(alpha.scope, ToolScope::ClaudeOnly));
        let Transport::Remote(spec) = &alpha.transport else {
            panic!("alpha is remote");
        };
        assert_eq!(spec.url, "https://alpha.example/mcp");
        assert_eq!(spec.bearer_token_env_var.as_deref(), Some("ALPHA_TOKEN"));

        let mike = &manifest.servers[2];
        assert!(matches!(mike.scope, ToolScope::CodexOnly));
        let Transport::Stdio(spec) = &mike.transport else {
            panic!("mike is stdio");
        };
        assert!(spec.args.is_empty());
        assert_eq!(
            spec.env,
            [
                ("A".to_string(), "1".to_string()),
                ("B".to_string(), "2".to_string())
            ]
        );
        assert_eq!(spec.cwd, None);

        let both = &manifest.servers[3];
        assert!(matches!(both.scope, ToolScope::Both));
        let Transport::Remote(spec) = &both.transport else {
            panic!("both is remote");
        };
        assert_eq!(spec.bearer_token_env_var, None);
    }

    #[test]
    fn manifest_rejects_server_with_both_command_and_url() {
        let dir = fixture("both-arm");
        let err = load_err(&dir, "[servers.foo]\ncommand = \"x\"\nurl = \"https://y\"\n");
        assert!(matches!(err, SyncError::ManifestInvalid(_)));
        let text = err.to_string();
        assert!(text.contains("foo"), "{text}");
        assert!(text.contains("both command and url"), "{text}");
    }

    #[test]
    fn manifest_rejects_server_with_neither_command_nor_url() {
        let dir = fixture("neither-arm");
        let err = load_err(&dir, "[servers.bar]\ncwd = \"/tmp\"\n");
        assert!(matches!(err, SyncError::ManifestInvalid(_)));
        let text = err.to_string();
        assert!(text.contains("bar"), "{text}");
        assert!(text.contains("neither command nor url"), "{text}");
    }

    #[test]
    fn manifest_rejects_unknown_tools_values() {
        let dir = fixture("tools-arm");
        for tools in ["[\"gemini\"]", "[]", "[\"claude\", \"gemini\"]", "\"claude\"", "[3]"] {
            let err = load_err(
                &dir,
                &format!("[servers.baz]\ncommand = \"x\"\ntools = {tools}\n"),
            );
            assert!(matches!(err, SyncError::ManifestInvalid(_)));
            let text = err.to_string();
            assert!(text.contains("baz"), "{text}");
            assert!(text.contains("tools"), "{text}");
        }
    }

    #[test]
    fn absent_state_file_is_empty_state() {
        let dir = fixture("absent-state");
        let state = load_state(&dir.join("missing.toml")).expect("absent state loads");
        assert!(state.claude_managed.is_empty());
        assert!(state.codex_managed.is_empty());
    }

    #[test]
    fn save_state_sorts_names_and_round_trips() {
        let dir = fixture("state");
        let path = dir.join("mcp-sync-state.toml");
        let state = SyncState {
            claude_managed: vec!["zeta".to_string(), "alpha".to_string()],
            codex_managed: vec!["mike".to_string()],
        };
        save_state(&path, &state, false).expect("state saves");
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "claude_managed = [\"alpha\", \"zeta\"]\ncodex_managed = [\"mike\"]\n"
        );
        let reloaded = load_state(&path).expect("state reloads");
        assert_eq!(reloaded.claude_managed, ["alpha", "zeta"]);
        assert_eq!(reloaded.codex_managed, ["mike"]);
    }

    #[test]
    fn save_state_skips_write_when_rendered_state_is_converged() {
        let dir = fixture("converged-state");
        let path = dir.join("mcp-sync-state.toml");
        let state = SyncState {
            claude_managed: vec!["alpha".to_string()],
            codex_managed: vec!["mike".to_string()],
        };
        save_state(&path, &state, false).expect("first save writes");
        let bytes = fs::read_to_string(&path).unwrap();
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        save_state(&path, &state, false).expect("converged save skips");
        assert_eq!(fs::read_to_string(&path).unwrap(), bytes);
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        save_state(&path, &state, true).expect("converged dry-run skips");
        assert_eq!(fs::read_to_string(&path).unwrap(), bytes);
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
    }

    const PRIOR: &str = "\
# manifest header comment
[servers.first]
command = \"npx\"
args = [\"-y\", \"first-mcp\"]

# a kept comment between tables
[servers.second]
url = \"https://second.example/mcp\"
tools = [\"codex\"]
";

    #[test]
    fn append_preserves_prior_bytes_and_round_trips() {
        let dir = fixture("append");
        let path = dir.join("mcp-servers.toml");
        fs::write(&path, PRIOR).expect("seed manifest");
        let entries = [
            ServerEntry {
                name: "third".to_string(),
                transport: Transport::Stdio(StdioSpec {
                    command: "uv".to_string(),
                    args: vec!["run".to_string(), "server.py".to_string()],
                    env: vec![("KEY".to_string(), "VAL".to_string())],
                    cwd: Some("/tmp/third".to_string()),
                }),
                scope: ToolScope::ClaudeOnly,
                platforms: Vec::new(),
            },
            ServerEntry {
                name: "fourth".to_string(),
                transport: Transport::Remote(RemoteSpec {
                    url: "https://fourth.example/mcp".to_string(),
                    bearer_token_env_var: None,
                }),
                scope: ToolScope::Both,
                platforms: Vec::new(),
            },
        ];
        append_servers(&path, &entries, false).expect("append succeeds");
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.starts_with(PRIOR), "{text}");

        let manifest = load_manifest(&path).expect("appended manifest reloads");
        let names: Vec<&str> = manifest
            .servers
            .iter()
            .map(|server| server.name.as_str())
            .collect();
        assert_eq!(names, ["first", "second", "third", "fourth"]);

        let third = &manifest.servers[2];
        assert!(matches!(third.scope, ToolScope::ClaudeOnly));
        let Transport::Stdio(spec) = &third.transport else {
            panic!("third is stdio");
        };
        assert_eq!(spec.command, "uv");
        assert_eq!(spec.args, ["run", "server.py"]);
        assert_eq!(spec.env, [("KEY".to_string(), "VAL".to_string())]);
        assert_eq!(spec.cwd.as_deref(), Some("/tmp/third"));

        let fourth = &manifest.servers[3];
        assert!(matches!(fourth.scope, ToolScope::Both));
        let fourth_text = text.split("[servers.fourth]").nth(1).expect("fourth table");
        assert!(!fourth_text.contains("tools"), "{fourth_text}");
    }

    #[test]
    fn append_rejects_name_already_in_manifest() {
        let dir = fixture("append-dup");
        let path = dir.join("mcp-servers.toml");
        fs::write(&path, PRIOR).expect("seed manifest");
        let entries = [ServerEntry {
            name: "first".to_string(),
            transport: Transport::Remote(RemoteSpec {
                url: "https://dup.example/mcp".to_string(),
                bearer_token_env_var: None,
            }),
            scope: ToolScope::Both,
            platforms: Vec::new(),
        }];
        let err = append_servers(&path, &entries, false).expect_err("duplicate rejected");
        assert!(matches!(err, SyncError::ManifestInvalid(_)));
        let text = err.to_string();
        assert!(text.contains("first"), "{text}");
        assert!(text.contains("already in the manifest"), "{text}");
        assert_eq!(fs::read_to_string(&path).unwrap(), PRIOR);
    }

    #[test]
    fn repo_manifest_loads_all_nineteen_servers() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/mcp-servers.toml");
        let manifest = load_manifest(&path).expect("repo manifest loads");
        assert_eq!(manifest.servers.len(), 19);
        assert_eq!(manifest.servers[0].name, "XcodeBuildMCP");
        assert!(manifest
            .servers
            .iter()
            .all(|server| server.name != "gmail"));
    }
}
