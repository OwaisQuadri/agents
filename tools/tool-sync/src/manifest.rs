use std::collections::HashSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

use crate::SyncError;

#[derive(Debug, Eq, PartialEq)]
pub struct ToolManifest {
    pub tools: Vec<ToolSpec>,
}

#[derive(Debug, Eq, PartialEq)]
pub struct ToolSpec {
    pub name: String,
    pub source: ToolSource,
    pub platforms: Vec<Platform>,
    pub installer: InstallerSpec,
    pub commands: Vec<PathBuf>,
    pub mcp_server: Option<String>,
    pub pi_extension: Option<PathBuf>,
}

#[derive(Debug, Eq, PartialEq)]
pub enum ToolSource {
    Embedded { path: PathBuf },
    Git { url: String, revision: String },
}

#[derive(Debug, Eq, PartialEq)]
pub struct InstallerSpec {
    pub command: String,
    pub args: Vec<String>,
    pub preview_args: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Platform {
    Macos,
    Linux,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawManifest {
    tools: Vec<RawTool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTool {
    name: String,
    source: toml::Value,
    platforms: Vec<String>,
    installer: InstallerSpec,
    commands: Vec<PathBuf>,
    #[serde(default)]
    mcp_server: Option<String>,
    #[serde(default)]
    pi_extension: Option<PathBuf>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEmbedded {
    path: PathBuf,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawGit {
    url: String,
    revision: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSource {
    #[serde(default)]
    path: Option<PathBuf>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    revision: Option<String>,
    #[serde(default)]
    embedded: Option<toml::Value>,
    #[serde(default)]
    git: Option<RawGit>,
}

impl<'de> Deserialize<'de> for InstallerSpec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct RawInstaller {
            command: String,
            #[serde(default)]
            args: Vec<String>,
            #[serde(default)]
            preview_args: Vec<String>,
        }

        let raw = RawInstaller::deserialize(deserializer)?;
        Ok(Self {
            command: raw.command,
            args: raw.args,
            preview_args: raw.preview_args,
        })
    }
}

/// Loads and validates an executable-tool manifest.
/// Takes the tracked manifest path and returns declarations in file order.
/// Returns `SyncError` when the file cannot be read, TOML cannot be parsed,
/// or a declaration is incomplete, duplicated, unsupported, or escapes its root.
pub fn load(path: &Path) -> Result<ToolManifest, SyncError> {
    let text =
        fs::read_to_string(path).map_err(|error| SyncError::Io(path.to_path_buf(), error))?;
    let raw: RawManifest = toml::from_str(&text)
        .map_err(|error| SyncError::ParseToml(path.to_path_buf(), error.to_string()))?;
    validate(raw)
}

fn validate(raw: RawManifest) -> Result<ToolManifest, SyncError> {
    let mut names = HashSet::new();
    let mut provided_commands = HashSet::new();
    let mut tools = Vec::with_capacity(raw.tools.len());

    for raw_tool in raw.tools {
        if raw_tool.name.chars().any(char::is_control) {
            return Err(invalid("tool name contains control characters"));
        }
        let name = raw_tool.name.trim();
        if name.is_empty() {
            return Err(invalid("tool name is empty"));
        }
        if !names.insert(name.to_owned()) {
            return Err(invalid(format!("duplicate tool name {name}")));
        }

        let source = source_of(name, raw_tool.source)?;
        let platforms = platforms_of(name, raw_tool.platforms)?;
        validate_installer(name, &raw_tool.installer)?;

        if raw_tool.commands.is_empty() {
            return Err(tool_invalid(name, "commands is empty"));
        }
        for command in &raw_tool.commands {
            validate_relative(name, "command", command)?;
            let provided = command
                .file_name()
                .and_then(|part| part.to_str())
                .ok_or_else(|| {
                    tool_invalid(
                        name,
                        &format!("command {} has no file name", command.display()),
                    )
                })?;
            if !provided_commands.insert(provided.to_owned()) {
                return Err(tool_invalid(
                    name,
                    &format!("provided command {provided} is duplicated"),
                ));
            }
        }

        if raw_tool
            .mcp_server
            .as_ref()
            .is_some_and(|server| server.trim().is_empty())
        {
            return Err(tool_invalid(name, "MCP server reference is empty"));
        }
        if let Some(extension) = &raw_tool.pi_extension {
            validate_relative(name, "Pi extension", extension)?;
        }

        tools.push(ToolSpec {
            name: name.to_owned(),
            source,
            platforms,
            installer: raw_tool.installer,
            commands: raw_tool.commands,
            mcp_server: raw_tool.mcp_server,
            pi_extension: raw_tool.pi_extension,
        });
    }

    Ok(ToolManifest { tools })
}

fn source_of(name: &str, value: toml::Value) -> Result<ToolSource, SyncError> {
    let raw: RawSource = value
        .try_into()
        .map_err(|error| tool_invalid(name, &format!("source is invalid: {error}")))?;
    match (raw.path, raw.url, raw.revision, raw.embedded, raw.git) {
        (Some(path), None, None, None, None) => embedded_of(name, path),
        (None, None, None, Some(value), None) => {
            let path = if let Some(path) = value.as_str() {
                PathBuf::from(path)
            } else {
                let embedded: RawEmbedded = value.try_into().map_err(|error| {
                    tool_invalid(name, &format!("embedded source is invalid: {error}"))
                })?;
                embedded.path
            };
            embedded_of(name, path)
        }
        (None, Some(url), Some(revision), None, None) => git_of(name, url, revision),
        (None, None, None, None, Some(git)) => git_of(name, git.url, git.revision),
        _ => Err(tool_invalid(
            name,
            "source must contain exactly an embedded path or a Git URL and revision",
        )),
    }
}

fn embedded_of(name: &str, path: PathBuf) -> Result<ToolSource, SyncError> {
    validate_relative(name, "embedded source", &path)?;
    Ok(ToolSource::Embedded { path })
}

fn git_of(name: &str, url: String, revision: String) -> Result<ToolSource, SyncError> {
    if url.trim().is_empty() || revision.trim().is_empty() {
        return Err(tool_invalid(name, "Git URL and revision must not be empty"));
    }
    Ok(ToolSource::Git { url, revision })
}

fn platforms_of(name: &str, values: Vec<String>) -> Result<Vec<Platform>, SyncError> {
    if values.is_empty() {
        return Err(tool_invalid(name, "platforms is empty"));
    }
    let mut seen = HashSet::new();
    let mut platforms = Vec::with_capacity(values.len());
    for value in values {
        let platform = match value.as_str() {
            "macos" => Platform::Macos,
            "linux" => Platform::Linux,
            _ => return Err(tool_invalid(name, &format!("unsupported platform {value}"))),
        };
        if !seen.insert(platform) {
            return Err(tool_invalid(name, &format!("duplicate platform {value}")));
        }
        platforms.push(platform);
    }
    Ok(platforms)
}

fn validate_installer(name: &str, installer: &InstallerSpec) -> Result<(), SyncError> {
    if installer.command.trim().is_empty() {
        return Err(tool_invalid(name, "installer command is empty"));
    }
    if installer.args.iter().any(|arg| arg.is_empty()) {
        return Err(tool_invalid(
            name,
            "installer args contains an empty argument",
        ));
    }
    if installer.preview_args.iter().any(|arg| arg.is_empty()) {
        return Err(tool_invalid(
            name,
            "installer preview_args contains an empty argument",
        ));
    }
    Ok(())
}

fn validate_relative(name: &str, field: &str, path: &Path) -> Result<(), SyncError> {
    let is_valid = !path.as_os_str().is_empty()
        && !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_) | Component::CurDir));
    if !is_valid {
        return Err(tool_invalid(
            name,
            &format!("{field} path {} escapes its repository", path.display()),
        ));
    }
    Ok(())
}

fn invalid(detail: impl Into<String>) -> SyncError {
    SyncError::ManifestInvalid(detail.into())
}

fn tool_invalid(name: &str, detail: &str) -> SyncError {
    invalid(format!("tool {name} {detail}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static FIXTURE_ID: AtomicU64 = AtomicU64::new(0);

    fn load_text(text: &str) -> Result<ToolManifest, SyncError> {
        let fixture_id = FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "tool-sync-{}-{fixture_id}.toml",
            std::process::id()
        ));
        fs::write(&path, text).expect("write fixture");
        let result = load(&path);
        let _ = fs::remove_file(path);
        result
    }

    const TOOL: &str = r#"
[[tools]]
name = "rag"
platforms = ["macos", "linux"]
commands = ["rag"]
mcp_server = "rag"
pi_extension = "pi/extensions/rag.ts"
source = { url = "https://example.test/rag.git", revision = "abc123" }
installer = { command = "./install.sh", args = [], preview_args = ["--dry-run"] }
"#;

    #[test]
    fn parses_git_tool_and_adapters() {
        let manifest = load_text(TOOL).expect("manifest loads");
        assert_eq!(manifest.tools.len(), 1);
        let tool = &manifest.tools[0];
        assert_eq!(tool.name, "rag");
        assert_eq!(tool.platforms, [Platform::Macos, Platform::Linux]);
        assert_eq!(tool.commands, [PathBuf::from("rag")]);
        assert_eq!(tool.mcp_server.as_deref(), Some("rag"));
        assert_eq!(
            tool.pi_extension.as_deref(),
            Some(Path::new("pi/extensions/rag.ts"))
        );
        assert_eq!(tool.installer.preview_args, ["--dry-run"]);
        assert!(matches!(tool.source, ToolSource::Git { .. }));
    }

    #[test]
    fn parses_embedded_source() {
        for declaration in [
            "{ path = \"vendor/rag\" }",
            "{ embedded = { path = \"vendor/rag\" } }",
        ] {
            let text = TOOL.replace(
                "{ url = \"https://example.test/rag.git\", revision = \"abc123\" }",
                declaration,
            );
            let manifest = load_text(&text).expect("manifest loads");
            assert_eq!(
                manifest.tools[0].source,
                ToolSource::Embedded {
                    path: PathBuf::from("vendor/rag")
                }
            );
        }
    }

    #[test]
    fn rejects_duplicate_names_and_provided_commands() {
        let duplicate_name = format!("{TOOL}\n{}", TOOL.replace("[[tools]]", "[[tools]]"));
        assert!(load_text(&duplicate_name).is_err());

        let second = TOOL
            .replace("name = \"rag\"", "name = \"other\"")
            .replace("commands = [\"rag\"]", "commands = [\"bin/rag\"]");
        assert!(load_text(&format!("{TOOL}\n{second}")).is_err());
    }

    #[test]
    fn rejects_escaped_paths_and_incomplete_sources() {
        for source in [
            "{ path = \"../rag\" }",
            "{ url = \"https://example.test/rag.git\" }",
            "{ revision = \"abc123\" }",
            "{ embedded = {} }",
        ] {
            let text = TOOL.replace(
                "{ url = \"https://example.test/rag.git\", revision = \"abc123\" }",
                source,
            );
            assert!(
                load_text(&text).is_err(),
                "source should be rejected: {source}"
            );
        }
        assert!(load_text(&TOOL.replace("pi/extensions/rag.ts", "../rag.ts")).is_err());
    }

    #[test]
    fn rejects_control_characters_even_when_name_would_be_trimmed() {
        let text = TOOL.replace("name = \"rag\"", "name = \"\\nrag\\n\"");
        let error = load_text(&text).expect_err("control characters must be rejected");
        assert!(error.to_string().contains("control characters"));
    }

    #[test]
    fn rejects_unknown_platform() {
        assert!(load_text(&TOOL.replace("\"macos\", \"linux\"", "\"windows\"")).is_err());
    }

    #[test]
    fn repository_manifest_declares_rag() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/tools.toml");
        let manifest = load(&path).expect("repository manifest loads");
        assert_eq!(manifest.tools.len(), 1);
        assert_eq!(manifest.tools[0].name, "rag");
        assert_eq!(manifest.tools[0].mcp_server.as_deref(), Some("rag"));
    }
}
