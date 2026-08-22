use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_FIXTURE_ID: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
    repository: PathBuf,
    remote: PathBuf,
    first_revision: String,
    second_revision: String,
    first_revision_contents: Vec<u8>,
    first_skill_show_me_contents: Vec<u8>,
}

impl Fixture {
    fn new() -> Self {
        let root = fixture_root("tool-sync-install");
        let repository = root.join("repository");
        let work = root.join("work");
        let remote = root.join("remote.git");
        fs::create_dir_all(&repository).expect("repository fixture directory");
        fs::create_dir_all(&work).expect("Git work tree");
        seed_repository_root_tree(&repository);
        seed_checkout_tree(&work);
        fs::write(work.join("revision"), "first revision\n").expect("first revision file");
        git(&work, &["init", "-q"]);
        git(&work, &["config", "user.email", "fixture@example.test"]);
        git(&work, &["config", "user.name", "Fixture"]);
        git(&work, &["add", "."]);
        git(&work, &["commit", "-qm", "first"]);
        let first_revision = git_text(&work, &["rev-parse", "HEAD"]);
        let first_revision_contents =
            fs::read(work.join("revision")).expect("first revision contents");
        let first_skill_show_me_contents =
            fs::read(work.join("skills/show-me/README.md")).expect("first skill contents");
        fs::write(work.join("revision"), "second revision\n").expect("second revision file");
        fs::write(
            work.join("skills/show-me/README.md"),
            "show-me checkout fixture second\n",
        )
        .expect("second skill contents");
        git(&work, &["commit", "-qam", "second"]);
        let second_revision = git_text(&work, &["rev-parse", "HEAD"]);
        fs::create_dir_all(&remote).expect("bare repository directory");
        let remote_path = canonical_path(&remote);
        git(&remote, &["init", "--bare", "-q"]);
        git(&work, &["remote", "add", "origin", path_text(&remote_path)]);
        git(&work, &["push", "-q", "origin", "HEAD:refs/heads/main"]);
        git(&remote, &["symbolic-ref", "HEAD", "refs/heads/main"]);

        Self {
            root,
            repository,
            remote,
            first_revision,
            second_revision,
            first_revision_contents,
            first_skill_show_me_contents,
        }
    }

    fn home(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    fn repository_root(&self) -> PathBuf {
        canonical_path(&self.repository)
    }

    fn remote_path(&self) -> PathBuf {
        canonical_path(&self.remote)
    }

    // Mirrors the actual pi-subagents entry in config/tools.toml after AGNT-0063.T02:
    // a root pi_package (".") with no skills field, matching the candidate's layout
    // (github.com/tintinweb/pi-subagents has no skills/ directory).
    fn candidate_manifest(&self, revision: &str, platforms: &str) -> PathBuf {
        let manifest = self.root.join(format!("manifest-candidate-{platforms}.toml"));
        let remote = self.remote_path();
        fs::write(
            &manifest,
            format!(
                r#"[[tools]]
name = "pi-subagents"
platforms = [{platforms}]
commands = []
pi_package = "."
source = {{ url = {:?}, revision = {:?} }}
installer = {{ command = "./install.sh", args = ["apply"], preview_args = ["preview"] }}
"#,
                path_text(&remote),
                revision
            ),
        )
        .expect("candidate manifest");
        manifest
    }

    fn candidate_checkout(&self, home: &Path) -> PathBuf {
        canonical_path(home).join(".cache/tool-sync/pi-subagents")
    }

    fn manifest(&self, revision: &str, platforms: &str) -> PathBuf {
        let manifest = self.root.join(format!("manifest-{platforms}.toml"));
        let remote = self.remote_path();
        fs::write(
            &manifest,
            format!(
                r#"[[tools]]
name = "rag"
platforms = [{platforms}]
commands = ["bin/rag"]
pi_extension = "pi/extensions/rag.ts"
pi_package = "."
skills = ["skills/show-me", "skills/grilling"]
source = {{ url = {:?}, revision = {:?} }}
installer = {{ command = "./install.sh", args = ["apply"], preview_args = ["preview"] }}
"#,
                path_text(&remote),
                revision
            ),
        )
        .expect("manifest");
        manifest
    }

    fn run(&self, home: &Path, manifest: &Path, platform: &str, is_dry_run: bool) -> Output {
        let record = self.root.join("installer-invocations");
        let home = canonical_path(home);
        let repository = self.repository_root();
        let mut command = Command::new(env!("CARGO_BIN_EXE_tool-sync"));
        command
            .env("TOOL_SYNC_RECORD", record)
            .args(["--home", path_text(&home)])
            .args(["--manifest", path_text(manifest)])
            .args(["--repository-root", path_text(&repository)])
            .args(["--platform", platform]);
        if is_dry_run {
            command.arg("--dry-run");
        }
        command.output().expect("run tool-sync")
    }

    fn checkout(&self, home: &Path) -> PathBuf {
        canonical_path(home).join(".cache/tool-sync/rag")
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

fn seed_repository_root_tree(root: &Path) {
    copy_tree(
        &fixture_file("install/repository-root/pi"),
        &root.join("pi"),
    );
}

fn seed_checkout_tree(root: &Path) {
    copy_tree(
        &fixture_file("install/checkout/skills"),
        &root.join("skills"),
    );
    copy_executable(
        &fixture_file("install/checkout/install.sh"),
        &root.join("install.sh"),
    );
    copy_executable(
        &fixture_file("install/checkout/bin/rag"),
        &root.join("bin/rag"),
    );
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

fn canonical_path(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn path_text(path: &Path) -> &str {
    path.to_str().expect("fixture path is UTF-8")
}

fn copy_tree(source: &Path, destination: &Path) {
    let metadata = fs::symlink_metadata(source).expect("fixture metadata");
    if metadata.is_dir() {
        fs::create_dir_all(destination).expect("fixture directory");
        let mut children = fs::read_dir(source)
            .expect("fixture children")
            .map(|entry| entry.expect("fixture child").path())
            .collect::<Vec<_>>();
        children.sort();
        for child in children {
            let target = destination.join(child.file_name().expect("fixture child name"));
            copy_tree(&child, &target);
        }
        return;
    }

    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).expect("fixture parent directory");
    }
    fs::copy(source, destination).expect("copy fixture file");
}

fn copy_executable(source: &Path, destination: &Path) {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).expect("executable parent directory");
    }
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

fn normalize_private_prefix(text: &str) -> String {
    text.replace("/private", "")
}

fn normalize_private_path_text(path: &Path) -> String {
    normalize_private_prefix(&path.display().to_string())
}

fn assert_lines_in_order(report: &str, expected: &[String]) {
    let mut previous = 0;
    for text in expected {
        let position = report[previous..]
            .find(text)
            .unwrap_or_else(|| panic!("missing {text:?} in {report}"))
            + previous;
        previous = position + text.len();
    }
}

fn assert_same_link_target(link: &Path, expected: &Path) {
    let actual = fs::read_link(link).expect("read link target");
    assert_eq!(
        normalize_private_path_text(&actual),
        normalize_private_path_text(expected)
    );
}

fn symlink_identity(path: &Path) -> (u64, u64) {
    let metadata = fs::symlink_metadata(path).expect("symlink metadata");
    (metadata.dev(), metadata.ino())
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
fn previews_a_pinned_checkout_with_package_skills_and_agents_without_writing_home() {
    let fixture = Fixture::new();
    let home = fixture.home("preview-home");
    fs::create_dir_all(&home).expect("preview home");
    fs::write(home.join("seed.txt"), b"unchanged home content\n").expect("home seed");
    let home = canonical_path(&home);
    let before = tree_snapshot(&home);
    let manifest = fixture.manifest(&fixture.first_revision, "\"linux\"");

    let output = fixture.run(&home, &manifest, "linux", true);

    assert!(output.status.success(), "{}", error_text(&output));
    let report = normalize_private_prefix(&output_text(&output));
    let checkout = fixture.checkout(&home);
    let repository = fixture.repository_root();
    let expected = vec![
        format!(
            "create directory {}",
            normalize_private_path_text(&home.join(".cache/tool-sync"))
        ),
        format!(
            "clone {} into {}",
            normalize_private_path_text(&fixture.remote_path()),
            normalize_private_path_text(&checkout)
        ),
        format!(
            "checkout {} in {}",
            fixture.first_revision,
            normalize_private_path_text(&checkout)
        ),
        format!(
            "install rag in {}: ./install.sh [\"preview\"]",
            normalize_private_path_text(&checkout)
        ),
        format!(
            "create directory {}",
            normalize_private_path_text(&home.join(".local/bin"))
        ),
        format!(
            "link command {} -> {}",
            normalize_private_path_text(&checkout.join("bin/rag")),
            normalize_private_path_text(&home.join(".local/bin/rag"))
        ),
        format!(
            "create directory {}",
            normalize_private_path_text(&home.join(".pi/agent/extensions"))
        ),
        format!(
            "link Pi extension {} -> {}",
            normalize_private_path_text(&repository.join("pi/extensions/rag.ts")),
            normalize_private_path_text(&home.join(".pi/agent/extensions/rag.ts"))
        ),
        format!(
            "link Pi package {} -> {}",
            normalize_private_path_text(&checkout.join(".")),
            normalize_private_path_text(&home.join(".pi/agent/extensions/rag"))
        ),
        format!(
            "create directory {}",
            normalize_private_path_text(&home.join(".agents/skills"))
        ),
        format!(
            "link skill {} -> {}",
            normalize_private_path_text(&checkout.join("skills/show-me")),
            normalize_private_path_text(&home.join(".agents/skills/show-me"))
        ),
        format!(
            "link skill {} -> {}",
            normalize_private_path_text(&checkout.join("skills/grilling")),
            normalize_private_path_text(&home.join(".agents/skills/grilling"))
        ),
    ];
    assert_lines_in_order(&report, &expected);
    assert!(!fixture.record().exists());
    assert_eq!(tree_snapshot(&home), before);
}

#[test]
fn applies_a_pinned_checkout_and_repeats_without_duplicate_directories_or_checkouts() {
    let fixture = Fixture::new();
    let home = fixture.home("apply-home");
    fs::create_dir_all(&home).expect("apply home");
    fs::write(home.join("notes.txt"), b"keep this home content\n").expect("home notes");
    let home = canonical_path(&home);
    let manifest = fixture.manifest(&fixture.first_revision, "\"linux\"");

    let first = fixture.run(&home, &manifest, "linux", false);

    assert!(first.status.success(), "{}", error_text(&first));
    let checkout = fixture.checkout(&home);
    assert_eq!(
        git_text(&checkout, &["rev-parse", "HEAD"]),
        fixture.first_revision
    );
    assert_eq!(
        fs::read(checkout.join("revision")).expect("selected revision contents"),
        fixture.first_revision_contents
    );
    assert_eq!(
        fs::read(checkout.join("skills/show-me/README.md")).expect("selected skill contents"),
        fixture.first_skill_show_me_contents
    );
    let command = home.join(".local/bin/rag");
    let extension = home.join(".pi/agent/extensions/rag.ts");
    let package = home.join(".pi/agent/extensions/rag");
    let show_me = home.join(".agents/skills/show-me");
    let grilling = home.join(".agents/skills/grilling");
    assert_same_link_target(&command, &checkout.join("bin/rag"));
    assert_same_link_target(
        &extension,
        &fixture.repository_root().join("pi/extensions/rag.ts"),
    );
    assert_same_link_target(&package, &checkout.join("."));
    assert_same_link_target(&show_me, &checkout.join("skills/show-me"));
    assert_same_link_target(&grilling, &checkout.join("skills/grilling"));
    assert_eq!(
        fs::read_to_string(home.join("notes.txt")).expect("notes after apply"),
        "keep this home content\n"
    );
    let checkout_metadata = fs::metadata(&checkout).expect("checkout metadata");
    let command_identity = symlink_identity(&command);
    let extension_identity = symlink_identity(&extension);
    let package_identity = symlink_identity(&package);
    let show_me_identity = symlink_identity(&show_me);
    let grilling_identity = symlink_identity(&grilling);
    assert_eq!(
        normalize_private_prefix(&fs::read_to_string(fixture.record()).expect("apply record")),
        normalize_private_prefix(&format!("{}|apply\n", checkout.display()))
    );

    let second = fixture.run(&home, &manifest, "linux", false);

    assert!(second.status.success(), "{}", error_text(&second));
    assert_eq!(
        git_text(&checkout, &["rev-parse", "HEAD"]),
        fixture.first_revision
    );
    assert_eq!(
        fs::read(checkout.join("revision")).expect("repeated selected revision contents"),
        fixture.first_revision_contents
    );
    assert_eq!(
        fs::read(checkout.join("skills/show-me/README.md"))
            .expect("repeated selected skill contents"),
        fixture.first_skill_show_me_contents
    );
    assert_same_link_target(&command, &checkout.join("bin/rag"));
    assert_same_link_target(
        &extension,
        &fixture.repository_root().join("pi/extensions/rag.ts"),
    );
    assert_same_link_target(&package, &checkout.join("."));
    assert_same_link_target(&show_me, &checkout.join("skills/show-me"));
    assert_same_link_target(&grilling, &checkout.join("skills/grilling"));
    assert_eq!(
        fs::read_to_string(home.join("notes.txt")).expect("notes after repeated apply"),
        "keep this home content\n"
    );
    let repeated_checkout_metadata = fs::metadata(&checkout).expect("repeated checkout metadata");
    assert_eq!(
        (
            repeated_checkout_metadata.dev(),
            repeated_checkout_metadata.ino()
        ),
        (checkout_metadata.dev(), checkout_metadata.ino()),
        "the existing checkout directory must be reused"
    );
    assert_eq!(symlink_identity(&command), command_identity);
    assert_eq!(symlink_identity(&extension), extension_identity);
    assert_eq!(symlink_identity(&package), package_identity);
    assert_eq!(symlink_identity(&show_me), show_me_identity);
    assert_eq!(symlink_identity(&grilling), grilling_identity);
    let mut cached = fs::read_dir(home.join(".cache/tool-sync"))
        .expect("cache directory")
        .map(|entry| entry.expect("cache entry").path())
        .collect::<Vec<_>>();
    cached.sort();
    assert_eq!(cached.as_slice(), std::slice::from_ref(&checkout));
    let report = normalize_private_prefix(&output_text(&second));
    assert!(report.contains("fetch repository"), "{report}");
    assert!(report.contains("install rag"), "{report}");
    assert!(!report.contains("clone "), "{report}");
    assert!(!report.contains("create directory"), "{report}");
    assert_eq!(
        normalize_private_prefix(
            &fs::read_to_string(fixture.record()).expect("repeated apply record")
        ),
        normalize_private_prefix(&format!("{0}|apply\n{0}|apply\n", checkout.display()))
    );
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
        assert!(
            report.contains(&format!("skip rag on {selected}")),
            "{report}"
        );
        for forbidden in [
            "clone ",
            "checkout ",
            "install ",
            "link command ",
            "link Pi extension ",
            "link Pi package ",
            "link skill ",
        ] {
            assert!(!report.contains(forbidden), "{report}");
        }
    }
    assert!(!home.exists());
    assert!(!fixture.record().exists());
}

#[test]
fn refuses_a_dirty_checkout_before_advancing_revision_or_mutating_home() {
    let fixture = Fixture::new();
    let home = fixture.home("dirty-home");
    fs::create_dir_all(&home).expect("dirty home");
    fs::write(home.join("notes.txt"), b"keep this home content\n").expect("dirty home notes");
    let home = canonical_path(&home);
    let first_manifest = fixture.manifest(&fixture.first_revision, "\"linux\"");
    let first = fixture.run(&home, &first_manifest, "linux", false);
    assert!(first.status.success(), "{}", error_text(&first));
    let checkout = fixture.checkout(&home);
    let dirty = checkout.join("uncommitted");
    fs::write(&dirty, "keep me").expect("dirty checkout file");
    let command = home.join(".local/bin/rag");
    let extension = home.join(".pi/agent/extensions/rag.ts");
    let package = home.join(".pi/agent/extensions/rag");
    let show_me = home.join(".agents/skills/show-me");
    let grilling = home.join(".agents/skills/grilling");
    let command_target = fs::read_link(&command).expect("command target");
    let extension_target = fs::read_link(&extension).expect("extension target");
    let package_target = fs::read_link(&package).expect("package target");
    let show_me_target = fs::read_link(&show_me).expect("show-me target");
    let grilling_target = fs::read_link(&grilling).expect("grilling target");
    let before = tree_snapshot(&home);
    let next_manifest = fixture.manifest(&fixture.second_revision, "\"linux\"");

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
        fs::read_to_string(home.join("notes.txt")).expect("notes retained"),
        "keep this home content\n"
    );
    assert_eq!(
        fs::read_link(command).expect("command retained"),
        command_target
    );
    assert_eq!(
        fs::read_link(extension).expect("extension retained"),
        extension_target
    );
    assert_eq!(
        fs::read_link(package).expect("package retained"),
        package_target
    );
    assert_eq!(
        fs::read_link(show_me).expect("show-me retained"),
        show_me_target
    );
    assert_eq!(
        fs::read_link(grilling).expect("grilling retained"),
        grilling_target
    );
    assert_eq!(tree_snapshot(&home), before);
    assert_eq!(
        normalize_private_prefix(&fs::read_to_string(fixture.record()).expect("dirty record")),
        normalize_private_prefix(&format!("{}|apply\n", checkout.display()))
    );
}

#[test]
fn refuses_a_non_symlink_collision_before_any_write() {
    let fixture = Fixture::new();
    let home = fixture.home("collision-home");
    fs::create_dir_all(&home).expect("collision home");
    fs::write(home.join("notes.txt"), b"keep this home content\n").expect("collision home notes");
    let home = canonical_path(&home);
    let collision = home.join(".pi/agent/extensions/rag");
    if let Some(parent) = collision.parent() {
        fs::create_dir_all(parent).expect("collision parent");
    }
    fs::write(&collision, "owned").expect("collision file");
    let before = tree_snapshot(&home);
    let manifest = fixture.manifest(&fixture.first_revision, "\"linux\"");

    let output = fixture.run(&home, &manifest, "linux", false);

    assert!(!output.status.success());
    let error = error_text(&output);
    assert!(error.contains("collides with a non-symlink"), "{error}");
    assert!(error.contains(path_text(&collision)), "{error}");
    assert_eq!(tree_snapshot(&home), before);
    assert!(!fixture.checkout(&home).exists());
    assert!(!fixture.record().exists());
}

#[test]
fn installs_an_embedded_tool_without_a_git_cache() {
    let fixture = Fixture::new();
    let home = fixture.home("embedded-home");
    fs::create_dir_all(&home).expect("embedded home");
    let embedded = fixture.repository.join("embedded");
    fs::create_dir_all(embedded.join("bin")).expect("embedded command directory");
    copy_executable(
        &fixture_file("install/checkout/install.sh"),
        &embedded.join("install.sh"),
    );
    copy_executable(
        &fixture_file("install/checkout/bin/rag"),
        &embedded.join("bin/embedded-rag"),
    );
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

// AGNT-0063.T04: proves the candidate replacement package (root pi_package, no
// skills) previews cleanly against an isolated home with a local fixture remote
// standing in for github.com/tintinweb/pi-subagents, without touching real $HOME
// and without any network access.
#[test]
fn previews_the_candidate_subagent_package_without_mutating_home() {
    let fixture = Fixture::new();
    let home = fixture.home("candidate-preview-home");
    fs::create_dir_all(&home).expect("candidate preview home");
    fs::write(home.join("seed.txt"), b"unchanged home content\n").expect("home seed");
    let home = canonical_path(&home);
    let before = tree_snapshot(&home);
    let manifest = fixture.candidate_manifest(&fixture.first_revision, "\"linux\"");

    let output = fixture.run(&home, &manifest, "linux", true);

    assert!(output.status.success(), "{}", error_text(&output));
    let report = normalize_private_prefix(&output_text(&output));
    let checkout = fixture.candidate_checkout(&home);
    let expected = vec![
        format!(
            "create directory {}",
            normalize_private_path_text(&home.join(".cache/tool-sync"))
        ),
        format!(
            "clone {} into {}",
            normalize_private_path_text(&fixture.remote_path()),
            normalize_private_path_text(&checkout)
        ),
        format!(
            "checkout {} in {}",
            fixture.first_revision,
            normalize_private_path_text(&checkout)
        ),
        format!(
            "install pi-subagents in {}: ./install.sh [\"preview\"]",
            normalize_private_path_text(&checkout)
        ),
        format!(
            "create directory {}",
            normalize_private_path_text(&home.join(".pi/agent/extensions"))
        ),
        format!(
            "link Pi package {} -> {}",
            normalize_private_path_text(&checkout.join(".")),
            normalize_private_path_text(&home.join(".pi/agent/extensions/pi-subagents"))
        ),
    ];
    assert_lines_in_order(&report, &expected);
    assert!(
        !report.contains("link skill "),
        "the candidate ships no skills directory, so preview must not plan a skill link: {report}"
    );
    assert!(!fixture.record().exists());
    assert_eq!(tree_snapshot(&home), before, "a dry-run preview must leave the home tree byte-identical");
}

// AGNT-0063.T04: proves the candidate package links and re-links deterministically,
// with no skills directory created and no other home content disturbed, in an
// isolated home distinct from the real $HOME.
#[test]
fn applies_and_repeats_the_candidate_subagent_package_link_without_skills() {
    let fixture = Fixture::new();
    let home = fixture.home("candidate-apply-home");
    fs::create_dir_all(&home).expect("candidate apply home");
    fs::write(home.join("notes.txt"), b"keep this home content\n").expect("home notes");
    let home = canonical_path(&home);
    let manifest = fixture.candidate_manifest(&fixture.first_revision, "\"linux\"");

    let first = fixture.run(&home, &manifest, "linux", false);

    assert!(first.status.success(), "{}", error_text(&first));
    let checkout = fixture.candidate_checkout(&home);
    assert_eq!(
        git_text(&checkout, &["rev-parse", "HEAD"]),
        fixture.first_revision
    );
    let package = home.join(".pi/agent/extensions/pi-subagents");
    assert_same_link_target(&package, &checkout.join("."));
    assert!(
        !home.join(".agents/skills").exists(),
        "a candidate manifest with no skills field must create no skills directory"
    );
    assert_eq!(
        fs::read_to_string(home.join("notes.txt")).expect("notes after apply"),
        "keep this home content\n",
        "unrelated home content must survive the candidate install untouched"
    );
    let checkout_metadata = fs::metadata(&checkout).expect("checkout metadata");
    let package_identity = symlink_identity(&package);

    let second = fixture.run(&home, &manifest, "linux", false);

    assert!(second.status.success(), "{}", error_text(&second));
    assert_eq!(
        git_text(&checkout, &["rev-parse", "HEAD"]),
        fixture.first_revision
    );
    assert_same_link_target(&package, &checkout.join("."));
    assert!(!home.join(".agents/skills").exists());
    assert_eq!(
        fs::read_to_string(home.join("notes.txt")).expect("notes after repeated apply"),
        "keep this home content\n"
    );
    let repeated_checkout_metadata = fs::metadata(&checkout).expect("repeated checkout metadata");
    assert_eq!(
        (repeated_checkout_metadata.dev(), repeated_checkout_metadata.ino()),
        (checkout_metadata.dev(), checkout_metadata.ino()),
        "the existing checkout directory must be reused, not re-cloned"
    );
    assert_eq!(
        symlink_identity(&package),
        package_identity,
        "the existing package link must be reused, not recreated"
    );
    let report = normalize_private_prefix(&output_text(&second));
    assert!(report.contains("fetch repository"), "{report}");
    assert!(!report.contains("clone "), "{report}");
    assert!(!report.contains("create directory"), "{report}");
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
