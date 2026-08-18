use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
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
}

impl Fixture {
    fn new() -> Self {
        let root = fixture_root("tool-sync-pi-resources");
        let repository = root.join("repository");
        let work = root.join("work");
        let remote = root.join("remote.git");
        seed_source_tree(&repository, "pi-resources/repository");
        seed_source_tree(&work, "pi-resources");
        fs::write(work.join("revision"), b"first revision\n").expect("first revision file");
        git(&work, &["init", "-q"]);
        git(&work, &["config", "user.email", "fixture@example.test"]);
        git(&work, &["config", "user.name", "Fixture"]);
        git(&work, &["add", "."]);
        git(&work, &["commit", "-qm", "first"]);
        let first_revision = git_text(&work, &["rev-parse", "HEAD"]);
        let first_revision_contents =
            fs::read(work.join("revision")).expect("first revision contents");
        fs::write(work.join("revision"), b"second revision\n").expect("second revision file");
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
            first_revision_contents,
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
commands = []
pi_package = "pi/packages/rag"
skills = ["skills/show-me", "skills/grilling"]
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

fn seed_source_tree(root: &Path, source: &str) {
    copy_tree(
        &fixture_file(&format!("{source}/pi/packages/rag")),
        &root.join("pi/packages/rag"),
    );
    copy_tree(
        &fixture_file(&format!("{source}/skills/show-me")),
        &root.join("skills/show-me"),
    );
    copy_tree(
        &fixture_file(&format!("{source}/skills/grilling")),
        &root.join("skills/grilling"),
    );
    copy_executable(
        &fixture_file(&format!("{source}/install.sh")),
        &root.join("install.sh"),
    );
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
    } else {
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).expect("fixture parent directory");
        }
        fs::copy(source, destination).expect("copy fixture file");
    }
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

fn normalize_private_prefix(text: &str) -> String {
    text.replace("/private", "")
}

fn normalize_private_path_text(path: &Path) -> String {
    normalize_private_prefix(&path.display().to_string())
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
fn installs_pi_package_and_skills_in_a_stable_order() {
    let fixture = Fixture::new();
    assert_ne!(fixture.first_revision, fixture.second_revision);
    let home = fixture.home("resource-home");
    fs::create_dir_all(&home).expect("home directory");
    assert_ne!(
        fs::read_to_string(fixture_file(
            "pi-resources/repository/pi/packages/rag/README.md"
        ))
        .expect("repository package fixture"),
        fs::read_to_string(fixture_file("pi-resources/pi/packages/rag/README.md"))
            .expect("checkout package fixture")
    );
    assert_ne!(
        fs::read_to_string(fixture_file(
            "pi-resources/repository/skills/show-me/README.md"
        ))
        .expect("repository show-me fixture"),
        fs::read_to_string(fixture_file("pi-resources/skills/show-me/README.md"))
            .expect("checkout show-me fixture")
    );
    assert_ne!(
        fs::read_to_string(fixture_file(
            "pi-resources/repository/skills/grilling/README.md"
        ))
        .expect("repository grilling fixture"),
        fs::read_to_string(fixture_file("pi-resources/skills/grilling/README.md"))
            .expect("checkout grilling fixture")
    );
    assert_ne!(
        fs::read_to_string(fixture_file("pi-resources/repository/install.sh"))
            .expect("repository install fixture"),
        fs::read_to_string(fixture_file("pi-resources/install.sh"))
            .expect("checkout install fixture")
    );
    let notes = home.join("notes.txt");
    fs::write(&notes, b"keep this home content\n").expect("home notes");
    let before = tree_snapshot(&home);
    let manifest = fixture.manifest(&fixture.first_revision, "\"linux\"");
    let checkout = fixture.checkout(&home);
    let package_source = checkout.join("pi/packages/rag");
    let show_me_source = checkout.join("skills/show-me");
    let grilling_source = checkout.join("skills/grilling");

    let dry_run = fixture.run(&home, &manifest, "linux", true);

    assert!(dry_run.status.success(), "{}", error_text(&dry_run));
    assert_eq!(tree_snapshot(&home), before);
    assert!(!fixture.record().exists());
    assert_eq!(
        normalize_private_prefix(&output_text(&dry_run))
            .lines()
            .collect::<Vec<_>>(),
        vec![
            format!(
                "create directory {}",
                home.join(".cache/tool-sync").display()
            ),
            format!(
                "clone {} into {}",
                fixture.remote.display(),
                checkout.display()
            ),
            format!(
                "checkout {} in {}",
                fixture.first_revision,
                checkout.display()
            ),
            format!(
                "install rag in {}: ./install.sh [\"preview\"]",
                checkout.display()
            ),
            format!(
                "create directory {}",
                home.join(".pi/agent/extensions").display()
            ),
            format!(
                "link Pi package {} -> {}",
                package_source.display(),
                home.join(".pi/agent/extensions/rag").display()
            ),
            format!("create directory {}", home.join(".agents/skills").display()),
            format!(
                "link skill {} -> {}",
                show_me_source.display(),
                home.join(".agents/skills/show-me").display()
            ),
            format!(
                "link skill {} -> {}",
                grilling_source.display(),
                home.join(".agents/skills/grilling").display()
            ),
        ]
    );

    let apply = fixture.run(&home, &manifest, "linux", false);

    assert!(apply.status.success(), "{}", error_text(&apply));
    let checkout_revision = git_text(&checkout, &["rev-parse", "HEAD"]);
    assert_eq!(checkout_revision, fixture.first_revision);
    assert_eq!(
        fs::read(checkout.join("revision")).expect("checkout revision file"),
        fixture.first_revision_contents
    );
    assert_eq!(
        normalize_private_path_text(
            &fs::read_link(home.join(".pi/agent/extensions/rag")).expect("package link")
        ),
        normalize_private_path_text(&package_source)
    );
    assert_eq!(
        normalize_private_path_text(
            &fs::read_link(home.join(".agents/skills/show-me")).expect("show-me link")
        ),
        normalize_private_path_text(&show_me_source)
    );
    assert_eq!(
        normalize_private_path_text(
            &fs::read_link(home.join(".agents/skills/grilling")).expect("grilling link")
        ),
        normalize_private_path_text(&grilling_source)
    );
    assert_eq!(
        fs::read(&notes).expect("notes after apply"),
        b"keep this home content\n"
    );
    assert_eq!(
        normalize_private_prefix(&fs::read_to_string(fixture.record()).expect("apply record")),
        normalize_private_prefix(&format!("{}|apply\n", checkout.display()))
    );
    assert_eq!(
        normalize_private_prefix(&output_text(&apply))
            .lines()
            .collect::<Vec<_>>(),
        vec![
            format!(
                "create directory {}",
                home.join(".cache/tool-sync").display()
            ),
            format!(
                "clone {} into {}",
                fixture.remote.display(),
                checkout.display()
            ),
            format!(
                "checkout {} in {}",
                fixture.first_revision,
                checkout.display()
            ),
            format!(
                "install rag in {}: ./install.sh [\"apply\"]",
                checkout.display()
            ),
            format!(
                "create directory {}",
                home.join(".pi/agent/extensions").display()
            ),
            format!(
                "link Pi package {} -> {}",
                package_source.display(),
                home.join(".pi/agent/extensions/rag").display()
            ),
            format!("create directory {}", home.join(".agents/skills").display()),
            format!(
                "link skill {} -> {}",
                show_me_source.display(),
                home.join(".agents/skills/show-me").display()
            ),
            format!(
                "link skill {} -> {}",
                grilling_source.display(),
                home.join(".agents/skills/grilling").display()
            ),
        ]
    );

    let repeated = fixture.run(&home, &manifest, "linux", false);

    assert!(repeated.status.success(), "{}", error_text(&repeated));
    assert_eq!(
        git_text(&checkout, &["rev-parse", "HEAD"]),
        fixture.first_revision
    );
    assert_eq!(
        fs::read(checkout.join("revision")).expect("repeated checkout revision file"),
        fixture.first_revision_contents
    );
    assert_eq!(
        normalize_private_path_text(
            &fs::read_link(home.join(".pi/agent/extensions/rag")).expect("repeated package link")
        ),
        normalize_private_path_text(&package_source)
    );
    assert_eq!(
        normalize_private_path_text(
            &fs::read_link(home.join(".agents/skills/show-me")).expect("repeated show-me link")
        ),
        normalize_private_path_text(&show_me_source)
    );
    assert_eq!(
        normalize_private_path_text(
            &fs::read_link(home.join(".agents/skills/grilling")).expect("repeated grilling link")
        ),
        normalize_private_path_text(&grilling_source)
    );
    assert_eq!(
        fs::read(&notes).expect("notes after repeated apply"),
        b"keep this home content\n"
    );
    assert_eq!(
        normalize_private_prefix(
            &fs::read_to_string(fixture.record()).expect("repeated apply record")
        ),
        normalize_private_prefix(&format!(
            "{}|apply\n{}|apply\n",
            checkout.display(),
            checkout.display()
        ))
    );
    assert_eq!(
        normalize_private_prefix(&output_text(&repeated))
            .lines()
            .collect::<Vec<_>>(),
        vec![
            format!("fetch repository {}", checkout.display()),
            format!(
                "checkout {} in {}",
                fixture.first_revision,
                checkout.display()
            ),
            format!(
                "install rag in {}: ./install.sh [\"apply\"]",
                checkout.display()
            ),
            format!(
                "link Pi package {} -> {}",
                package_source.display(),
                home.join(".pi/agent/extensions/rag").display()
            ),
            format!(
                "link skill {} -> {}",
                show_me_source.display(),
                home.join(".agents/skills/show-me").display()
            ),
            format!(
                "link skill {} -> {}",
                grilling_source.display(),
                home.join(".agents/skills/grilling").display()
            ),
        ]
    );
}
