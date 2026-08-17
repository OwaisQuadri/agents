use std::fs;
use std::path::Path;
use std::process::Command;

use crate::manifest::Platform;
use crate::plan::{Action, Plan};
use crate::SyncError;

/// Renders a plan as one stable line per action in application order.
/// It takes the plan and whether installer preview arguments are selected and returns the rendered text.
/// This operation cannot fail.
pub fn render(plan: &Plan, is_dry_run: bool) -> String {
    plan.actions
        .iter()
        .map(|action| render_action(action, is_dry_run))
        .collect::<Vec<_>>()
        .join("\n")
}

// TODO(AGNT-0012.T03): Render package and skill link actions.
// TODO(AGNT-0012.T07): Render derived-agent actions.
fn render_action(action: &Action, is_dry_run: bool) -> String {
    match action {
        Action::CreateDirectory { path } => format!("create directory {}", path.display()),
        Action::CloneRepository { url, destination } => {
            format!("clone {url} into {}", destination.display())
        }
        Action::FetchRepository { repository } => {
            format!("fetch repository {}", repository.display())
        }
        Action::CheckoutRevision {
            repository,
            revision,
        } => format!("checkout {revision} in {}", repository.display()),
        Action::RunInstaller {
            tool,
            working_directory,
            command,
            args,
            preview_args,
        } => {
            let selected_args = if is_dry_run { preview_args } else { args };
            format!(
                "install {tool} in {}: {command} {selected_args:?}",
                working_directory.display()
            )
        }
        Action::LinkCommand {
            source,
            destination,
        } => format!(
            "link command {} -> {}",
            source.display(),
            destination.display()
        ),
        Action::LinkPiExtension {
            source,
            destination,
        } => format!(
            "link Pi extension {} -> {}",
            source.display(),
            destination.display()
        ),
        Action::SkipPlatform { tool, platform } => {
            format!("skip {tool} on {}", platform_name(*platform))
        }
    }
}

fn platform_name(platform: Platform) -> &'static str {
    match platform {
        Platform::Macos => "macos",
        Platform::Linux => "linux",
    }
}

/// Applies a validated plan, or invokes only installer preview commands for a dry run.
/// It takes the plan and a dry-run selection and returns unit after every selected action succeeds.
///
/// # Errors
///
/// Returns `SyncError` at the first stale state, collision, filesystem failure, Git failure,
/// installer launch failure, or unsuccessful installer exit.
pub fn run(plan: &Plan, is_dry_run: bool) -> Result<(), SyncError> {
    check_stale_state(plan, is_dry_run)?;

    for action in &plan.actions {
        if is_dry_run {
            if let Action::RunInstaller {
                tool,
                working_directory,
                command,
                preview_args,
                ..
            } = action
            {
                if !is_unfetched_git_source(plan, working_directory) {
                    run_installer(tool, working_directory, command, preview_args)?;
                }
            }
            continue;
        }

        apply_action(action)?;
    }
    Ok(())
}

fn is_unfetched_git_source(plan: &Plan, working_directory: &Path) -> bool {
    matches!(
        fs::metadata(working_directory),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound
    ) && plan.actions.iter().any(|action| {
        matches!(
            action,
            Action::CloneRepository { destination, .. } if destination == working_directory
        )
    })
}

fn check_stale_state(plan: &Plan, is_dry_run: bool) -> Result<(), SyncError> {
    for action in &plan.actions {
        match action {
            Action::CreateDirectory { path } => require_missing(path)?,
            Action::CloneRepository { destination, .. } => require_missing(destination)?,
            Action::FetchRepository { repository } => {
                let metadata = fs::metadata(repository)
                    .map_err(|error| stale_or_io(repository, "repository disappeared", error))?;
                if !metadata.is_dir() {
                    return Err(SyncError::StaleState(
                        repository.clone(),
                        "repository is no longer a directory".to_owned(),
                    ));
                }
                if !is_dry_run {
                    require_clean_repository(repository)?;
                }
            }
            Action::LinkCommand { destination, .. }
            | Action::LinkPiExtension { destination, .. } => check_link_destination(destination)?,
            Action::CheckoutRevision { .. }
            | Action::RunInstaller { .. }
            | Action::SkipPlatform { .. } => {}
        }
    }
    Ok(())
}

fn require_missing(path: &Path) -> Result<(), SyncError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(SyncError::StaleState(
            path.to_path_buf(),
            "destination appeared after planning".to_owned(),
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(SyncError::Io(path.to_path_buf(), error)),
    }
}

fn stale_or_io(path: &Path, detail: &str, error: std::io::Error) -> SyncError {
    if error.kind() == std::io::ErrorKind::NotFound {
        SyncError::StaleState(path.to_path_buf(), detail.to_owned())
    } else {
        SyncError::Io(path.to_path_buf(), error)
    }
}

fn check_link_destination(destination: &Path) -> Result<(), SyncError> {
    match fs::symlink_metadata(destination) {
        Ok(metadata) if metadata.file_type().is_symlink() => Ok(()),
        Ok(_) => Err(SyncError::DestinationCollision(destination.to_path_buf())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(SyncError::Io(destination.to_path_buf(), error)),
    }
}

// TODO(AGNT-0012.T03): Apply package and skill links through verified linking.
// TODO(AGNT-0012.T07): Dispatch derived-agent rendering.
fn apply_action(action: &Action) -> Result<(), SyncError> {
    match action {
        Action::CreateDirectory { path } => {
            fs::create_dir_all(path).map_err(|error| SyncError::Io(path.clone(), error))
        }
        Action::CloneRepository { url, destination } => {
            let parent = destination.parent().unwrap_or_else(|| Path::new("."));
            run_git(
                parent,
                &["clone".into(), "--".into(), url.into(), destination.into()],
            )
        }
        Action::FetchRepository { repository } => git(repository, &["fetch", "--all", "--prune"]),
        Action::CheckoutRevision {
            repository,
            revision,
        } => git(repository, &["checkout", "--detach", revision]),
        Action::RunInstaller {
            tool,
            working_directory,
            command,
            args,
            ..
        } => run_installer(tool, working_directory, command, args),
        Action::LinkCommand {
            source,
            destination,
        }
        | Action::LinkPiExtension {
            source,
            destination,
        } => create_verified_link(source, destination),
        Action::SkipPlatform { .. } => Ok(()),
    }
}

fn require_clean_repository(repository: &Path) -> Result<(), SyncError> {
    let output = Command::new("git")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .arg("-C")
        .arg(repository)
        .args(["status", "--porcelain", "--untracked-files=all"])
        .output()
        .map_err(|error| SyncError::ProcessStart {
            program: "git".to_owned(),
            working_directory: repository.to_path_buf(),
            error,
        })?;
    if !output.status.success() {
        return Err(SyncError::GitFailed {
            repository: repository.to_path_buf(),
            status: output.status,
        });
    }
    if !output.stdout.is_empty() {
        return Err(SyncError::StaleState(
            repository.to_path_buf(),
            "repository became dirty after planning".to_owned(),
        ));
    }
    Ok(())
}

fn git(repository: &Path, args: &[&str]) -> Result<(), SyncError> {
    let mut owned = vec!["-C".into(), repository.as_os_str().into()];
    owned.extend(args.iter().map(Into::into));
    run_git(repository, &owned)
}

fn run_git(repository: &Path, args: &[std::ffi::OsString]) -> Result<(), SyncError> {
    let status = Command::new("git")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .args(args)
        .status()
        .map_err(|error| SyncError::ProcessStart {
            program: "git".to_owned(),
            working_directory: repository.to_path_buf(),
            error,
        })?;
    if status.success() {
        Ok(())
    } else {
        Err(SyncError::GitFailed {
            repository: repository.to_path_buf(),
            status,
        })
    }
}

fn run_installer(
    tool: &str,
    working_directory: &Path,
    command: &str,
    args: &[String],
) -> Result<(), SyncError> {
    let status = Command::new(command)
        .current_dir(working_directory)
        .args(args)
        .status()
        .map_err(|error| SyncError::ProcessStart {
            program: command.to_owned(),
            working_directory: working_directory.to_path_buf(),
            error,
        })?;
    if status.success() {
        Ok(())
    } else {
        Err(SyncError::InstallerFailed {
            tool: tool.to_owned(),
            status,
        })
    }
}

fn create_verified_link(source: &Path, destination: &Path) -> Result<(), SyncError> {
    fs::metadata(source).map_err(|error| SyncError::Io(source.to_path_buf(), error))?;
    match fs::symlink_metadata(destination) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            if fs::read_link(destination)
                .map_err(|error| SyncError::Io(destination.into(), error))?
                == source
            {
                return Ok(());
            }
            fs::remove_file(destination)
                .map_err(|error| SyncError::Io(destination.to_path_buf(), error))?;
        }
        Ok(_) => return Err(SyncError::DestinationCollision(destination.to_path_buf())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(SyncError::Io(destination.to_path_buf(), error)),
    }

    std::os::unix::fs::symlink(source, destination)
        .map_err(|error| SyncError::Io(destination.to_path_buf(), error))?;
    let observed =
        fs::read_link(destination).map_err(|error| SyncError::Io(destination.into(), error))?;
    if observed != source {
        return Err(SyncError::StaleState(
            destination.to_path_buf(),
            format!(
                "link resolves to {} instead of {}",
                observed.display(),
                source.display()
            ),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_ID: AtomicU64 = AtomicU64::new(0);

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("tool-sync-apply-{}-{id}", std::process::id()));
            fs::create_dir_all(&path).expect("fixture root");
            Self(path)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn renders_actions_in_order_with_selected_installer_arguments() {
        let plan = Plan {
            actions: vec![
                Action::CreateDirectory {
                    path: "/cache".into(),
                },
                Action::RunInstaller {
                    tool: "rag".into(),
                    working_directory: "/source".into(),
                    command: "./install".into(),
                    args: vec!["apply argument".into()],
                    preview_args: vec!["--preview".into()],
                },
            ],
        };

        assert_eq!(
            render(&plan, true),
            "create directory /cache\ninstall rag in /source: ./install [\"--preview\"]"
        );
    }

    #[test]
    fn dry_run_only_executes_preview_arguments_and_creates_nothing() {
        let fixture = Fixture::new();
        let script = fixture.0.join("preview.sh");
        fs::write(
            &script,
            "#!/bin/sh\ntest \"$#\" = 1 && test \"$1\" = '--preview argument'\n",
        )
        .expect("script");
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("executable");
        let uncreated = fixture.0.join("uncreated");
        let plan = Plan {
            actions: vec![
                Action::CreateDirectory {
                    path: uncreated.clone(),
                },
                Action::RunInstaller {
                    tool: "rag".into(),
                    working_directory: fixture.0.clone(),
                    command: script.to_string_lossy().into_owned(),
                    args: vec!["apply argument".into()],
                    preview_args: vec!["--preview argument".into()],
                },
            ],
        };

        run(&plan, true).expect("preview succeeds");

        assert!(!uncreated.exists());
    }

    #[test]
    fn dry_run_does_not_invoke_installer_for_unfetched_git_source() {
        let fixture = Fixture::new();
        let marker = fixture.0.join("installer-ran");
        let script = fixture.0.join("preview.sh");
        fs::write(
            &script,
            format!("#!/bin/sh\ntouch {:?}\n", marker.display().to_string()),
        )
        .expect("script");
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("executable");
        let cache = fixture.0.join("cache");
        let checkout = cache.join("rag");
        let plan = Plan {
            actions: vec![
                Action::CreateDirectory { path: cache },
                Action::CloneRepository {
                    url: "https://example.test/rag.git".into(),
                    destination: checkout.clone(),
                },
                Action::RunInstaller {
                    tool: "rag".into(),
                    working_directory: checkout,
                    command: script.to_string_lossy().into_owned(),
                    args: vec!["apply".into()],
                    preview_args: vec!["preview".into()],
                },
            ],
        };

        run(&plan, true).expect("missing fresh source is only rendered");

        assert!(!marker.exists(), "installer child was invoked");
        assert!(!fixture.0.join("cache").exists(), "dry run wrote cache");
    }

    #[test]
    fn stale_collision_is_found_before_any_write() {
        let fixture = Fixture::new();
        let uncreated = fixture.0.join("first");
        let collision = fixture.0.join("command");
        fs::write(&collision, "owned").expect("collision");
        let plan = Plan {
            actions: vec![
                Action::CreateDirectory {
                    path: uncreated.clone(),
                },
                Action::LinkCommand {
                    source: fixture.0.join("source"),
                    destination: collision,
                },
            ],
        };

        assert!(matches!(
            run(&plan, false),
            Err(SyncError::DestinationCollision(_))
        ));
        assert!(!uncreated.exists());
    }

    #[test]
    fn checkout_revision_detaches_at_exact_pinned_commit_with_real_git() {
        let fixture = Fixture::new();
        let repository = &fixture.0;
        let git = |args: &[&str]| {
            Command::new("git")
                .arg("-C")
                .arg(repository)
                .args(args)
                .output()
                .expect("real local Git starts")
        };
        assert!(git(&["init", "--quiet"]).status.success());
        assert!(git(&["config", "user.name", "Tool Sync Test"])
            .status
            .success());
        assert!(git(&["config", "user.email", "tool-sync@example.test"])
            .status
            .success());

        let tracked = repository.join("tracked");
        fs::write(&tracked, "pinned\n").expect("first version");
        assert!(git(&["add", "tracked"]).status.success());
        assert!(git(&["commit", "--quiet", "-m", "pinned"]).status.success());
        let pinned_output = git(&["rev-parse", "HEAD"]);
        assert!(pinned_output.status.success());
        let pinned = String::from_utf8(pinned_output.stdout)
            .expect("commit ID is UTF-8")
            .trim()
            .to_owned();

        fs::write(&tracked, "newer\n").expect("second version");
        assert!(git(&["commit", "--quiet", "-am", "newer"]).status.success());
        let newer_output = git(&["rev-parse", "HEAD"]);
        assert!(newer_output.status.success());
        assert_ne!(pinned.as_bytes(), newer_output.stdout.trim_ascii());

        let plan = Plan {
            actions: vec![Action::CheckoutRevision {
                repository: repository.clone(),
                revision: pinned.clone(),
            }],
        };
        run(&plan, false).expect("checkout succeeds");

        let actual_output = git(&["rev-parse", "HEAD"]);
        assert!(actual_output.status.success());
        assert_eq!(pinned.as_bytes(), actual_output.stdout.trim_ascii());
        assert_eq!(
            fs::read_to_string(tracked).expect("checked-out file"),
            "pinned\n"
        );
        assert!(
            !git(&["symbolic-ref", "--quiet", "HEAD"]).status.success(),
            "checkout must leave HEAD detached"
        );
    }

    #[test]
    fn apply_creates_and_verifies_link() {
        let fixture = Fixture::new();
        let source = fixture.0.join("source");
        fs::write(&source, "executable").expect("source");
        let directory = fixture.0.join("bin");
        let destination = directory.join("rag");
        let plan = Plan {
            actions: vec![
                Action::CreateDirectory { path: directory },
                Action::LinkCommand {
                    source: source.clone(),
                    destination: destination.clone(),
                },
            ],
        };

        run(&plan, false).expect("apply succeeds");

        assert_eq!(fs::read_link(destination).expect("link target"), source);
    }
}
