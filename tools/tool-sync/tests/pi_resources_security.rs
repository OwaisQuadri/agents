use std::collections::BTreeMap;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

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
        let root = fixture_root("tool-sync-pi-resources-security");
        let repository = root.join("repository");
        let work = root.join("work");
        let remote = root.join("remote.git");

        fs::create_dir_all(repository.join("pi/extensions")).expect("repository fixture");
        fs::write(
            repository.join("pi/extensions/rag.ts"),
            "export const fixture = \"tool-sync integration extension\";\n",
        )
        .expect("Pi extension fixture");
        fs::create_dir_all(repository.join("bundle")).expect("embedded source fixture");
        fs::create_dir_all(root.join("outside")).expect("outside directory");
        copy_executable(
            &fixture_file("installer.sh"),
            &repository.join("bundle/install.sh"),
        );

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

    fn checkout(&self, home: &Path) -> PathBuf {
        home.join(".cache/tool-sync/rag")
    }

    fn manifest(&self, name: &str, body: &str) -> PathBuf {
        let manifest = self.root.join(format!("{name}.toml"));
        fs::write(&manifest, body).expect("manifest");
        manifest
    }

    fn command(&self, home: &Path, manifest: &Path, platform: &str, is_dry_run: bool) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_tool-sync"));
        command
            .env("TOOL_SYNC_RECORD", self.record())
            .args(["--home", path_text(home)])
            .args(["--manifest", path_text(manifest)])
            .args(["--repository-root", path_text(&self.repository)])
            .args(["--platform", platform]);
        if is_dry_run {
            command.arg("--dry-run");
        }
        command
    }

    fn record(&self) -> PathBuf {
        self.root.join("record.log")
    }

    fn protected_snapshot(&self, home: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        snapshot_trees(&[("repository", &self.repository), ("home", home)])
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct ChildGuard {
    child: Option<Child>,
}

impl ChildGuard {
    fn new(child: Child) -> Self {
        Self { child: Some(child) }
    }

    fn wait(&mut self) {
        self.child
            .as_mut()
            .expect("spawned helper")
            .wait()
            .expect("wait for helper");
        self.child.take();
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
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

fn write_executable(destination: &Path, content: &str) {
    fs::write(destination, content).expect("write executable fixture");
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

fn error_text(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("tool-sync error is UTF-8")
}

fn snapshot_trees(roots: &[(&str, &Path)]) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, prefix: &Path, path: &Path, entries: &mut BTreeMap<PathBuf, Vec<u8>>) {
        let mut children = fs::read_dir(path)
            .expect("read snapshot directory")
            .map(|entry| entry.expect("snapshot entry").path())
            .collect::<Vec<_>>();
        children.sort();
        for child in children {
            let relative = child.strip_prefix(root).expect("snapshot relative path");
            let metadata = fs::symlink_metadata(&child).expect("snapshot metadata");
            let key = prefix.join(relative);
            if metadata.file_type().is_symlink() {
                entries.insert(
                    key,
                    fs::read_link(&child)
                        .expect("snapshot link")
                        .into_os_string()
                        .into_encoded_bytes(),
                );
            } else if metadata.is_dir() {
                entries.insert(key.clone(), Vec::new());
                visit(root, prefix, &child, entries);
            } else {
                entries.insert(key, fs::read(&child).expect("snapshot file"));
            }
        }
    }

    let mut entries = BTreeMap::new();
    for (name, root) in roots {
        if root.exists() {
            visit(root, Path::new(name), root, &mut entries);
        }
    }
    entries
}

fn real_git_path() -> PathBuf {
    let output = Command::new("bash")
        .args(["-lc", "command -v git"])
        .output()
        .expect("discover local Git");
    assert!(output.status.success(), "command -v git failed");
    PathBuf::from(
        String::from_utf8(output.stdout)
            .expect("Git path is UTF-8")
            .trim(),
    )
}

#[test]
fn rejects_a_source_path_that_escapes_the_repository() {
    let fixture = Fixture::new();
    let home = fixture.home("escape-home");
    fs::create_dir_all(fixture.root.join("outside")).expect("outside directory");
    std::os::unix::fs::symlink("../outside", fixture.repository.join("escape"))
        .expect("escape symlink");
    let manifest = fixture.manifest(
        "escape",
        &format!(
            r#"[[tools]]
name = "rag"
platforms = ["linux"]
commands = []
pi_extension = "pi/extensions/rag.ts"
source = {{ path = "escape" }}
installer = {{ command = "./install.sh", args = ["apply"], preview_args = ["preview"] }}
"#
        ),
    );
    let before = fixture.protected_snapshot(&home);

    let output = fixture.command(&home, &manifest, "linux", true).output().expect("run tool-sync");

    assert!(!output.status.success());
    let error = error_text(&output);
    assert!(error.contains("outside repository"), "{error}");
    assert_eq!(before, fixture.protected_snapshot(&home));
    assert!(!fixture.record().exists());
}

#[test]
fn rejects_duplicate_pi_extension_destinations() {
    let fixture = Fixture::new();
    let home = fixture.home("duplicate-home");
    let manifest = fixture.manifest(
        "duplicate",
        &format!(
            r#"[[tools]]
name = "alpha"
platforms = ["linux"]
commands = []
pi_extension = "pi/extensions/rag.ts"
source = {{ path = "bundle" }}
installer = {{ command = "./install.sh", args = ["apply"], preview_args = ["preview"] }}

[[tools]]
name = "beta"
platforms = ["linux"]
commands = []
pi_extension = "other/rag.ts"
source = {{ path = "bundle" }}
installer = {{ command = "./install.sh", args = ["apply"], preview_args = ["preview"] }}
"#
        ),
    );
    let before = fixture.protected_snapshot(&home);

    let output = fixture.command(&home, &manifest, "linux", true).output().expect("run tool-sync");

    assert!(!output.status.success());
    let error = error_text(&output);
    assert!(error.contains("Pi extension rag.ts is duplicated"), "{error}");
    assert_eq!(before, fixture.protected_snapshot(&home));
    assert!(!fixture.record().exists());
}

#[test]
fn refuses_a_dirty_checkout_before_fetching_or_linking() {
    let fixture = Fixture::new();
    let home = fixture.home("dirty-home");
    let manifest = fixture.manifest(
        "dirty",
        &format!(
            r#"[[tools]]
name = "rag"
platforms = ["linux"]
commands = ["bin/rag"]
pi_extension = "pi/extensions/rag.ts"
source = {{ url = {:?}, revision = {:?} }}
installer = {{ command = "./install.sh", args = ["apply"], preview_args = ["preview"] }}
"#,
            path_text(&fixture.remote),
            fixture.first_revision,
        ),
    );

    let first = fixture.command(&home, &manifest, "linux", false).output().expect("run tool-sync");
    assert!(first.status.success(), "{}", error_text(&first));
    let checkout = fixture.checkout(&home);
    assert_eq!(
        fs::read_to_string(fixture.record()).expect("installer invocations"),
        format!(
            "{}|apply\n",
            fs::canonicalize(&checkout)
                .expect("canonical checkout")
                .display()
        )
    );
    let dirty = checkout.join("uncommitted");
    fs::write(&dirty, "keep me").expect("dirty checkout file");
    let before = fixture.protected_snapshot(&home);

    let second = fixture.command(&home, &manifest, "linux", false).output().expect("run tool-sync");

    assert!(!second.status.success());
    let error = error_text(&second);
    assert!(error.contains("dirty"), "{error}");
    assert!(error.contains(path_text(&checkout)), "{error}");
    assert_eq!(before, fixture.protected_snapshot(&home));
    assert_eq!(
        fs::read_to_string(fixture.record()).expect("installer invocations"),
        format!(
            "{}|apply\n",
            fs::canonicalize(&checkout)
                .expect("canonical checkout")
                .display()
        )
    );
}

#[test]
fn refuses_a_non_symlink_pi_extension_collision() {
    let fixture = Fixture::new();
    let home = fixture.home("collision-home");
    fs::create_dir_all(home.join(".pi/agent/extensions")).expect("extension directory");
    fs::write(home.join(".pi/agent/extensions/rag.ts"), "owned").expect("collision file");
    let manifest = fixture.manifest(
        "collision",
        &format!(
            r#"[[tools]]
name = "rag"
platforms = ["linux"]
commands = []
pi_extension = "pi/extensions/rag.ts"
source = {{ path = "bundle" }}
installer = {{ command = "./install.sh", args = ["apply"], preview_args = ["preview"] }}
"#
        ),
    );
    let before = fixture.protected_snapshot(&home);

    let output = fixture.command(&home, &manifest, "linux", false).output().expect("run tool-sync");

    assert!(!output.status.success());
    let error = error_text(&output);
    assert!(error.contains("collides with a non-symlink"), "{error}");
    assert_eq!(before, fixture.protected_snapshot(&home));
    assert!(!fixture.record().exists());
}

#[test]
fn fails_closed_when_the_installer_exits_non_zero() {
    let fixture = Fixture::new();
    let home = fixture.home("installer-home");
    write_executable(
        &fixture.repository.join("bundle/install.sh"),
        "#!/bin/sh\nset -eu\nprintf '%s|%s\\n' \"$PWD\" \"${1:-}\" >> \"$TOOL_SYNC_RECORD\"\nexit 23\n",
    );
    let manifest = fixture.manifest(
        "installer",
        &format!(
            r#"[[tools]]
name = "rag"
platforms = ["linux"]
commands = []
pi_extension = "pi/extensions/rag.ts"
source = {{ path = "bundle" }}
installer = {{ command = "./install.sh", args = ["apply"], preview_args = ["preview"] }}
"#
        ),
    );
    let before = fixture.protected_snapshot(&home);

    let output = fixture.command(&home, &manifest, "linux", false).output().expect("run tool-sync");

    assert!(!output.status.success());
    let error = error_text(&output);
    assert!(error.contains("installer for rag failed"), "{error}");
    assert_eq!(before, fixture.protected_snapshot(&home));
    let record = fs::read_to_string(fixture.record()).expect("installer record");
    assert_eq!(
        record,
        format!(
            "{}|apply\n",
            fs::canonicalize(fixture.repository.join("bundle"))
                .expect("canonical bundle")
                .display()
        )
    );
}

#[test]
fn fails_closed_when_checkout_state_becomes_dirty_after_planning() {
    let fixture = Fixture::new();
    let home = fixture.home("stale-home");
    let manifest = fixture.manifest(
        "stale",
        &format!(
            r#"[[tools]]
name = "rag"
platforms = ["linux"]
commands = ["bin/rag"]
pi_extension = "pi/extensions/rag.ts"
source = {{ url = {:?}, revision = {:?} }}
installer = {{ command = "./install.sh", args = ["apply"], preview_args = ["preview"] }}
"#,
            path_text(&fixture.remote),
            fixture.second_revision,
        ),
    );

    let first = fixture.command(&home, &manifest, "linux", false).output().expect("run tool-sync");
    assert!(first.status.success(), "{}", error_text(&first));
    let checkout = fixture.checkout(&home);
    let before = fixture.protected_snapshot(&home);

    let runtime = fixture.root.join("runtime");
    fs::create_dir_all(&runtime).expect("runtime directory");
    let count_file = runtime.join("git-count");
    let marker_file = runtime.join("git-ready");
    let wrapper = runtime.join("git");
    let dirty = checkout.join("transient-dirty");
    let real_git = real_git_path();
    write_executable(
        &wrapper,
        &format!(
            "#!/bin/sh\nset -eu\ncount_file=${{GIT_COUNT_FILE:?}}\nmarker_file=${{GIT_MARKER_FILE:?}}\nreal_git=${{REAL_GIT:?}}\nif [ \"${{1:-}}\" = \"-C\" ] && [ \"${{3:-}}\" = \"status\" ] && [ \"${{4:-}}\" = \"--porcelain\" ] && [ \"${{5:-}}\" = \"--untracked-files=all\" ]; then\n  count=0\n  if [ -f \"$count_file\" ]; then\n    count=$(cat \"$count_file\")\n  fi\n  count=$((count + 1))\n  printf '%s\\n' \"$count\" > \"$count_file\"\n  if [ \"$count\" -eq 1 ]; then\n    : > \"$marker_file\"\n  elif [ \"$count\" -eq 2 ]; then\n    sleep 0.25\n  fi\nfi\nexec \"$real_git\" \"$@\"\n",
        ),
    );
    let mut dirtyer = ChildGuard::new(
        Command::new("sh")
            .arg("-c")
            .arg(format!(
                "while [ ! -e {marker} ]; do sleep 0.01; done; sleep 0.05; printf dirty > {dirty}; sleep 0.25; rm -f {dirty}",
                marker = shell_escape(marker_file.as_path()),
                dirty = shell_escape(dirty.as_path())
            ))
            .spawn()
            .expect("spawn dirtyer"),
    );

    let mut command = fixture.command(&home, &manifest, "linux", false);
    let mut path = OsString::from(path_text(&runtime));
    path.push(":");
    if let Some(existing) = env::var_os("PATH") {
        path.push(existing);
    }
    command
        .env("PATH", path)
        .env("REAL_GIT", real_git)
        .env("GIT_COUNT_FILE", &count_file)
        .env("GIT_MARKER_FILE", &marker_file);
    let output = command.output().expect("run tool-sync");
    dirtyer.wait();

    assert!(!output.status.success());
    let error = error_text(&output);
    assert!(error.contains("repository became dirty after planning"), "{error}");
    assert_eq!(before, fixture.protected_snapshot(&home));
    assert_eq!(
        fs::read_to_string(fixture.record()).expect("installer invocations"),
        format!(
            "{}|apply\n",
            fs::canonicalize(&checkout)
                .expect("canonical checkout")
                .display()
        )
    );
}

fn shell_escape(path: &Path) -> String {
    let text = path_text(path);
    if text.contains('\'') {
        panic!("test path contains a single quote: {text}");
    }
    format!("'{}'", text)
}
