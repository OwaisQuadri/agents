use std::path::Path;

use serde_json::{json, Map, Value};

use crate::drift::{Change, ChangeKind, Tool};
use crate::error::SyncError;
use crate::fsio;

pub struct HookRegistration {
    pub command: String,
    pub timeout_secs: u64,
}

/// Validates that the live hooks file parses into the hooks shape, before
/// any write touches it.
/// Takes the hooks.json path; returns Ok when the file is absent or holds
/// an object whose hooks.UserPromptSubmit slot can hold group entries.
///
/// # Errors
/// ParseJson on invalid JSON or a wrong-shaped document; Io on a read
/// failure other than not-found.
pub fn validate(path: &Path) -> Result<(), SyncError> {
    let Some(text) = fsio::read_opt(path)? else {
        return Ok(());
    };
    let mut root: Value = serde_json::from_str(&text)
        .map_err(|err| SyncError::ParseJson(path.to_path_buf(), err.to_string()))?;
    prompt_groups(&mut root, path).map(|_| ())
}

/// Ensures the Codex hooks file carries the UserPromptSubmit registration.
/// Matches the managed entry by the stable hooks/rag-recall command suffix,
/// so foreign hook entries survive and a moved repo path is updated in
/// place; creates the file when absent.
/// Takes the hooks.json path, the registration, and the dry-run flag;
/// returns the changes.
///
/// # Errors
/// Io, ParseJson, ChangedSinceRead, BackupFailed, VerifyFailed.
pub fn sync_codex_hook(
    path: &Path,
    reg: &HookRegistration,
    is_dry_run: bool,
) -> Result<Vec<Change>, SyncError> {
    let snapshot = fsio::read_opt(path)?;
    let mut root = match snapshot.as_deref() {
        Some(text) => serde_json::from_str(text)
            .map_err(|err| SyncError::ParseJson(path.to_path_buf(), err.to_string()))?,
        None => Value::Object(Map::new()),
    };
    let groups = prompt_groups(&mut root, path)?;
    let kind = match managed_position(groups) {
        Some((group_idx, entry_idx)) => {
            let entry = &mut groups[group_idx]["hooks"][entry_idx];
            let is_command_current =
                entry.get("command").and_then(Value::as_str) == Some(reg.command.as_str());
            let is_timeout_current =
                entry.get("timeout").and_then(Value::as_u64) == Some(reg.timeout_secs);
            if is_command_current && is_timeout_current {
                return Ok(Vec::new());
            }
            entry["command"] = json!(reg.command);
            entry["timeout"] = json!(reg.timeout_secs);
            ChangeKind::Update
        }
        None => {
            groups.push(json!({
                "hooks": [{
                    "type": "command",
                    "command": reg.command,
                    "timeout": reg.timeout_secs,
                }]
            }));
            ChangeKind::Add
        }
    };
    let mut content =
        serde_json::to_string_pretty(&root).expect("json value serializes");
    content.push('\n');
    fsio::write_verified(path, &content, snapshot.as_deref(), is_dry_run)?;
    Ok(vec![Change {
        tool: Tool::Codex,
        server: "UserPromptSubmit hook".to_string(),
        kind,
    }])
}

fn prompt_groups<'a>(root: &'a mut Value, path: &Path) -> Result<&'a mut Vec<Value>, SyncError> {
    let Value::Object(root_obj) = root else {
        return Err(not_shape(path, "top level is not an object"));
    };
    let hooks = root_obj
        .entry("hooks")
        .or_insert_with(|| Value::Object(Map::new()));
    let Value::Object(hooks_obj) = hooks else {
        return Err(not_shape(path, "hooks is not an object"));
    };
    let events = hooks_obj
        .entry("UserPromptSubmit")
        .or_insert_with(|| Value::Array(Vec::new()));
    let Value::Array(groups) = events else {
        return Err(not_shape(path, "hooks.UserPromptSubmit is not an array"));
    };
    Ok(groups)
}

fn not_shape(path: &Path, detail: &str) -> SyncError {
    SyncError::ParseJson(path.to_path_buf(), detail.to_string())
}

const MANAGED_COMMAND_SUFFIX: &str = "hooks/rag-recall";

fn managed_position(groups: &[Value]) -> Option<(usize, usize)> {
    groups.iter().enumerate().find_map(|(group_idx, group)| {
        let entries = group.get("hooks")?.as_array()?;
        entries
            .iter()
            .position(|entry| {
                entry
                    .get("command")
                    .and_then(Value::as_str)
                    .is_some_and(|command| command.ends_with(MANAGED_COMMAND_SUFFIX))
            })
            .map(|entry_idx| (group_idx, entry_idx))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn fixture(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "mcp-sync-hooks-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create fixture dir");
        dir
    }

    fn reg() -> HookRegistration {
        HookRegistration {
            command: "/Users/owaisquadri/Documents/agents/hooks/rag-recall".to_string(),
            timeout_secs: 10,
        }
    }

    fn pretty(value: &Value) -> String {
        let mut text = serde_json::to_string_pretty(value).expect("json value serializes");
        text.push('\n');
        text
    }

    fn managed_group() -> Value {
        json!({
            "hooks": [{
                "type": "command",
                "command": reg().command,
                "timeout": 10,
            }]
        })
    }

    #[test]
    fn absent_file_is_created_with_the_managed_entry() {
        let dir = fixture("absent");
        let path = dir.join("hooks.json");
        let changes = sync_codex_hook(&path, &reg(), false).expect("sync");
        assert_eq!(changes.len(), 1);
        assert!(matches!(changes[0].kind, ChangeKind::Add));
        assert!(matches!(changes[0].tool, Tool::Codex));
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            pretty(&json!({"hooks": {"UserPromptSubmit": [managed_group()]}}))
        );
    }

    #[test]
    fn second_run_reports_no_changes_and_writes_nothing() {
        let dir = fixture("idempotent");
        let path = dir.join("hooks.json");
        sync_codex_hook(&path, &reg(), false).expect("first sync");
        let before = fs::read_to_string(&path).unwrap();
        let changes = sync_codex_hook(&path, &reg(), false).expect("second sync");
        assert!(changes.is_empty());
        assert_eq!(fs::read_to_string(&path).unwrap(), before);
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
    }

    #[test]
    fn foreign_events_entries_and_keys_survive_the_add() {
        let dir = fixture("foreign");
        let path = dir.join("hooks.json");
        let mut doc = json!({
            "schema": 1,
            "hooks": {
                "SessionStart": [
                    {"hooks": [{"type": "command", "command": "/other/session-start", "timeout": 5}]}
                ],
                "UserPromptSubmit": [
                    {"matcher": "*", "hooks": [{"type": "command", "command": "/other/hook", "timeout": 3}]}
                ]
            }
        });
        fs::write(&path, pretty(&doc)).unwrap();
        let changes = sync_codex_hook(&path, &reg(), false).expect("sync");
        assert_eq!(changes.len(), 1);
        assert!(matches!(changes[0].kind, ChangeKind::Add));
        doc["hooks"]["UserPromptSubmit"]
            .as_array_mut()
            .unwrap()
            .push(managed_group());
        assert_eq!(fs::read_to_string(&path).unwrap(), pretty(&doc));
    }

    #[test]
    fn drifted_timeout_is_updated_in_place() {
        let dir = fixture("timeout");
        let path = dir.join("hooks.json");
        let mut doc = json!({
            "hooks": {
                "UserPromptSubmit": [
                    {"hooks": [
                        {"type": "command", "command": "/other/hook", "timeout": 3},
                        {"type": "command", "command": reg().command, "timeout": 99}
                    ]}
                ]
            }
        });
        fs::write(&path, pretty(&doc)).unwrap();
        let changes = sync_codex_hook(&path, &reg(), false).expect("sync");
        assert_eq!(changes.len(), 1);
        assert!(matches!(changes[0].kind, ChangeKind::Update));
        doc["hooks"]["UserPromptSubmit"][0]["hooks"][1]["timeout"] = json!(10);
        assert_eq!(fs::read_to_string(&path).unwrap(), pretty(&doc));
    }

    #[test]
    fn moved_repo_command_is_updated_in_place_not_appended() {
        let dir = fixture("moved");
        let path = dir.join("hooks.json");
        let mut doc = json!({
            "hooks": {
                "UserPromptSubmit": [
                    {"hooks": [
                        {"type": "command", "command": "/other/hook", "timeout": 3},
                        {"type": "command", "command": "/old/checkout/hooks/rag-recall", "timeout": 10}
                    ]}
                ]
            }
        });
        fs::write(&path, pretty(&doc)).unwrap();
        let changes = sync_codex_hook(&path, &reg(), false).expect("sync");
        assert_eq!(changes.len(), 1);
        assert!(matches!(changes[0].kind, ChangeKind::Update));
        doc["hooks"]["UserPromptSubmit"][0]["hooks"][1]["command"] = json!(reg().command);
        assert_eq!(fs::read_to_string(&path).unwrap(), pretty(&doc));
    }

    #[test]
    fn validate_accepts_absent_and_shaped_and_refuses_the_rest() {
        let dir = fixture("validate");
        let path = dir.join("hooks.json");
        assert!(validate(&path).is_ok());
        fs::write(&path, pretty(&json!({"hooks": {"UserPromptSubmit": []}}))).unwrap();
        assert!(validate(&path).is_ok());
        fs::write(&path, "{not json").unwrap();
        assert!(matches!(validate(&path), Err(SyncError::ParseJson(_, _))));
        fs::write(&path, pretty(&json!({"hooks": {"UserPromptSubmit": {}}}))).unwrap();
        assert!(matches!(validate(&path), Err(SyncError::ParseJson(_, _))));
    }

    #[test]
    fn invalid_json_is_refused_and_left_untouched() {
        let dir = fixture("invalid");
        let path = dir.join("hooks.json");
        fs::write(&path, "{not json").unwrap();
        let err = match sync_codex_hook(&path, &reg(), false) {
            Ok(_) => panic!("must refuse invalid json"),
            Err(err) => err,
        };
        assert!(matches!(err, SyncError::ParseJson(ref p, _) if p == &path));
        assert_eq!(fs::read_to_string(&path).unwrap(), "{not json");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
    }
}
