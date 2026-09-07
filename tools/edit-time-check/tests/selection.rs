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
fn repository_configuration_cannot_remove_checks_or_reduce_coverage() {
    let fixture = Fixture::new();
    std::fs::create_dir(fixture.0.join(".git")).unwrap();
    let global = fixture.0.join("global.toml");
    std::fs::write(&global, "version=1").unwrap();
    let options = Options::for_paths(global, fixture.0.join("comments"), fixture.0.join("code"));
    let target = fixture.0.join("source.rs");
    let root = fixture.0.to_str().unwrap();
    for text in [
        "version=1\nrules=[]",
        "version=1\nrules=['privacy','comment-length','boolean-name']",
        "version=1\ninclude=[]",
        "version=1\ninclude=['src/**']",
        "version=1\nexclude=['**/private/**']",
        "version=1\ngenerated=['**/*.rs']",
        "version=1\ninclude=['**/*']",
    ] {
        std::fs::write(fixture.0.join(".edit-time.toml"), text).unwrap();
        assert!(options.resolve(&target, Some(root)).is_err(), "{text}");
    }
}

#[test]
fn repository_configuration_preserves_or_strengthens_installed_settings() {
    let fixture = Fixture::new();
    std::fs::create_dir(fixture.0.join(".git")).unwrap();
    let global = fixture.0.join("global.toml");
    let installed = "version=1\nrules=['privacy']\ninclude=['src/**']\nexclude=['src/vendor/**']\ngenerated=['src/*.generated.rs']\ntotal_ms=1000\nprocess_budget_ms=800\n[rule_ms]\nprivacy=100";
    std::fs::write(&global, installed).unwrap();
    let options = Options::for_paths(global, fixture.0.join("comments"), fixture.0.join("code"));
    let target = fixture.0.join("source.rs");
    let root = fixture.0.to_str().unwrap();
    for text in ["version=1", installed] {
        std::fs::write(fixture.0.join(".edit-time.toml"), text).unwrap();
        let config = options.resolve(&target, Some(root)).unwrap().0;
        assert_eq!(config.rules, [edit_time_check::Rule::Privacy]);
        assert_eq!(config.include, ["src/**"]);
        assert!(!config.is_selected("source.rs").unwrap());
        assert!(!config.is_selected("src/vendor/a.rs").unwrap());
        assert!(!config.is_selected("src/a.generated.rs").unwrap());
        assert_eq!(config.total_ms, 1000);
        assert_eq!(config.process_budget_ms, 800);
        assert_eq!(config.rule_limit(edit_time_check::Rule::Privacy), 100);
    }
    std::fs::write(
        fixture.0.join(".edit-time.toml"),
        "version=1\nrules=['boolean-name','privacy']\ninclude=['tests/**','src/**']\nexclude=[]\ngenerated=[]\ntotal_ms=900\nprocess_budget_ms=700\n[rule_ms]\nprivacy=90",
    )
    .unwrap();
    let config = options.resolve(&target, Some(root)).unwrap().0;
    assert_eq!(
        config.rules,
        [
            edit_time_check::Rule::BooleanName,
            edit_time_check::Rule::Privacy
        ]
    );
    for path in ["tests/a.rs", "src/vendor/a.rs", "src/a.generated.rs"] {
        assert!(config.is_selected(path).unwrap(), "{path}");
    }
    assert_eq!(config.total_ms, 900);
    assert_eq!(config.process_budget_ms, 700);
    assert_eq!(config.rule_limit(edit_time_check::Rule::Privacy), 90);
    for text in [
        "broken",
        "version=2",
        "version=1\ncommand='true'",
        "version=1\nrules=['privacy','missing']",
        "version=1\nrules=['privacy','privacy']",
        "version=1\ninclude=['src/**','[']",
        "version=1\ninclude=['**']",
        "version=1\nexclude=['src/vendor/nested/**']",
        "version=1\ngenerated=['src/a.generated.rs']",
        "version=1\ntotal_ms=1001",
        "version=1\nprocess_budget_ms=801",
        "version=1\n[rule_ms]\nprivacy=101",
    ] {
        std::fs::write(fixture.0.join(".edit-time.toml"), text).unwrap();
        assert!(options.resolve(&target, Some(root)).is_err(), "{text}");
    }
}

#[test]
fn installed_configuration_retains_selection_replacement() {
    let config =
        Config::parse("version=1\nrules=[]\ninclude=[]\nexclude=[]\ngenerated=[]").unwrap();
    assert!(config.rules.is_empty());
    assert!(config.include.is_empty());
    assert!(config.exclude.is_empty());
    assert!(config.generated.is_empty());
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
        "version=1\ntotal_ms=1000\n[rule_ms]\nprivacy=100",
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
