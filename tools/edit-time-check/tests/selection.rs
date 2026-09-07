use edit_time_check::{Config, Options};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "edit-time-selection-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(std::fs::canonicalize(path).unwrap())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn patterns_match_components_not_substrings_and_unknown_config_blocks() {
    let config = Config::parse(include_str!("../../../config/edit-time.toml")).unwrap();
    for path in [
        "node_modules/x.ts",
        "src/target/x.rs",
        ".cache/file",
        "dist/a.ts",
        "x.generated.ts",
    ] {
        assert!(!config.is_selected(path).unwrap(), "{path}");
    }
    for path in [
        "src/node_modules_notes.ts",
        "targeting.rs",
        "README.md",
        "src/main.rs",
    ] {
        assert!(config.is_selected(path).unwrap(), "{path}");
    }
    for text in [
        "version=2",
        "version=1\ncommand='true'",
        "version=1\nrules=['missing']",
        "version=1\ninclude=['[']",
        "version=1\ntotal_ms=20001",
        "version=1\n[rule_ms]\nprivacy=501",
        "version=1\nrules=['privacy','privacy']",
    ] {
        assert!(Config::parse(text).is_err(), "{text}");
    }
}

#[test]
fn target_repository_is_authoritative_and_bad_present_override_blocks() {
    let fixture = Fixture::new();
    std::fs::create_dir(fixture.0.join(".git")).unwrap();
    let global = fixture.0.join("global.toml");
    std::fs::write(&global, "version=1\n").unwrap();
    let options = Options::for_paths(global, fixture.0.join("comments"), fixture.0.join("code"));
    let target = fixture.0.join("source.rs");
    let root = fixture.0.to_str().unwrap();
    assert!(options.resolve(&target, Some(root)).is_ok());
    let nested = fixture.0.join("new/deep/source.rs");
    assert!(options.resolve(&nested, Some(root)).is_ok());
    assert!(!fixture.0.join("new").exists());
    assert!(options.resolve(&target, None).is_err());
    std::fs::write(
        fixture.0.join(".edit-time.toml"),
        "version=1\nrules=['privacy']\ntotal_ms=1000\n[rule_ms]\nprivacy=100",
    )
    .unwrap();
    let config = options.resolve(&target, Some(root)).unwrap().0;
    assert_eq!(config.total_ms, 1000);
    assert_eq!(config.rule_limit(edit_time_check::Rule::Privacy), 100);
    std::fs::write(fixture.0.join(".edit-time.toml"), "broken").unwrap();
    assert!(options.resolve(&target, Some(root)).is_err());
    std::fs::remove_file(fixture.0.join(".edit-time.toml")).unwrap();
    std::fs::create_dir(fixture.0.join(".edit-time.toml")).unwrap();
    assert!(options.resolve(&target, Some(root)).is_err());
}
