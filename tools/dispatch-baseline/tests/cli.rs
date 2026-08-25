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
        assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn write(&self, name: &str, body: &str) {
        std::fs::write(self.dir.join(name), body).unwrap();
    }

    fn stamp(&self) -> PathBuf {
        let path = self.root.join("stamp.json");
        let output = Command::new(BIN)
            .args(["stamp", "--repo"])
            .arg(&self.dir)
            .arg("--out")
            .arg(&path)
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        path
    }

    fn check(&self, stamp: &Path) -> (i32, String) {
        let output = Command::new(BIN)
            .args(["check", "--repo"])
            .arg(&self.dir)
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
    assert!(out.contains("new.txt"), "{out}");
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
