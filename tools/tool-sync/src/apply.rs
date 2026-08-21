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
        Action::LinkPiPackage {
            source,
            destination,
        } => format!(
            "link Pi package {} -> {}",
            source.display(),
            destination.display()
        ),
        Action::LinkSkill {
            source,
            destination,
        } => format!(
            "link skill {} -> {}",
            source.display(),
            destination.display()
        ),
        Action::LinkHerdrPlugin { tool, source } => {
            format!("link herdr plugin {} for {tool}", source.display())
        }
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
            | Action::LinkPiExtension { destination, .. }
            | Action::LinkPiPackage { destination, .. }
            | Action::LinkSkill { destination, .. } => check_link_destination(destination)?,
            Action::CheckoutRevision { .. }
            | Action::RunInstaller { .. }
            | Action::LinkHerdrPlugin { .. }
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
        }
        | Action::LinkPiPackage {
            source,
            destination,
        }
        | Action::LinkSkill {
            source,
            destination,
        } => create_verified_link(source, destination),
        Action::LinkHerdrPlugin { tool, source } => link_herdr_plugin(tool, source),
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

fn link_herdr_plugin(tool: &str, source: &Path) -> Result<(), SyncError> {
    let status = Command::new("herdr")
        .args(["plugin", "link"])
        .arg(source)
        .status()
        .map_err(|error| SyncError::ProcessStart {
            program: "herdr".to_owned(),
            working_directory: source.to_path_buf(),
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
    fn renders_package_and_skill_actions_in_order() {
        let plan = Plan {
            actions: vec![
                Action::LinkPiPackage {
                    source: "/source/package".into(),
                    destination: "/root/package".into(),
                },
                Action::LinkSkill {
                    source: "/source/skill".into(),
                    destination: "/shared/skill".into(),
                },
            ],
        };

        assert_eq!(
            render(&plan, true),
            "link Pi package /source/package -> /root/package\nlink skill /source/skill -> /shared/skill"
        );
    }

    #[test]
    fn renders_herdr_plugin_link_naming_tool_and_source() {
        let plan = Plan {
            actions: vec![Action::LinkHerdrPlugin {
                tool: "herdr-worktree-layout".into(),
                source: "/repo/herdr/worktree-layout".into(),
            }],
        };

        assert_eq!(
            render(&plan, true),
            "link herdr plugin /repo/herdr/worktree-layout for herdr-worktree-layout"
        );
        assert_eq!(render(&plan, false), render(&plan, true));
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
    fn dry_run_does_not_write_package_or_skill_links() {
        let fixture = Fixture::new();
        let package_source = fixture.0.join("package");
        let skill_source = fixture.0.join("skill");
        fs::create_dir_all(&package_source).expect("package source");
        fs::create_dir_all(&skill_source).expect("skill source");
        let package_root = fixture.0.join("home/.pi/agent/extensions");
        let skill_root = fixture.0.join("home/.agents/skills");
        let package_destination = package_root.join("package");
        let skill_destination = skill_root.join("skill");
        let plan = Plan {
            actions: vec![
                Action::CreateDirectory { path: package_root },
                Action::LinkPiPackage {
                    source: package_source,
                    destination: package_destination,
                },
                Action::CreateDirectory { path: skill_root },
                Action::LinkSkill {
                    source: skill_source,
                    destination: skill_destination,
                },
            ],
        };

        run(&plan, true).expect("dry run succeeds");

        assert!(!fixture.0.join("home/.pi/agent/extensions").exists());
        assert!(!fixture.0.join("home/.agents/skills").exists());
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
    fn stale_package_collision_is_found_before_any_write() {
        let fixture = Fixture::new();
        let uncreated = fixture.0.join("first");
        let collision = fixture.0.join("package");
        fs::write(&collision, "owned").expect("collision");
        let plan = Plan {
            actions: vec![
                Action::CreateDirectory {
                    path: uncreated.clone(),
                },
                Action::LinkPiPackage {
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

    #[test]
    fn apply_creates_and_verifies_package_and_skill_links() {
        let fixture = Fixture::new();
        let package_source = fixture.0.join("package");
        let skill_source = fixture.0.join("skill");
        fs::create_dir_all(&package_source).expect("package source");
        fs::create_dir_all(&skill_source).expect("skill source");
        let package_root = fixture.0.join("home/.pi/agent/extensions");
        let skill_root = fixture.0.join("home/.agents/skills");
        let package_destination = package_root.join("package");
        let skill_destination = skill_root.join("skill");
        let plan = Plan {
            actions: vec![
                Action::CreateDirectory {
                    path: package_root.clone(),
                },
                Action::LinkPiPackage {
                    source: package_source.clone(),
                    destination: package_destination.clone(),
                },
                Action::CreateDirectory {
                    path: skill_root.clone(),
                },
                Action::LinkSkill {
                    source: skill_source.clone(),
                    destination: skill_destination.clone(),
                },
            ],
        };

        run(&plan, false).expect("apply succeeds");

        assert_eq!(
            fs::read_link(package_destination).expect("package link"),
            package_source
        );
        assert_eq!(
            fs::read_link(skill_destination).expect("skill link"),
            skill_source
        );
    }
}
