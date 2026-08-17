use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

// TODO(AGNT-0012.T13): Cover package, skill, agent, and repeated stack installation.
static NEXT_FIXTURE_ID: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
    repository: PathBuf,
    remote: PathBuf,
    first_revision: String,
    second_revision: String,
}

impl Fixture {
    fn new() -> Self {
        let root = fixture_root("tool-sync-install");
        let repository = root.join("repository");
        let work = root.join("work");
        let remote = root.join("remote.git");
        fs::create_dir_all(repository.join("pi/extensions")).expect("repository fixture");
        fs::copy(
            fixture_file("rag.ts"),
            repository.join("pi/extensions/rag.ts"),
        )
        .expect("Pi extension fixture");
        fs::create_dir_all(&work).expect("Git work tree");
        git(&work, &["init", "-q"]);
        git(&work, &["config", "user.email", "fixture@example.test"]);
        git(&work, &["config", "user.name", "Fixture"]);
        fs::create_dir_all(work.join("bin")).expect("command directory");
        copy_executable(&fixture_file("installer.sh"), &work.join("install.sh"));
        copy_executable(&fixture_file("rag"), &work.join("bin/rag"));
        fs::write(work.join("revision"), "first\n").expect("first revision file");
        git(&work, &["add", "."]);
        git(&work, &["commit", "-qm", "first"]);
        let first_revision = git_text(&work, &["rev-parse", "HEAD"]);
        fs::write(work.join("revision"), "second\n").expect("second revision file");
        git(&work, &["commit", "-qam", "second"]);
        let second_revision = git_text(&work, &["rev-parse", "HEAD"]);
        fs::create_dir_all(&remote).expect("bare repository directory");
        git(&remote, &["init", "--bare", "-q"]);
        git(&work, &["remote", "add", "origin", path_text(&remote)]);
        git(&work, &["push", "-q", "origin", "HEAD:refs/heads/main"]);
        git(&remote, &["symbolic-ref", "HEAD", "refs/heads/main"]);

        Self {
            root,
            repository,
            remote,
            first_revision,
            second_revision,
        }
    }

    fn home(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    fn manifest(&self, revision: &str, platforms: &str) -> PathBuf {
        let manifest = self.root.join(format!("manifest-{platforms}.toml"));
        fs::write(
            &manifest,
            format!(
                r#"[[tools]]
name = "rag"
platforms = [{platforms}]
commands = ["bin/rag"]
pi_extension = "pi/extensions/rag.ts"
source = {{ url = {:?}, revision = {:?} }}
installer = {{ command = "./install.sh", args = ["apply"], preview_args = ["preview"] }}
"#,
                path_text(&self.remote),
                revision
            ),
        )
        .expect("manifest");
        manifest
    }

    fn run(&self, home: &Path, manifest: &Path, platform: &str, is_dry_run: bool) -> Output {
        let record = self.root.join("installer-invocations");
        let mut command = Command::new(env!("CARGO_BIN_EXE_tool-sync"));
        command
            .env("TOOL_SYNC_RECORD", record)
            .args(["--home", path_text(home)])
            .args(["--manifest", path_text(manifest)])
            .args(["--repository-root", path_text(&self.repository)])
            .args(["--platform", platform]);
        if is_dry_run {
            command.arg("--dry-run");
        }
        command.output().expect("run tool-sync")
    }

    fn checkout(&self, home: &Path) -> PathBuf {
        home.join(".cache/tool-sync/rag")
    }

    fn record(&self) -> PathBuf {
        self.root.join("installer-invocations")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn fixture_root(prefix: &str) -> PathBuf {
    loop {
        let id = NEXT_FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
        let root = env::temp_dir().join(format!("{prefix}-{}-{id}", std::process::id()));
        match fs::create_dir(&root) {
            Ok(()) => return root,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => panic!("create fixture root {}: {error}", root.display()),
        }
    }
}

fn fixture_file(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn path_text(path: &Path) -> &str {
    path.to_str().expect("fixture path is UTF-8")
}

fn copy_executable(source: &Path, destination: &Path) {
    fs::copy(source, destination).expect("copy executable fixture");
    fs::set_permissions(destination, fs::Permissions::from_mode(0o755))
        .expect("set executable fixture mode");
}

fn git(directory: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(directory)
        .args(args)
        .output()
        .expect("run local Git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_text(directory: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(directory)
        .args(args)
        .output()
        .expect("run local Git");
    assert!(output.status.success(), "git {args:?} failed");
    String::from_utf8(output.stdout)
        .expect("Git output is UTF-8")
        .trim()
        .to_owned()
}

fn output_text(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("tool-sync output is UTF-8")
}

fn error_text(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("tool-sync error is UTF-8")
}

fn tree_snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, path: &Path, entries: &mut BTreeMap<PathBuf, Vec<u8>>) {
        let mut children = fs::read_dir(path)
            .expect("read snapshot directory")
            .map(|entry| entry.expect("snapshot entry").path())
            .collect::<Vec<_>>();
        children.sort();
        for child in children {
            let relative = child.strip_prefix(root).expect("snapshot relative path");
            let metadata = fs::symlink_metadata(&child).expect("snapshot metadata");
            if metadata.file_type().is_symlink() {
                entries.insert(
                    relative.to_owned(),
                    fs::read_link(&child)
                        .expect("snapshot link")
                        .into_os_string()
                        .into_encoded_bytes(),
                );
            } else if metadata.is_dir() {
                entries.insert(relative.to_owned(), Vec::new());
                visit(root, &child, entries);
            } else {
                entries.insert(
                    relative.to_owned(),
                    fs::read(&child).expect("snapshot file"),
                );
            }
        }
    }

    let mut entries = BTreeMap::new();
    if root.exists() {
        visit(root, root, &mut entries);
    }
    entries
}

#[test]
fn previews_clone_in_order_without_writing_or_invoking_a_missing_checkout() {
    let fixture = Fixture::new();
    let home = fixture.home("preview-home");
    fs::create_dir(&home).expect("preview home");
    fs::write(home.join("seed"), b"unchanged\0bytes").expect("home seed");
    let before = tree_snapshot(&home);
    let manifest = fixture.manifest(&fixture.first_revision, "\"linux\"");

    let output = fixture.run(&home, &manifest, "linux", true);

    assert!(output.status.success(), "{}", error_text(&output));
    let report = output_text(&output);
    let expected = [
        format!(
            "create directory {}",
            home.join(".cache/tool-sync").display()
        ),
        format!("clone {}", fixture.remote.display()),
        format!("checkout {}", fixture.first_revision),
        "install rag".to_owned(),
        "[\"preview\"]".to_owned(),
        format!(
            "link command {}",
            home.join(".cache/tool-sync/rag/bin/rag").display()
        ),
        "link Pi extension".to_owned(),
    ];
    let mut previous = 0;
    for text in expected {
        let position = report[previous..]
            .find(&text)
            .unwrap_or_else(|| panic!("missing {text:?} in {report}"))
            + previous;
        previous = position + text.len();
    }
    assert!(!fixture.record().exists(), "missing checkout invoked child");
    assert_eq!(tree_snapshot(&home), before);
}

#[test]
fn applies_a_pinned_revision_and_repeats_without_duplicate_directories_or_checkouts() {
    let fixture = Fixture::new();
    let home = fixture.home("apply-home");
    let manifest = fixture.manifest(&fixture.first_revision, "\"linux\", \"macos\"");

    let first = fixture.run(&home, &manifest, "linux", false);

    assert!(first.status.success(), "{}", error_text(&first));
    let checkout = fixture.checkout(&home);
    assert_eq!(
        git_text(&checkout, &["rev-parse", "HEAD"]),
        fixture.first_revision
    );
    let command = home.join(".local/bin/rag");
    let extension = home.join(".pi/agent/extensions/rag.ts");
    let command_target = fs::read_link(&command).expect("command link");
    let extension_target = fs::read_link(&extension).expect("extension link");
    assert_eq!(command_target, checkout.join("bin/rag"));
    assert_eq!(
        extension_target,
        fs::canonicalize(fixture.repository.join("pi/extensions/rag.ts"))
            .expect("canonical extension")
    );
    let selected_contents = fs::read(checkout.join("revision")).expect("selected contents");
    let checkout_metadata = fs::metadata(&checkout).expect("checkout metadata");

    let second = fixture.run(&home, &manifest, "linux", false);

    assert!(second.status.success(), "{}", error_text(&second));
    assert_eq!(
        git_text(&checkout, &["rev-parse", "HEAD"]),
        fixture.first_revision
    );
    assert_eq!(
        fs::read(checkout.join("revision")).expect("repeated selected contents"),
        selected_contents
    );
    assert_eq!(
        fs::read_link(command).expect("repeated command link"),
        command_target
    );
    assert_eq!(
        fs::read_link(extension).expect("repeated extension link"),
        extension_target
    );
    let repeated_metadata = fs::metadata(&checkout).expect("repeated checkout metadata");
    assert_eq!(
        (repeated_metadata.dev(), repeated_metadata.ino()),
        (checkout_metadata.dev(), checkout_metadata.ino()),
        "the existing checkout directory must be reused"
    );
    let mut cached = fs::read_dir(home.join(".cache/tool-sync"))
        .expect("cache directory")
        .map(|entry| entry.expect("cache entry").path())
        .collect::<Vec<_>>();
    cached.sort();
    assert_eq!(cached, [checkout]);
    let report = output_text(&second);
    assert!(report.contains("fetch repository"), "{report}");
    assert!(report.contains("install rag"), "{report}");
    assert!(!report.contains("clone "), "{report}");
    assert!(!report.contains("create directory"), "{report}");
    assert_eq!(
        fs::read_to_string(fixture.record()).expect("installer invocations"),
        format!(
            "{0}|apply\n{0}|apply\n",
            fs::canonicalize(&cached[0])
                .expect("canonical checkout")
                .display()
        )
    );
}

#[test]
fn previews_the_child_for_an_existing_checkout() {
    let fixture = Fixture::new();
    let home = fixture.home("child-preview-home");
    let manifest = fixture.manifest(&fixture.first_revision, "\"linux\"");
    let applied = fixture.run(&home, &manifest, "linux", false);
    assert!(applied.status.success(), "{}", error_text(&applied));
    fs::write(fixture.record(), "").expect("clear invocation record");
    let before = tree_snapshot(&home);

    let preview = fixture.run(&home, &manifest, "linux", true);

    assert!(preview.status.success(), "{}", error_text(&preview));
    let invocations = fs::read_to_string(fixture.record()).expect("preview invocation");
    assert_eq!(
        invocations,
        format!(
            "{}|preview\n",
            fs::canonicalize(fixture.checkout(&home))
                .expect("canonical checkout")
                .display()
        )
    );
    assert_eq!(tree_snapshot(&home), before);
}

#[test]
fn skips_tools_selected_for_the_other_platform_without_inspecting_them() {
    let fixture = Fixture::new();
    let home = fixture.home("skip-home");
    for (declared, selected) in [("\"macos\"", "linux"), ("\"linux\"", "macos")] {
        let manifest = fixture.manifest("missing-revision", declared);
        let output = fixture.run(&home, &manifest, selected, true);
        assert!(output.status.success(), "{}", error_text(&output));
        let report = output_text(&output);
        assert_eq!(report.lines().count(), 1, "{report}");
        assert!(
            report.contains(&format!("skip rag on {selected}")),
            "{report}"
        );
        for forbidden in ["clone ", "checkout ", "install ", "link "] {
            assert!(!report.contains(forbidden), "{report}");
        }
    }
    assert!(!home.exists());
    assert!(!fixture.record().exists());
}

#[test]
fn refuses_a_dirty_checkout_before_advancing_revision_or_links() {
    let fixture = Fixture::new();
    let home = fixture.home("dirty-home");
    let first_manifest = fixture.manifest(&fixture.first_revision, "\"linux\"");
    let first = fixture.run(&home, &first_manifest, "linux", false);
    assert!(first.status.success(), "{}", error_text(&first));
    let checkout = fixture.checkout(&home);
    let dirty = checkout.join("uncommitted");
    fs::write(&dirty, "keep me").expect("dirty checkout file");
    let command = home.join(".local/bin/rag");
    let extension = home.join(".pi/agent/extensions/rag.ts");
    let command_target = fs::read_link(&command).expect("command target");
    let extension_target = fs::read_link(&extension).expect("extension target");
    let next_manifest = fixture.manifest(&fixture.second_revision, "\"linux\", \"macos\"");

    let output = fixture.run(&home, &next_manifest, "linux", false);

    assert!(!output.status.success());
    let error = error_text(&output);
    assert!(error.contains("dirty"), "{error}");
    assert!(error.contains(path_text(&checkout)), "{error}");
    assert_eq!(
        git_text(&checkout, &["rev-parse", "HEAD"]),
        fixture.first_revision
    );
    assert_eq!(
        fs::read_to_string(dirty).expect("dirty file retained"),
        "keep me"
    );
    assert_eq!(
        fs::read_link(command).expect("command retained"),
        command_target
    );
    assert_eq!(
        fs::read_link(extension).expect("extension retained"),
        extension_target
    );
}

#[test]
fn installs_an_embedded_tool_without_a_git_cache() {
    let fixture = Fixture::new();
    let home = fixture.home("embedded-home");
    let embedded = fixture.repository.join("embedded");
    fs::create_dir_all(embedded.join("bin")).expect("embedded command directory");
    copy_executable(&fixture_file("installer.sh"), &embedded.join("install.sh"));
    copy_executable(&fixture_file("rag"), &embedded.join("bin/embedded-rag"));
    let manifest = fixture.root.join("embedded.toml");
    fs::write(
        &manifest,
        r#"[[tools]]
name = "embedded-rag"
platforms = ["linux"]
commands = ["bin/embedded-rag"]
source = { path = "embedded" }
installer = { command = "./install.sh", args = ["apply"], preview_args = ["preview"] }
"#,
    )
    .expect("embedded manifest");

    let output = fixture.run(&home, &manifest, "linux", false);

    assert!(output.status.success(), "{}", error_text(&output));
    assert!(!home.join(".cache/tool-sync").exists());
    let resolved_embedded = fs::canonicalize(&embedded).expect("canonical embedded source");
    assert_eq!(
        fs::read_link(home.join(".local/bin/embedded-rag")).expect("embedded command link"),
        resolved_embedded.join("bin/embedded-rag")
    );
    assert_eq!(
        fs::read_to_string(fixture.record()).expect("embedded installer invocation"),
        format!("{}|apply\n", resolved_embedded.display())
    );
}

#[test]
fn top_level_installer_dry_run_leaves_an_absent_home_absent() {
    let root = fixture_root("tool-sync-top-level");
    let repository = root.join("repository");
    let home = root.join("home");
    fs::create_dir_all(repository.join("skills/fixture")).expect("top-level fixture");
    fs::create_dir(repository.join("agents")).expect("agents fixture");
    fs::write(repository.join("CLAUDE.md"), "fixture").expect("instructions fixture");
    let installer = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../install.sh");

    let output = Command::new("bash")
        .arg(installer)
        .arg("--dry-run")
        .env("HOME", &home)
        .env("REPO_TARGET", &repository)
        .output()
        .expect("run top-level installer dry-run");

    assert!(output.status.success(), "{}", error_text(&output));
    assert!(!home.exists());
    fs::remove_dir_all(root).expect("top-level fixture cleanup");
}

#[test]
fn top_level_dry_run_requires_tool_sync_to_be_built() {
    let root = fixture_root("tool-sync-top-level-unbuilt");
    let repository = root.join("repository");
    let home = root.join("home");
    fs::create_dir_all(repository.join("skills/fixture")).expect("top-level fixture");
    fs::create_dir_all(repository.join("tools/tool-sync")).expect("tool-sync fixture");
    fs::write(repository.join("CLAUDE.md"), "fixture").expect("instructions fixture");
    fs::write(repository.join("tools/tool-sync/Cargo.toml"), "[package]\n").expect("crate fixture");
    let installer = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../install.sh");

    let output = Command::new("bash")
        .arg(installer)
        .arg("--dry-run")
        .env("HOME", &home)
        .env("REPO_TARGET", &repository)
        .output()
        .expect("run top-level installer dry-run");

    assert!(!output.status.success());
    assert!(error_text(&output).contains("cargo build --release --manifest-path"));
    assert!(!home.exists());
    fs::remove_dir_all(root).expect("top-level fixture cleanup");
}
