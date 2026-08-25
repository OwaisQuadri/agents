use std::path::{Path, PathBuf};
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_dispatch-baseline");

struct Fixture {
    root: PathBuf,
    dir: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Fixture {
        let root =
            std::env::temp_dir().join(format!("dispatch-baseline-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("repo");
        std::fs::create_dir_all(&dir).unwrap();
        let fixture = Fixture { root, dir };
        fixture.git(&["init", "--initial-branch=main"]);
        fixture.git(&["config", "user.email", "test@example.com"]);
        fixture.git(&["config", "user.name", "test"]);
        fixture.write("tracked.txt", "one\n");
        fixture.git(&["add", "."]);
        fixture.git(&["commit", "-m", "initial"]);
        fixture
    }

    fn git(&self, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(&self.dir)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn write(&self, name: &str, body: &str) {
        std::fs::write(self.dir.join(name), body).unwrap();
    }

    fn stamp(&self) -> PathBuf {
        self.stamp_from(&self.dir)
    }

    fn stamp_from(&self, repo: &Path) -> PathBuf {
        let path = self.root.join("stamp.json");
        let output = Command::new(BIN)
            .args(["stamp", "--repo"])
            .arg(repo)
            .arg("--out")
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        path
    }

    fn check(&self, stamp: &Path) -> (i32, String) {
        self.check_from(&self.dir, stamp)
    }

    fn check_from(&self, repo: &Path, stamp: &Path) -> (i32, String) {
        let output = Command::new(BIN)
            .args(["check", "--repo"])
            .arg(repo)
            .arg("--stamp")
            .arg(stamp)
            .arg("--json")
            .output()
            .unwrap();
        (
            output.status.code().unwrap(),
            String::from_utf8_lossy(&output.stdout).to_string(),
        )
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn dirty_at_stamp_and_unchanged_is_empty_delta() {
    let fixture = Fixture::new("dirty-unchanged");
    fixture.write("tracked.txt", "sibling edit\n");
    let stamp = fixture.stamp();
    let (code, out) = fixture.check(&stamp);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("\"modified\": []"), "{out}");
}

#[test]
fn untracked_work_products_at_stamp_are_not_delta() {
    let fixture = Fixture::new("untracked-at-stamp");
    fixture.write("report.md", "prior worker output\n");
    let stamp = fixture.stamp();
    let (code, out) = fixture.check(&stamp);
    assert_eq!(code, 0, "{out}");
    assert!(!out.contains("report.md"), "{out}");
}

#[test]
fn file_modified_after_stamp_is_delta() {
    let fixture = Fixture::new("modified-after");
    let stamp = fixture.stamp();
    fixture.write("tracked.txt", "this run wrote here\n");
    let (code, out) = fixture.check(&stamp);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("tracked.txt"), "{out}");
}

#[test]
fn untracked_file_created_after_stamp_is_delta() {
    let fixture = Fixture::new("untracked-after");
    let stamp = fixture.stamp();
    fixture.write("new.txt", "fresh\n");
    let (code, out) = fixture.check(&stamp);
    assert_eq!(code, 1, "{out}");
    assert!(
        out.contains("\"untracked\": [\n    \"new.txt\"\n  ]"),
        "{out}"
    );
}

#[test]
fn repository_subdirectory_uses_root_relative_hashes() {
    let fixture = Fixture::new("subdirectory");
    let subdirectory = fixture.dir.join("sub");
    std::fs::create_dir(&subdirectory).unwrap();
    fixture.write("sub/file.txt", "committed\n");
    fixture.git(&["add", "."]);
    fixture.git(&["commit", "-m", "subdirectory file"]);
    fixture.write("sub/file.txt", "dirty before stamp\n");
    let stamp = fixture.stamp_from(&subdirectory);
    fixture.write("sub/file.txt", "changed after stamp\n");
    let (code, out) = fixture.check_from(&subdirectory, &stamp);
    assert_eq!(
        code, 1,
        "an edit through a subdirectory must be a delta: {out}"
    );
    assert!(
        out.contains("sub/file.txt"),
        "the path must stay root-relative: {out}"
    );
}

#[test]
fn commit_after_stamp_reports_moved_ref_with_both_hashes() {
    let fixture = Fixture::new("moved-ref");
    let stamp = fixture.stamp();
    let before = fixture.git(&["rev-parse", "HEAD"]);
    fixture.write("tracked.txt", "two\n");
    fixture.git(&["add", "."]);
    fixture.git(&["commit", "-m", "sibling advance"]);
    let after = fixture.git(&["rev-parse", "HEAD"]);
    let (code, out) = fixture.check(&stamp);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains(&before) && out.contains(&after), "{out}");
    assert!(out.contains("\"name\": \"HEAD\""), "{out}");
    assert!(out.contains("\"modified\": []"), "{out}");
}

#[test]
fn clean_repo_with_no_activity_is_empty_delta() {
    let fixture = Fixture::new("clean");
    let stamp = fixture.stamp();
    let (code, out) = fixture.check(&stamp);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("\"moved_refs\": []"), "{out}");
}

// The 2026-08-25 hole: a status code alone cannot see an edit to a file that was already
// dirty, and an untracked work product is the normal case this tool exists for. Without a
// content hash the verifier's own fix reflex reports a clean delta.
#[test]
fn edit_to_an_already_untracked_file_is_delta() {
    let fixture = Fixture::new("edit-untracked");
    fixture.write("product.json", "{\"tasks\":[]}\n");
    let stamp = fixture.stamp();
    fixture.write("product.json", "{\"tasks\":[],\"fixed\":true}\n");
    let (code, out) = fixture.check(&stamp);
    assert_eq!(
        code, 1,
        "an edit to an untracked file must be a delta: {out}"
    );
    assert!(
        out.contains("product.json"),
        "the delta must name the file: {out}"
    );
}

#[test]
fn edit_to_an_already_modified_tracked_file_is_delta() {
    let fixture = Fixture::new("edit-modified");
    fixture.write("tracked.txt", "a sibling's uncommitted edit\n");
    let stamp = fixture.stamp();
    fixture.write("tracked.txt", "a sibling's edit, then ours on top\n");
    let (code, out) = fixture.check(&stamp);
    assert_eq!(
        code, 1,
        "a second edit to the same file must be a delta: {out}"
    );
    assert!(
        out.contains("tracked.txt"),
        "the delta must name the file: {out}"
    );
}
