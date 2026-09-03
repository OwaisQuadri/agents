use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::manifest::{Platform, ToolManifest, ToolSource};
use crate::plan::{pi_extension_backup, Action, Plan};
use crate::SyncError;

/// Supplies absolute roots and the selected platform to plan construction.
/// It takes repository, home, and managed-cache roots plus a platform, returns
/// planning context data, and cannot fail.
#[derive(Debug, Eq, PartialEq)]
pub struct Context {
    pub repository_root: PathBuf,
    pub home_root: PathBuf,
    pub cache_root: PathBuf,
    pub platform: Platform,
}

/// Builds an ordered, data-only installation plan from a validated manifest.
/// It takes a manifest and absolute planning context, returns actions for the
/// selected platform, and errors on unsafe paths, checkout state, or collisions.
pub fn build(manifest: &ToolManifest, context: &Context) -> Result<Plan, SyncError> {
    let mut actions = Vec::new();
    let mut planned_paths = HashSet::new();

    for tool in &manifest.tools {
        if !tool.platforms.contains(&context.platform) {
            actions.push(Action::SkipPlatform {
                tool: tool.name.clone(),
                platform: context.platform,
            });
            continue;
        }

        let (working_directory, is_checkout_present) = match &tool.source {
            ToolSource::Embedded { path } => (
                resolve_inside(&context.repository_root, path, "embedded source")?,
                true,
            ),
            ToolSource::Git { url, revision } => {
                let checkout = context.cache_root.join(&tool.name);
                let is_checkout_present = match fs::symlink_metadata(&checkout) {
                    Ok(_) => {
                        inspect_checkout(&checkout, url)?;
                        actions.push(Action::FetchRepository {
                            repository: checkout.clone(),
                        });
                        true
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        add_directory(&context.cache_root, &mut planned_paths, &mut actions)?;
                        actions.push(Action::CloneRepository {
                            url: url.clone(),
                            destination: checkout.clone(),
                        });
                        false
                    }
                    Err(error) => return Err(SyncError::Io(checkout, error)),
                };
                actions.push(Action::CheckoutRevision {
                    repository: checkout.clone(),
                    revision: revision.clone(),
                });
                (checkout, is_checkout_present)
            }
        };

        actions.push(Action::RunInstaller {
            tool: tool.name.clone(),
            working_directory: working_directory.clone(),
            command: tool.installer.command.clone(),
            args: tool.installer.args.clone(),
            preview_args: tool.installer.preview_args.clone(),
        });

        let command_directory = context.home_root.join(".local/bin");
        for command in &tool.commands {
            let destination = command_directory.join(
                command
                    .file_name()
                    .expect("validated command paths have file names"),
            );
            reject_non_symlink(&destination, "command")?;
            add_directory(&command_directory, &mut planned_paths, &mut actions)?;
            reserve_destination(&destination, &mut planned_paths, "command")?;
            actions.push(Action::LinkCommand {
                source: working_directory.join(command),
                destination,
            });
        }

        if tool.pi_extension.is_some() || tool.pi_package.is_some() {
            let extension_directory = context.home_root.join(".pi/agent/extensions");
            add_directory(&extension_directory, &mut planned_paths, &mut actions)?;

            if let Some(extension) = &tool.pi_extension {
                let (source, source_root) = if tool.is_pi_extension_from_source {
                    (working_directory.join(extension), working_directory.clone())
                } else {
                    (
                        resolve_inside(&context.repository_root, extension, "Pi extension")?,
                        context.repository_root.clone(),
                    )
                };
                let destination = extension_directory.join(
                    extension
                        .file_name()
                        .expect("validated Pi extension paths have file names"),
                );
                if tool.is_pi_extension_takeover_allowed {
                    validate_takeover_destination(&destination)?;
                } else {
                    reject_non_symlink(&destination, "Pi extension")?;
                }
                reserve_destination(&destination, &mut planned_paths, "Pi extension")?;
                actions.push(Action::LinkPiExtension {
                    source,
                    source_root,
                    destination,
                    is_takeover_allowed: tool.is_pi_extension_takeover_allowed,
                });
            }

            if let Some(package) = &tool.pi_package {
                let source = resource_source(
                    &working_directory,
                    package,
                    "Pi package",
                    is_checkout_present,
                )?;
                let destination = if package == Path::new(".") {
                    extension_directory.join(&tool.name)
                } else {
                    extension_directory.join(
                        package
                            .file_name()
                            .expect("validated Pi package paths have file names"),
                    )
                };
                reject_non_symlink(&destination, "Pi package")?;
                reserve_destination(&destination, &mut planned_paths, "Pi package")?;
                actions.push(Action::LinkPiPackage {
                    source,
                    destination,
                });
            }
        }

        if !tool.skills.is_empty() {
            let skill_directory = context.home_root.join(".agents/skills");
            add_directory(&skill_directory, &mut planned_paths, &mut actions)?;

            for skill in &tool.skills {
                let source =
                    resource_source(&working_directory, skill, "skill", is_checkout_present)?;
                let destination = skill_directory.join(
                    skill
                        .file_name()
                        .expect("validated skill paths have file names"),
                );
                reject_non_symlink(&destination, "skill")?;
                reserve_destination(&destination, &mut planned_paths, "skill")?;
                actions.push(Action::LinkSkill {
                    source,
                    destination,
                });
            }
        }

        if let Some(plugin) = &tool.herdr_plugin {
            let source = if plugin == Path::new(".") {
                working_directory.clone()
            } else {
                resource_source(
                    &working_directory,
                    plugin,
                    "herdr plugin",
                    is_checkout_present,
                )?
            };
            actions.push(Action::LinkHerdrPlugin {
                tool: tool.name.clone(),
                source,
            });
        }
    }

    // install.sh owns ~/.pi/agent/agents: it renders each agent without a model line so the
    // tier map in config/model-tiers.json routes it. A second renderer here fought that one.
    Ok(Plan { actions })
}

fn inspect_checkout(path: &Path, expected_url: &str) -> Result<(), SyncError> {
    let observed_url = git_output(path, &["remote", "get-url", "origin"])
        .map(|output| output.trim().to_owned())
        .unwrap_or_else(|_| "<missing origin>".to_owned());
    if observed_url != expected_url {
        return Err(planning_error(format!(
            "foreign checkout {}: expected origin {expected_url}, observed {observed_url}",
            path.display()
        )));
    }

    let status = git_output(path, &["status", "--porcelain", "--untracked-files=all"])?;
    if !status.is_empty() {
        return Err(planning_error(format!(
            "managed checkout {} is dirty",
            path.display()
        )));
    }
    Ok(())
}

fn git_output(repository: &Path, args: &[&str]) -> Result<String, SyncError> {
    // A git hook exports GIT_DIR and friends, which override -C and would aim
    // this inspection at the hook's own repository instead of the checkout.
    let output = Command::new("git")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_PREFIX")
        .arg("-C")
        .arg(repository)
        .args(args)
        .output()
        .map_err(|error| SyncError::Io(repository.to_path_buf(), error))?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(planning_error(format!(
            "cannot inspect checkout {}: {detail}",
            repository.display()
        )));
    }
    String::from_utf8(output.stdout).map_err(|error| {
        planning_error(format!(
            "Git returned invalid text for {}: {error}",
            repository.display()
        ))
    })
}

fn resolve_inside(root: &Path, relative: &Path, field: &str) -> Result<PathBuf, SyncError> {
    let canonical_root =
        fs::canonicalize(root).map_err(|error| SyncError::Io(root.into(), error))?;
    let candidate = root.join(relative);
    let resolved =
        fs::canonicalize(&candidate).map_err(|error| SyncError::Io(candidate.clone(), error))?;
    if !resolved.starts_with(&canonical_root) {
        return Err(planning_error(format!(
            "{field} {} is outside repository {}",
            candidate.display(),
            root.display()
        )));
    }
    Ok(resolved)
}

fn resource_source(
    root: &Path,
    relative: &Path,
    field: &str,
    is_checkout_present: bool,
) -> Result<PathBuf, SyncError> {
    if is_checkout_present {
        resolve_inside(root, relative, field)
    } else {
        Ok(root.join(relative))
    }
}

fn validate_takeover_destination(destination: &Path) -> Result<(), SyncError> {
    match fs::symlink_metadata(destination) {
        Ok(metadata) if metadata.file_type().is_symlink() => Ok(()),
        Ok(metadata) if metadata.is_file() => {
            let backup = pi_extension_backup(destination);
            match fs::symlink_metadata(&backup) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Ok(_) => Err(planning_error(format!(
                    "Pi extension backup {} already exists",
                    backup.display()
                ))),
                Err(error) => Err(SyncError::Io(backup, error)),
            }
        }
        Ok(_) => Err(planning_error(format!(
            "Pi extension destination {} collides with a non-file",
            destination.display()
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(SyncError::Io(destination.to_path_buf(), error)),
    }
}

fn reject_non_symlink(destination: &Path, field: &str) -> Result<(), SyncError> {
    match fs::symlink_metadata(destination) {
        Ok(metadata) if metadata.file_type().is_symlink() => Ok(()),
        Ok(_) => Err(planning_error(format!(
            "{field} destination {} collides with a non-symlink",
            destination.display()
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(SyncError::Io(destination.to_path_buf(), error)),
    }
}

fn reserve_destination(
    destination: &Path,
    planned: &mut HashSet<PathBuf>,
    field: &str,
) -> Result<(), SyncError> {
    if !planned.insert(destination.to_path_buf()) {
        return Err(planning_error(format!(
            "{field} destination {} collides with another planned destination",
            destination.display()
        )));
    }
    Ok(())
}

fn add_directory(
    path: &Path,
    planned: &mut HashSet<PathBuf>,
    actions: &mut Vec<Action>,
) -> Result<(), SyncError> {
    if planned.contains(path) {
        return Ok(());
    }
    match fs::metadata(path) {
        Ok(_) => return Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(SyncError::Io(path.to_path_buf(), error)),
    }
    planned.insert(path.to_path_buf());
    actions.push(Action::CreateDirectory {
        path: path.to_path_buf(),
    });
    Ok(())
}

fn planning_error(detail: String) -> SyncError {
    SyncError::ManifestInvalid(format!("installation plan invalid: {detail}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{InstallerSpec, ToolSpec};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_FIXTURE_ID: AtomicU64 = AtomicU64::new(0);

    struct Fixture {
        root: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let fixture_id = NEXT_FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "tool-sync-planner-{}-{fixture_id}",
                std::process::id()
            ));
            fs::create_dir_all(&root).expect("fixture root");
            Self { root }
        }

        fn context(&self, platform: Platform) -> Context {
            Context {
                repository_root: self.root.clone(),
                home_root: self.root.join("home"),
                cache_root: self.root.join("cache"),
                platform,
            }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn tool(source: ToolSource) -> ToolSpec {
        ToolSpec {
            name: "rag".to_owned(),
            source,
            platforms: vec![Platform::Linux],
            installer: InstallerSpec {
                command: "./install.sh".to_owned(),
                args: vec!["literal argument".to_owned()],
                preview_args: vec!["--dry-run".to_owned()],
            },
            commands: vec![PathBuf::from("bin/rag")],
            mcp_server: None,
            pi_extension: None,
            is_pi_extension_from_source: false,
            is_pi_extension_takeover_allowed: false,
            pi_package: None,
            skills: Vec::new(),
            herdr_plugin: None,
        }
    }

    fn manifest(tool: ToolSpec) -> ToolManifest {
        ToolManifest { tools: vec![tool] }
    }

    fn run_git(directory: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(directory)
            .args(args)
            .status()
            .expect("run git");
        assert!(status.success());
    }

    fn initialize_checkout(path: &Path, origin: &str) {
        fs::create_dir_all(path).expect("checkout directory");
        run_git(path, &["init", "-q"]);
        run_git(path, &["config", "user.email", "test@example.test"]);
        run_git(path, &["config", "user.name", "Test"]);
        fs::write(path.join("tracked"), "clean").expect("tracked file");
        run_git(path, &["add", "tracked"]);
        run_git(path, &["commit", "-qm", "fixture"]);
        run_git(path, &["remote", "add", "origin", origin]);
    }

    #[test]
    fn orders_git_source_installer_and_links() {
        let fixture = Fixture::new();
        fs::create_dir_all(fixture.root.join("pi/extensions")).expect("adapter directory");
        fs::write(fixture.root.join("pi/extensions/rag.ts"), "extension").expect("adapter");
        let mut spec = tool(ToolSource::Git {
            url: "https://example.test/rag.git".to_owned(),
            revision: "abc123".to_owned(),
        });
        spec.pi_extension = Some(PathBuf::from("pi/extensions/rag.ts"));

        let context = fixture.context(Platform::Linux);
        let plan = build(&manifest(spec), &context).expect("plan");

        assert!(matches!(plan.actions[0], Action::CreateDirectory { .. }));
        assert!(matches!(plan.actions[1], Action::CloneRepository { .. }));
        assert!(matches!(plan.actions[2], Action::CheckoutRevision { .. }));
        assert!(matches!(plan.actions[3], Action::RunInstaller { .. }));
        assert_eq!(
            plan.actions[4],
            Action::CreateDirectory {
                path: context.home_root.join(".local/bin"),
            }
        );
        assert_eq!(
            plan.actions[5],
            Action::LinkCommand {
                source: context.cache_root.join("rag/bin/rag"),
                destination: context.home_root.join(".local/bin/rag"),
            }
        );
        assert!(matches!(plan.actions[6], Action::CreateDirectory { .. }));
        assert!(matches!(plan.actions[7], Action::LinkPiExtension { .. }));
    }

    #[test]
    fn links_a_pi_extension_from_a_git_source() {
        let fixture = Fixture::new();
        let mut spec = tool(ToolSource::Git {
            url: "https://example.test/herdr.git".to_owned(),
            revision: "abc123".to_owned(),
        });
        spec.commands.clear();
        spec.pi_extension = Some(PathBuf::from(
            "src/integration/assets/pi/herdr-agent-state.ts",
        ));
        spec.is_pi_extension_from_source = true;
        spec.is_pi_extension_takeover_allowed = true;

        let context = fixture.context(Platform::Linux);
        let extension_directory = context.home_root.join(".pi/agent/extensions");
        fs::create_dir_all(&extension_directory).expect("extension directory");
        fs::write(
            extension_directory.join("herdr-agent-state.ts"),
            "managed extension",
        )
        .expect("managed extension");
        let plan = build(&manifest(spec), &context).expect("plan");

        assert_eq!(
            plan.actions
                .iter()
                .find(|action| matches!(action, Action::LinkPiExtension { .. })),
            Some(&Action::LinkPiExtension {
                source: context
                    .cache_root
                    .join("rag/src/integration/assets/pi/herdr-agent-state.ts"),
                source_root: context.cache_root.join("rag"),
                destination: context
                    .home_root
                    .join(".pi/agent/extensions/herdr-agent-state.ts"),
                is_takeover_allowed: true,
            })
        );
    }

    #[test]
    fn plans_a_source_extension_that_the_next_revision_adds() {
        let fixture = Fixture::new();
        let checkout = fixture.root.join("cache/rag");
        initialize_checkout(&checkout, "https://example.test/herdr.git");
        let mut spec = tool(ToolSource::Git {
            url: "https://example.test/herdr.git".to_owned(),
            revision: "next-revision".to_owned(),
        });
        spec.commands.clear();
        spec.pi_extension = Some(PathBuf::from("next/herdr-agent-state.ts"));
        spec.is_pi_extension_from_source = true;

        let context = fixture.context(Platform::Linux);
        let plan = build(&manifest(spec), &context).expect("plan before revision checkout");

        assert!(matches!(
            &plan.actions[4],
            Action::LinkPiExtension { source, source_root, .. }
                if source == &checkout.join("next/herdr-agent-state.ts")
                    && source_root == &checkout
        ));
    }

    #[test]
    fn plans_resource_only_package_extension_and_skill_links() {
        let fixture = Fixture::new();
        fs::create_dir_all(fixture.root.join("bundle")).expect("bundle source");
        fs::create_dir_all(fixture.root.join("extensions")).expect("extension source parent");
        fs::write(fixture.root.join("extensions/rag.ts"), "extension").expect("extension source");
        fs::create_dir_all(fixture.root.join("bundle/packages/rag")).expect("package source");
        fs::create_dir_all(fixture.root.join("bundle/skills/show-me")).expect("skill source");
        let mut spec = tool(ToolSource::Embedded {
            path: PathBuf::from("bundle"),
        });
        spec.commands.clear();
        spec.pi_extension = Some(PathBuf::from("extensions/rag.ts"));
        spec.pi_package = Some(PathBuf::from("packages/rag"));
        spec.skills = vec![PathBuf::from("skills/show-me")];

        let context = fixture.context(Platform::Linux);
        let plan = build(&manifest(spec), &context).expect("plan");

        assert_eq!(
            plan.actions[0],
            Action::RunInstaller {
                tool: "rag".to_owned(),
                working_directory: fs::canonicalize(fixture.root.join("bundle"))
                    .expect("canonical bundle"),
                command: "./install.sh".to_owned(),
                args: vec!["literal argument".to_owned()],
                preview_args: vec!["--dry-run".to_owned()],
            }
        );
        assert_eq!(
            plan.actions[1],
            Action::CreateDirectory {
                path: context.home_root.join(".pi/agent/extensions"),
            }
        );
        assert_eq!(
            plan.actions[2],
            Action::LinkPiExtension {
                source: fs::canonicalize(fixture.root.join("extensions/rag.ts"))
                    .expect("canonical extension"),
                source_root: fixture.root.clone(),
                destination: context.home_root.join(".pi/agent/extensions/rag.ts"),
                is_takeover_allowed: false,
            }
        );
        assert_eq!(
            plan.actions[3],
            Action::LinkPiPackage {
                source: fs::canonicalize(fixture.root.join("bundle/packages/rag"))
                    .expect("canonical package"),
                destination: context.home_root.join(".pi/agent/extensions/rag"),
            }
        );
        assert_eq!(
            plan.actions[4],
            Action::CreateDirectory {
                path: context.home_root.join(".agents/skills"),
            }
        );
        assert_eq!(
            plan.actions[5],
            Action::LinkSkill {
                source: fs::canonicalize(fixture.root.join("bundle/skills/show-me"))
                    .expect("canonical skill"),
                destination: context.home_root.join(".agents/skills/show-me"),
            }
        );
    }

    #[test]
    fn plans_root_pi_package_uses_tool_name_destination() {
        let fixture = Fixture::new();
        fs::create_dir_all(fixture.root.join("bundle")).expect("bundle source");
        let mut spec = tool(ToolSource::Embedded {
            path: PathBuf::from("bundle"),
        });
        spec.commands.clear();
        spec.pi_package = Some(PathBuf::from("."));

        let context = fixture.context(Platform::Linux);
        let plan = build(&manifest(spec), &context).expect("plan");

        assert_eq!(
            plan.actions[0],
            Action::RunInstaller {
                tool: "rag".to_owned(),
                working_directory: fs::canonicalize(fixture.root.join("bundle"))
                    .expect("canonical bundle"),
                command: "./install.sh".to_owned(),
                args: vec!["literal argument".to_owned()],
                preview_args: vec!["--dry-run".to_owned()],
            }
        );
        assert_eq!(
            plan.actions[1],
            Action::CreateDirectory {
                path: context.home_root.join(".pi/agent/extensions"),
            }
        );
        assert_eq!(
            plan.actions[2],
            Action::LinkPiPackage {
                source: fs::canonicalize(fixture.root.join("bundle")).expect("canonical bundle"),
                destination: context.home_root.join(".pi/agent/extensions/rag"),
            }
        );
    }

    #[test]
    fn selects_platform_without_inspecting_source() {
        let fixture = Fixture::new();
        let spec = tool(ToolSource::Embedded {
            path: PathBuf::from("missing"),
        });

        let plan = build(&manifest(spec), &fixture.context(Platform::Macos)).expect("skip plan");

        assert_eq!(
            plan.actions,
            [Action::SkipPlatform {
                tool: "rag".to_owned(),
                platform: Platform::Macos,
            }]
        );
    }

    #[test]
    fn plans_git_package_and_skill_links_from_planned_checkout() {
        let fixture = Fixture::new();
        let mut spec = tool(ToolSource::Git {
            url: "https://example.test/rag.git".to_owned(),
            revision: "abc123".to_owned(),
        });
        spec.commands.clear();
        spec.pi_package = Some(PathBuf::from("packages/rag"));
        spec.skills = vec![PathBuf::from("skills/show-me")];

        let context = fixture.context(Platform::Linux);
        let plan = build(&manifest(spec), &context).expect("plan");
        let checkout = context.cache_root.join("rag");

        assert_eq!(
            plan.actions[0],
            Action::CreateDirectory {
                path: context.cache_root.clone(),
            }
        );
        assert_eq!(
            plan.actions[1],
            Action::CloneRepository {
                url: "https://example.test/rag.git".to_owned(),
                destination: checkout.clone(),
            }
        );
        assert_eq!(
            plan.actions[2],
            Action::CheckoutRevision {
                repository: checkout.clone(),
                revision: "abc123".to_owned(),
            }
        );
        assert_eq!(
            plan.actions[3],
            Action::RunInstaller {
                tool: "rag".to_owned(),
                working_directory: checkout.clone(),
                command: "./install.sh".to_owned(),
                args: vec!["literal argument".to_owned()],
                preview_args: vec!["--dry-run".to_owned()],
            }
        );
        assert_eq!(
            plan.actions[5],
            Action::LinkPiPackage {
                source: checkout.join("packages/rag"),
                destination: context.home_root.join(".pi/agent/extensions/rag"),
            }
        );
        assert_eq!(
            plan.actions[7],
            Action::LinkSkill {
                source: checkout.join("skills/show-me"),
                destination: context.home_root.join(".agents/skills/show-me"),
            }
        );
    }

    #[test]
    fn plans_git_package_and_skill_links_from_checkout() {
        let fixture = Fixture::new();
        fs::create_dir_all(fixture.root.join("extensions")).expect("extension source parent");
        fs::write(fixture.root.join("extensions/rag.ts"), "extension").expect("extension source");
        let checkout = fixture.root.join("cache/rag");
        initialize_checkout(&checkout, "https://example.test/rag.git");
        fs::create_dir_all(checkout.join("packages/rag")).expect("package source");
        fs::write(checkout.join("packages/rag/package.toml"), "package").expect("package file");
        fs::create_dir_all(checkout.join("skills/show-me")).expect("skill source");
        fs::write(checkout.join("skills/show-me/skill.md"), "skill").expect("skill file");
        run_git(&checkout, &["add", "packages", "skills"]);
        run_git(&checkout, &["commit", "-qm", "resources"]);

        let mut spec = tool(ToolSource::Git {
            url: "https://example.test/rag.git".to_owned(),
            revision: "abc123".to_owned(),
        });
        spec.commands.clear();
        spec.pi_extension = Some(PathBuf::from("extensions/rag.ts"));
        spec.pi_package = Some(PathBuf::from("packages/rag"));
        spec.skills = vec![PathBuf::from("skills/show-me")];

        let context = fixture.context(Platform::Linux);
        let plan = build(&manifest(spec), &context).expect("plan");

        assert_eq!(
            plan.actions[0],
            Action::FetchRepository {
                repository: checkout.clone(),
            }
        );
        assert_eq!(
            plan.actions[1],
            Action::CheckoutRevision {
                repository: checkout.clone(),
                revision: "abc123".to_owned(),
            }
        );
        assert_eq!(
            plan.actions[2],
            Action::RunInstaller {
                tool: "rag".to_owned(),
                working_directory: checkout.clone(),
                command: "./install.sh".to_owned(),
                args: vec!["literal argument".to_owned()],
                preview_args: vec!["--dry-run".to_owned()],
            }
        );
        assert_eq!(
            plan.actions[3],
            Action::CreateDirectory {
                path: context.home_root.join(".pi/agent/extensions"),
            }
        );
        assert_eq!(
            plan.actions[4],
            Action::LinkPiExtension {
                source: fs::canonicalize(fixture.root.join("extensions/rag.ts"))
                    .expect("canonical extension"),
                source_root: fixture.root.clone(),
                destination: context.home_root.join(".pi/agent/extensions/rag.ts"),
                is_takeover_allowed: false,
            }
        );
        assert_eq!(
            plan.actions[5],
            Action::LinkPiPackage {
                source: fs::canonicalize(checkout.join("packages/rag")).expect("canonical package"),
                destination: context.home_root.join(".pi/agent/extensions/rag"),
            }
        );
        assert_eq!(
            plan.actions[6],
            Action::CreateDirectory {
                path: context.home_root.join(".agents/skills"),
            }
        );
        assert_eq!(
            plan.actions[7],
            Action::LinkSkill {
                source: fs::canonicalize(checkout.join("skills/show-me")).expect("canonical skill"),
                destination: context.home_root.join(".agents/skills/show-me"),
            }
        );
    }

    #[test]
    fn plans_git_herdr_plugin_from_planned_checkout() {
        let fixture = Fixture::new();
        let mut spec = tool(ToolSource::Git {
            url: "https://example.test/fresh-worktree.git".to_owned(),
            revision: "abc123".to_owned(),
        });
        spec.commands.clear();
        spec.herdr_plugin = Some(PathBuf::from("plugins/fresh"));

        let context = fixture.context(Platform::Linux);
        let plan = build(&manifest(spec), &context).expect("plan");

        assert_eq!(
            plan.actions[4],
            Action::LinkHerdrPlugin {
                tool: "rag".to_owned(),
                source: context.cache_root.join("rag/plugins/fresh"),
            }
        );
    }

    #[test]
    fn plans_embedded_herdr_plugin_from_repository() {
        let fixture = Fixture::new();
        fs::create_dir_all(fixture.root.join("bundle/herdr/worktree-layout"))
            .expect("plugin source");
        let mut spec = tool(ToolSource::Embedded {
            path: PathBuf::from("bundle"),
        });
        spec.commands.clear();
        spec.herdr_plugin = Some(PathBuf::from("herdr/worktree-layout"));

        let plan = build(&manifest(spec), &fixture.context(Platform::Linux)).expect("plan");

        assert_eq!(
            plan.actions[1],
            Action::LinkHerdrPlugin {
                tool: "rag".to_owned(),
                source: fs::canonicalize(fixture.root.join("bundle/herdr/worktree-layout"))
                    .expect("canonical plugin"),
            }
        );
    }

    #[test]
    fn plans_root_herdr_plugin_at_checkout_root() {
        let fixture = Fixture::new();
        let mut spec = tool(ToolSource::Git {
            url: "https://example.test/fresh-worktree.git".to_owned(),
            revision: "abc123".to_owned(),
        });
        spec.commands.clear();
        spec.herdr_plugin = Some(PathBuf::from("."));

        let context = fixture.context(Platform::Linux);
        let plan = build(&manifest(spec), &context).expect("plan");

        assert_eq!(
            plan.actions[4],
            Action::LinkHerdrPlugin {
                tool: "rag".to_owned(),
                source: context.cache_root.join("rag"),
            }
        );
    }

    #[test]
    fn rejects_colliding_skill_destinations() {
        let fixture = Fixture::new();
        fs::create_dir_all(fixture.root.join("bundle")).expect("bundle source");
        fs::create_dir_all(fixture.root.join("bundle/skills/first/show-me")).expect("first skill");
        fs::create_dir_all(fixture.root.join("bundle/skills/second/show-me"))
            .expect("second skill");
        let mut spec = tool(ToolSource::Embedded {
            path: PathBuf::from("bundle"),
        });
        spec.commands.clear();
        spec.skills = vec![
            PathBuf::from("skills/first/show-me"),
            PathBuf::from("skills/second/show-me"),
        ];

        let error = build(&manifest(spec), &fixture.context(Platform::Linux))
            .expect_err("duplicate skill destination rejected");

        assert!(error
            .to_string()
            .contains("collides with another planned destination"));
    }

    #[test]
    fn refuses_dirty_and_foreign_checkouts() {
        let fixture = Fixture::new();
        let checkout = fixture.root.join("cache/rag");
        initialize_checkout(&checkout, "https://example.test/rag.git");
        fs::write(checkout.join("untracked"), "dirty").expect("dirty file");
        let source = ToolSource::Git {
            url: "https://example.test/rag.git".to_owned(),
            revision: "abc123".to_owned(),
        };
        let error = build(&manifest(tool(source)), &fixture.context(Platform::Linux))
            .expect_err("dirty checkout rejected");
        assert!(error.to_string().contains("dirty"));

        fs::remove_file(checkout.join("untracked")).expect("clean checkout");
        let source = ToolSource::Git {
            url: "https://foreign.test/rag.git".to_owned(),
            revision: "abc123".to_owned(),
        };
        let error = build(&manifest(tool(source)), &fixture.context(Platform::Linux))
            .expect_err("foreign checkout rejected");
        let message = error.to_string();
        assert!(message.contains("https://foreign.test/rag.git"));
        assert!(message.contains("https://example.test/rag.git"));
    }

    #[test]
    fn rejects_invalid_pi_extension_takeover_destinations() {
        let fixture = Fixture::new();
        let destination = fixture.root.join("herdr-agent-state.ts");
        fs::create_dir_all(&destination).expect("directory collision");
        assert!(validate_takeover_destination(&destination).is_err());

        fs::remove_dir(&destination).expect("remove directory fixture");
        fs::write(&destination, "version 8").expect("managed extension");
        fs::write(pi_extension_backup(&destination), "existing backup").expect("existing backup");
        assert!(validate_takeover_destination(&destination).is_err());
    }

    #[test]
    fn refuses_non_symlink_command_collision() {
        let fixture = Fixture::new();
        fs::create_dir_all(fixture.root.join("embedded/bin")).expect("embedded source");
        fs::create_dir_all(fixture.root.join("home/.local/bin")).expect("command directory");
        fs::write(fixture.root.join("home/.local/bin/rag"), "mine").expect("collision");
        let spec = tool(ToolSource::Embedded {
            path: PathBuf::from("embedded"),
        });

        let error = build(&manifest(spec), &fixture.context(Platform::Linux))
            .expect_err("collision rejected");

        assert!(error.to_string().contains("non-symlink"));
    }

    #[cfg(unix)]
    #[test]
    fn refuses_adapter_symlink_outside_repository() {
        use std::os::unix::fs::symlink;

        let fixture = Fixture::new();
        fs::create_dir_all(fixture.root.join("embedded")).expect("embedded source");
        let outside = fixture.root.with_extension("outside");
        fs::write(&outside, "outside").expect("outside adapter");
        symlink(&outside, fixture.root.join("adapter.ts")).expect("adapter symlink");
        let mut spec = tool(ToolSource::Embedded {
            path: PathBuf::from("embedded"),
        });
        spec.pi_extension = Some(PathBuf::from("adapter.ts"));

        let error = build(&manifest(spec), &fixture.context(Platform::Linux))
            .expect_err("escaped adapter rejected");

        assert!(error.to_string().contains("outside repository"));
        fs::remove_file(outside).expect("outside cleanup");
    }
}
