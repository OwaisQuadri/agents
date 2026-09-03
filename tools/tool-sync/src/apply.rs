use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::Command;

use crate::manifest::Platform;
use crate::plan::{pi_extension_backup, Action, Plan};
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
        Action::RetireForeignCheckout {
            checkout,
            destination,
        } => format!(
            "retire foreign checkout {} -> {}",
            checkout.display(),
            destination.display()
        ),
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
            is_takeover_allowed,
            ..
        } => format!(
            "link Pi extension {} -> {}{}",
            source.display(),
            destination.display(),
            if *is_takeover_allowed {
                " with managed-file takeover enabled"
            } else {
                ""
            }
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
    let is_absent = matches!(
        fs::metadata(working_directory),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound
    );
    (is_absent || is_retired_checkout(plan, working_directory))
        && plan.actions.iter().any(|action| {
            matches!(
                action,
                Action::CloneRepository { destination, .. } if destination == working_directory
            )
        })
}

fn is_retired_checkout(plan: &Plan, path: &Path) -> bool {
    plan.actions.iter().any(|action| {
        matches!(
            action,
            Action::RetireForeignCheckout { checkout, .. } if checkout == path
        )
    })
}

fn check_stale_state(plan: &Plan, is_dry_run: bool) -> Result<(), SyncError> {
    for action in &plan.actions {
        match action {
            Action::CreateDirectory { path } => require_missing(path)?,
            Action::CloneRepository { destination, .. } => {
                if !is_retired_checkout(plan, destination) {
                    require_missing(destination)?;
                }
            }
            Action::RetireForeignCheckout {
                checkout,
                destination,
            } => {
                let metadata = fs::metadata(checkout).map_err(|error| {
                    stale_or_io(checkout, "foreign checkout disappeared", error)
                })?;
                if !metadata.is_dir() {
                    return Err(SyncError::StaleState(
                        checkout.clone(),
                        "foreign checkout is no longer a directory".to_owned(),
                    ));
                }
                require_missing(destination)?;
            }
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
            Action::LinkPiExtension {
                destination,
                is_takeover_allowed,
                ..
            } => check_pi_extension_destination(destination, *is_takeover_allowed)?,
            Action::LinkCommand { destination, .. }
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

fn check_pi_extension_destination(
    destination: &Path,
    is_takeover_allowed: bool,
) -> Result<(), SyncError> {
    match fs::symlink_metadata(destination) {
        Ok(metadata) if metadata.file_type().is_symlink() => Ok(()),
        Ok(metadata) if metadata.is_file() && is_takeover_allowed => {
            require_missing(&pi_extension_backup(destination))
        }
        Ok(_) => Err(SyncError::DestinationCollision(destination.to_path_buf())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(SyncError::Io(destination.to_path_buf(), error)),
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
        Action::RetireForeignCheckout {
            checkout,
            destination,
        } => retire_foreign_checkout(checkout, destination),
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
        Action::LinkPiExtension {
            source,
            source_root,
            destination,
            is_takeover_allowed,
        } => link_pi_extension(source, source_root, destination, *is_takeover_allowed),
        Action::LinkCommand {
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

fn retire_foreign_checkout(checkout: &Path, destination: &Path) -> Result<(), SyncError> {
    fs::rename(checkout, destination)
        .map_err(|error| SyncError::Io(destination.to_path_buf(), error))?;
    fs::symlink_metadata(destination)
        .map_err(|error| stale_or_io(destination, "retired checkout is missing", error))?;
    match fs::symlink_metadata(checkout) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Ok(_) => Err(SyncError::StaleState(
            checkout.to_path_buf(),
            "foreign checkout survived its own retirement".to_owned(),
        )),
        Err(error) => Err(SyncError::Io(checkout.to_path_buf(), error)),
    }
}

fn require_clean_repository(repository: &Path) -> Result<(), SyncError> {
    // A git hook exports GIT_DIR and friends, which override -C and would aim
    // this check at the hook's own repository instead of the checkout.
    let output = Command::new("git")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_PREFIX")
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
    // A git hook exports GIT_DIR and friends, which override -C and would aim
    // this command at the hook's own repository instead of the checkout.
    let status = Command::new("git")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_PREFIX")
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

fn link_pi_extension(
    source: &Path,
    source_root: &Path,
    destination: &Path,
    is_takeover_allowed: bool,
) -> Result<(), SyncError> {
    verify_source_inside(source, source_root)?;
    let is_regular_file = fs::symlink_metadata(destination)
        .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink());
    if !is_regular_file {
        create_verified_link(source, destination)?;
        return verify_source_inside(destination, source_root);
    }
    if !is_takeover_allowed {
        return Err(SyncError::DestinationCollision(destination.to_path_buf()));
    }

    let backup = pi_extension_backup(destination);
    let (snapshot, identity) = create_verified_backup(destination, &backup)?;
    verify_unchanged(destination, &snapshot, identity)?;
    verify_source_inside(source, source_root)?;
    atomic_replace_with_link(source, source_root, destination)
}

fn create_verified_backup(
    source: &Path,
    backup: &Path,
) -> Result<(Vec<u8>, (u64, u64)), SyncError> {
    let identity = file_identity(source)?;
    let temporary_backup = backup.with_extension(format!("tmp.{}", std::process::id()));
    let snapshot = copy_new(source, &temporary_backup)?;
    if let Err(error) = fs::hard_link(&temporary_backup, backup) {
        let _ = fs::remove_file(&temporary_backup);
        return Err(SyncError::Io(backup.to_path_buf(), error));
    }
    let observed = fs::read(backup).map_err(|error| SyncError::Io(backup.to_path_buf(), error))?;
    if observed != snapshot {
        return Err(SyncError::StaleState(
            backup.to_path_buf(),
            "backup does not match its source".to_owned(),
        ));
    }
    fs::remove_file(&temporary_backup).map_err(|error| SyncError::Io(temporary_backup, error))?;
    Ok((snapshot, identity))
}

fn verify_unchanged(
    source: &Path,
    snapshot: &[u8],
    expected_identity: (u64, u64),
) -> Result<(), SyncError> {
    let current = fs::read(source).map_err(|error| SyncError::Io(source.to_path_buf(), error))?;
    if current != snapshot || file_identity(source)? != expected_identity {
        return Err(SyncError::StaleState(
            source.to_path_buf(),
            "managed file changed while it was backed up".to_owned(),
        ));
    }
    Ok(())
}

fn file_identity(path: &Path) -> Result<(u64, u64), SyncError> {
    use std::os::unix::fs::MetadataExt;

    let metadata = fs::metadata(path).map_err(|error| SyncError::Io(path.to_path_buf(), error))?;
    Ok((metadata.dev(), metadata.ino()))
}

fn atomic_replace_with_link(
    source: &Path,
    source_root: &Path,
    destination: &Path,
) -> Result<(), SyncError> {
    let temporary_link = destination.with_extension(format!("link.{}", std::process::id()));
    std::os::unix::fs::symlink(source, &temporary_link)
        .map_err(|error| SyncError::Io(temporary_link.clone(), error))?;
    if let Err(error) = verify_source_inside(&temporary_link, source_root) {
        let _ = fs::remove_file(&temporary_link);
        return Err(error);
    }
    fs::rename(&temporary_link, destination)
        .map_err(|error| SyncError::Io(destination.to_path_buf(), error))?;
    verify_source_inside(destination, source_root)
}

fn copy_new(source: &Path, destination: &Path) -> Result<Vec<u8>, SyncError> {
    let contents = fs::read(source).map_err(|error| SyncError::Io(source.to_path_buf(), error))?;
    let permissions =
        fs::metadata(source).map_err(|error| SyncError::Io(source.to_path_buf(), error))?;
    let mut destination_file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|error| SyncError::Io(destination.to_path_buf(), error))?;
    let copy_result = (|| {
        destination_file
            .write_all(&contents)
            .map_err(|error| SyncError::Io(destination.to_path_buf(), error))?;
        destination_file
            .sync_all()
            .map_err(|error| SyncError::Io(destination.to_path_buf(), error))?;
        fs::set_permissions(destination, permissions.permissions())
            .map_err(|error| SyncError::Io(destination.to_path_buf(), error))?;
        let observed = fs::read(destination)
            .map_err(|error| SyncError::Io(destination.to_path_buf(), error))?;
        if observed != contents {
            return Err(SyncError::StaleState(
                destination.to_path_buf(),
                "copied file does not match its source".to_owned(),
            ));
        }
        Ok(contents)
    })();
    if copy_result.is_err() {
        let _ = fs::remove_file(destination);
    }
    copy_result
}

fn verify_source_inside(source: &Path, source_root: &Path) -> Result<(), SyncError> {
    let resolved_source =
        fs::canonicalize(source).map_err(|error| SyncError::Io(source.to_path_buf(), error))?;
    let resolved_root = fs::canonicalize(source_root)
        .map_err(|error| SyncError::Io(source_root.to_path_buf(), error))?;
    if !resolved_source.starts_with(&resolved_root) {
        return Err(SyncError::StaleState(
            source.to_path_buf(),
            format!(
                "source resolves outside {} to {}",
                resolved_root.display(),
                resolved_source.display()
            ),
        ));
    }
    Ok(())
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
    fn retires_a_foreign_checkout_and_keeps_its_contents() {
        let fixture = Fixture::new();
        let checkout = fixture.0.join("cache/rag");
        fs::create_dir_all(&checkout).expect("foreign checkout");
        fs::write(checkout.join("revision"), "someone else's work").expect("foreign file");
        let aside = fixture.0.join("cache/rag.foreign-20231114-221320");
        let plan = Plan {
            actions: vec![Action::RetireForeignCheckout {
                checkout: checkout.clone(),
                destination: aside.clone(),
            }],
        };

        run(&plan, false).expect("retire succeeds");

        assert_eq!(
            fs::read_to_string(aside.join("revision")).expect("retired file"),
            "someone else's work"
        );
        assert!(!checkout.exists());
    }

    #[test]
    fn refuses_to_retire_onto_an_occupied_destination() {
        let fixture = Fixture::new();
        let checkout = fixture.0.join("cache/rag");
        fs::create_dir_all(&checkout).expect("foreign checkout");
        fs::write(checkout.join("revision"), "someone else's work").expect("foreign file");
        let aside = fixture.0.join("cache/rag.foreign-20231114-221320");
        fs::create_dir_all(&aside).expect("occupied aside");
        let plan = Plan {
            actions: vec![Action::RetireForeignCheckout {
                checkout: checkout.clone(),
                destination: aside.clone(),
            }],
        };

        let error = run(&plan, false).expect_err("occupied aside must fail");

        assert!(matches!(error, SyncError::StaleState(_, _)));
        assert_eq!(
            fs::read_to_string(checkout.join("revision")).expect("untouched foreign file"),
            "someone else's work"
        );
        assert!(fs::read_dir(&aside)
            .expect("aside entries")
            .next()
            .is_none());
    }

    #[test]
    fn dry_run_neither_retires_nor_previews_a_foreign_checkout() {
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
        let checkout = fixture.0.join("cache/rag");
        fs::create_dir_all(&checkout).expect("foreign checkout");
        fs::write(checkout.join("revision"), "someone else's work").expect("foreign file");
        let aside = fixture.0.join("cache/rag.foreign-20231114-221320");
        let plan = Plan {
            actions: vec![
                Action::RetireForeignCheckout {
                    checkout: checkout.clone(),
                    destination: aside.clone(),
                },
                Action::CloneRepository {
                    url: "https://example.test/rag.git".into(),
                    destination: checkout.clone(),
                },
                Action::RunInstaller {
                    tool: "rag".into(),
                    working_directory: checkout.clone(),
                    command: script.to_string_lossy().into_owned(),
                    args: vec!["apply".into()],
                    preview_args: vec!["preview".into()],
                },
            ],
        };

        run(&plan, true).expect("a retiring plan is only rendered");

        assert_eq!(
            fs::read_to_string(checkout.join("revision")).expect("untouched foreign file"),
            "someone else's work"
        );
        assert!(!aside.exists(), "dry run created the aside");
        assert!(!marker.exists(), "installer child was invoked");
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
    fn refuses_a_pi_extension_symlink_that_escapes_its_source() {
        let fixture = Fixture::new();
        let source_root = fixture.0.join("checkout");
        let destination_root = fixture.0.join("home/.pi/agent/extensions");
        fs::create_dir_all(&source_root).expect("source root");
        let outside = fixture.0.join("outside.ts");
        fs::write(&outside, "outside").expect("outside source");
        let source = source_root.join("herdr-agent-state.ts");
        std::os::unix::fs::symlink(&outside, &source).expect("escaping source link");
        let destination = destination_root.join("herdr-agent-state.ts");
        let plan = Plan {
            actions: vec![
                Action::CreateDirectory {
                    path: destination_root,
                },
                Action::LinkPiExtension {
                    source: source.clone(),
                    source_root,
                    destination: destination.clone(),
                    is_takeover_allowed: false,
                },
            ],
        };

        let error = run(&plan, false).expect_err("escaping source must fail");

        assert!(error.to_string().contains("source resolves outside"));
        assert!(!destination.exists());
    }

    #[test]
    fn backs_up_an_allowed_pi_extension_takeover() {
        let fixture = Fixture::new();
        let source_root = fixture.0.join("checkout");
        let destination_root = fixture.0.join("home/.pi/agent/extensions");
        fs::create_dir_all(&source_root).expect("source root");
        fs::create_dir_all(&destination_root).expect("destination root");
        let source = source_root.join("herdr-agent-state.ts");
        fs::write(&source, "version 9").expect("source");
        let destination = destination_root.join("herdr-agent-state.ts");
        fs::write(&destination, "version 8").expect("existing extension");
        let plan = Plan {
            actions: vec![Action::LinkPiExtension {
                source: source.clone(),
                source_root,
                destination: destination.clone(),
                is_takeover_allowed: true,
            }],
        };

        run(&plan, false).expect("takeover succeeds");

        assert_eq!(
            fs::read_to_string(pi_extension_backup(&destination)).unwrap(),
            "version 8"
        );
        assert_eq!(fs::read_link(destination).unwrap(), source);
    }

    #[test]
    fn refuses_to_overwrite_a_takeover_backup_created_after_preflight() {
        let fixture = Fixture::new();
        let source_root = fixture.0.join("checkout");
        fs::create_dir_all(&source_root).expect("source root");
        let source = source_root.join("herdr-agent-state.ts");
        fs::write(&source, "version 9").expect("source");
        let destination = fixture.0.join("herdr-agent-state.ts");
        fs::write(&destination, "version 8").expect("existing extension");
        let backup = pi_extension_backup(&destination);
        fs::write(&backup, "protected backup").expect("racing backup");

        let error = link_pi_extension(&source, &source_root, &destination, true)
            .expect_err("existing backup must block takeover");

        assert!(matches!(error, SyncError::Io(_, _)));
        assert_eq!(fs::read_to_string(destination).unwrap(), "version 8");
        assert_eq!(fs::read_to_string(backup).unwrap(), "protected backup");
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
