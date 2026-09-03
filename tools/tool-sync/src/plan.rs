use std::path::{Path, PathBuf};
use std::time::SystemTime;

use time::OffsetDateTime;

use crate::manifest::Platform;

pub(crate) fn pi_extension_backup(destination: &std::path::Path) -> PathBuf {
    let mut backup = destination.as_os_str().to_os_string();
    backup.push(".pre-tool-sync");
    backup.into()
}

pub(crate) fn foreign_checkout_aside(checkout: &Path, at: SystemTime) -> PathBuf {
    let stamp = OffsetDateTime::from(at);
    let mut aside = checkout.as_os_str().to_os_string();
    aside.push(format!(
        ".foreign-{:04}{:02}{:02}-{:02}{:02}{:02}",
        stamp.year(),
        u8::from(stamp.month()),
        stamp.day(),
        stamp.hour(),
        stamp.minute(),
        stamp.second()
    ));
    aside.into()
}

/// Holds installation actions in application order.
/// It takes no inputs beyond its action vector, returns data consumed by renderers
/// and appliers, and cannot fail.
#[derive(Debug, Default, Eq, PartialEq)]
pub struct Plan {
    pub actions: Vec<Action>,
}

/// Describes one data-only installation operation.
/// Each variant takes all paths or process arguments needed by an applier, returns
/// no value itself, and cannot fail until interpreted.
#[derive(Debug, Eq, PartialEq)]
pub enum Action {
    CreateDirectory {
        path: PathBuf,
    },
    CloneRepository {
        url: String,
        destination: PathBuf,
    },
    FetchRepository {
        repository: PathBuf,
    },
    RetireForeignCheckout {
        checkout: PathBuf,
        destination: PathBuf,
    },
    CheckoutRevision {
        repository: PathBuf,
        revision: String,
    },
    RunInstaller {
        tool: String,
        working_directory: PathBuf,
        command: String,
        args: Vec<String>,
        preview_args: Vec<String>,
    },
    LinkCommand {
        source: PathBuf,
        destination: PathBuf,
    },
    LinkPiExtension {
        source: PathBuf,
        source_root: PathBuf,
        destination: PathBuf,
        is_takeover_allowed: bool,
    },
    LinkPiPackage {
        source: PathBuf,
        destination: PathBuf,
    },
    LinkSkill {
        source: PathBuf,
        destination: PathBuf,
    },
    LinkHerdrPlugin {
        tool: String,
        source: PathBuf,
    },
    SkipPlatform {
        tool: String,
        platform: Platform,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_action_order() {
        let plan = Plan {
            actions: vec![
                Action::CreateDirectory {
                    path: PathBuf::from("/cache"),
                },
                Action::FetchRepository {
                    repository: PathBuf::from("/cache/rag"),
                },
                Action::RunInstaller {
                    tool: "rag".to_owned(),
                    working_directory: PathBuf::from("/cache/rag"),
                    command: "./install.sh".to_owned(),
                    args: Vec::new(),
                    preview_args: vec!["--dry-run".to_owned()],
                },
            ],
        };

        assert!(matches!(plan.actions[0], Action::CreateDirectory { .. }));
        assert!(matches!(plan.actions[1], Action::FetchRepository { .. }));
        assert!(matches!(plan.actions[2], Action::RunInstaller { .. }));
    }

    #[test]
    fn foreign_aside_names_the_checkout_with_a_utc_timestamp() {
        let checkout = Path::new("/cache/rag");

        assert_eq!(
            foreign_checkout_aside(
                checkout,
                SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000)
            ),
            PathBuf::from("/cache/rag.foreign-20231114-221320")
        );
        assert_eq!(
            foreign_checkout_aside(checkout, SystemTime::UNIX_EPOCH),
            PathBuf::from("/cache/rag.foreign-19700101-000000")
        );
    }
}
