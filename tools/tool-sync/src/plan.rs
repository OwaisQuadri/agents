use std::path::PathBuf;

use crate::manifest::Platform;

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
        destination: PathBuf,
    },
    // TODO(AGNT-0012.T02): Plan source-relative package and skill links.
    LinkPiPackage {
        source: PathBuf,
        destination: PathBuf,
    },
    LinkSkill {
        source: PathBuf,
        destination: PathBuf,
    },
    RenderPiAgent {
        source: PathBuf,
        destination: PathBuf,
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
}
