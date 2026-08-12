use std::fmt;
use std::path::PathBuf;

pub enum SyncError {
    Io(PathBuf, std::io::Error),
    ParseToml(PathBuf, String),
    ParseJson(PathBuf, String),
    ManifestInvalid(String),
    BackupFailed(PathBuf),
    VerifyFailed(PathBuf, String),
    ChangedSinceRead(PathBuf),
}
impl fmt::Display for SyncError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SyncError::Io(path, err) => {
                write!(f, "cannot read or write {}: {}", path.display(), err)
            }
            SyncError::ParseToml(path, detail) => {
                write!(f, "{} is not valid TOML: {}", path.display(), detail)
            }
            SyncError::ParseJson(path, detail) => {
                write!(f, "{} is not valid JSON: {}", path.display(), detail)
            }
            SyncError::ManifestInvalid(detail) => write!(f, "manifest invalid: {}", detail),
            SyncError::BackupFailed(path) => {
                write!(f, "backup missing at {}; nothing was written", path.display())
            }
            SyncError::VerifyFailed(path, detail) => write!(
                f,
                "{} re-read differs after write: {}; restore the .pre-sync backup",
                path.display(),
                detail
            ),
            SyncError::ChangedSinceRead(path) => write!(
                f,
                "{} changed while mcp-sync ran; re-run mcp-sync",
                path.display()
            ),
        }
    }
}

impl fmt::Debug for SyncError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl std::error::Error for SyncError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_message_names_path_and_operation() {
        let inner = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
        let text = SyncError::Io(PathBuf::from("/tmp/x.toml"), inner).to_string();
        assert!(text.contains("/tmp/x.toml"), "{text}");
        assert!(text.contains("read or write"), "{text}");
        assert!(text.contains("denied"), "{text}");
    }

    #[test]
    fn parse_toml_message_names_path_and_detail() {
        let text =
            SyncError::ParseToml(PathBuf::from("/tmp/x.toml"), String::from("unclosed table"))
                .to_string();
        assert!(text.contains("/tmp/x.toml"), "{text}");
        assert!(text.contains("TOML"), "{text}");
        assert!(text.contains("unclosed table"), "{text}");
    }

    #[test]
    fn parse_json_message_names_path_and_detail() {
        let text = SyncError::ParseJson(PathBuf::from("/tmp/c.json"), String::from("EOF at line 3"))
            .to_string();
        assert!(text.contains("/tmp/c.json"), "{text}");
        assert!(text.contains("JSON"), "{text}");
        assert!(text.contains("EOF at line 3"), "{text}");
    }

    #[test]
    fn manifest_invalid_message_names_detail() {
        let text =
            SyncError::ManifestInvalid(String::from("server foo has both command and url"))
                .to_string();
        assert!(text.contains("manifest invalid"), "{text}");
        assert!(text.contains("server foo has both command and url"), "{text}");
    }

    #[test]
    fn backup_failed_message_names_path_and_recovery() {
        let text = SyncError::BackupFailed(PathBuf::from("/tmp/x.toml.pre-sync")).to_string();
        assert!(text.contains("/tmp/x.toml.pre-sync"), "{text}");
        assert!(text.contains("nothing was written"), "{text}");
    }

    #[test]
    fn verify_failed_message_names_path_and_recovery() {
        let text = SyncError::VerifyFailed(PathBuf::from("/tmp/c.json"), String::from("key order"))
            .to_string();
        assert!(text.contains("/tmp/c.json"), "{text}");
        assert!(text.contains("key order"), "{text}");
        assert!(text.contains("restore"), "{text}");
        assert!(text.contains(".pre-sync"), "{text}");
    }

    #[test]
    fn changed_since_read_message_names_path_and_recovery() {
        let text = SyncError::ChangedSinceRead(PathBuf::from("/tmp/x.toml")).to_string();
        assert!(text.contains("/tmp/x.toml"), "{text}");
        assert!(text.contains("re-run"), "{text}");
    }

    #[test]
    fn sync_error_is_std_error() {
        fn takes_error(_: &dyn std::error::Error) {}
        takes_error(&SyncError::ManifestInvalid(String::from("x")));
    }
}
