use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::Value;

static SCRATCH_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
    bin: PathBuf,
    log: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "skill-eval-model-capabilities-{}-{}",
            std::process::id(),
            SCRATCH_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("config")).unwrap();
        fs::create_dir(root.join("skills")).unwrap();
        fs::write(root.join("config/model-tiers.json"), "{}\n").unwrap();
        let bin = root.join("bin");
        fs::create_dir(&bin).unwrap();
        fs::copy(
            env!("CARGO_BIN_EXE_fake-pi-model-capabilities"),
            bin.join("pi"),
        )
        .unwrap();
        make_executable(&bin.join("pi"));
        Self {
            log: root.join("pi.log"),
            root,
            bin,
        }
    }

    fn run(&self, output: &str, scenario: &str) -> Output {
        let path = std::env::var_os("PATH").unwrap_or_default();
        let paths = std::iter::once(self.bin.clone()).chain(std::env::split_paths(&path));
        Command::new(env!("CARGO_BIN_EXE_skill-eval"))
            .args(["model-capabilities", "--output", output])
            .current_dir(&self.root)
            .env("PATH", std::env::join_paths(paths).unwrap())
            .env("FAKE_PI_LOG", &self.log)
            .env("FAKE_PI_SCENARIO", scenario)
            .output()
            .unwrap()
    }

    fn preview(&self, path: &str) -> Output {
        Command::new(env!("CARGO_BIN_EXE_skill-eval"))
            .args([
                "t1-screen-preview",
                "--capabilities",
                path,
                "--format",
                "json",
            ])
            .current_dir(&self.root)
            .output()
            .unwrap()
    }

    fn snapshot(&self, path: &str) -> Value {
        serde_json::from_slice(&fs::read(self.root.join(path)).unwrap()).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}

#[test]
fn full_union_inventory_is_stable_complete_and_redacted() {
    let fixture = Fixture::new();
    let output = fixture.run("snapshots/catalog.json", "valid");
    assert_success(&output);
    let snapshot = fixture.snapshot("snapshots/catalog.json");

    assert_eq!(snapshot["snapshot_version"], 1);
    assert_eq!(snapshot["pi_version"], "synthetic-pi-1.2.3");
    assert_eq!(snapshot["counts"]["union"], 6);
    assert_eq!(snapshot["counts"]["all_extension_list"], 5);
    assert_eq!(snapshot["counts"]["core_rpc"], 5);
    assert_eq!(snapshot["counts"]["list_only"], 1);
    assert_eq!(snapshot["counts"]["rpc_only"], 1);
    assert_eq!(snapshot["counts"]["moving_aliases"], 1);
    assert_eq!(
        snapshot["probe_commands"][0],
        serde_json::json!({
            "purpose": "all_extension_availability",
            "program": "pi",
            "arguments": ["--list-models"],
            "extension_mode": "normal_deployed_discovery",
            "request_id": null,
            "request_type": null
        })
    );
    assert_eq!(
        snapshot["probe_commands"][1]["request_id"],
        "skill-eval-models"
    );
    assert_eq!(
        snapshot["probe_commands"][1]["request_type"],
        "get_available_models"
    );

    let identities = snapshot["models"]
        .as_array()
        .unwrap()
        .iter()
        .map(|model| {
            format!(
                "{}/{}",
                model["provider"].as_str().unwrap(),
                model["model"].as_str().unwrap()
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        identities,
        [
            "anthropic/core-default",
            "anthropic/core-holey",
            "core/rpc-only",
            "extension/list-only",
            "openrouter/vendor/moving",
            "provider/both",
        ]
    );

    let default = row(&snapshot, "anthropic", "core-default");
    assert_eq!(
        default["supported_pi_thinking_levels"],
        serde_json::json!(["off", "minimal", "low", "medium", "high"])
    );
    let holey = row(&snapshot, "anthropic", "core-holey");
    assert_eq!(
        holey["supported_pi_thinking_levels"],
        serde_json::json!(["off", "high", "xhigh"])
    );
    assert_eq!(holey["input_modes"], serde_json::json!(["text", "image"]));

    let list_only = row(&snapshot, "extension", "list-only");
    assert_eq!(list_only["display_name"], Value::Null);
    assert_eq!(list_only["reasoning"], Value::Null);
    assert_eq!(list_only["supported_pi_thinking_levels"], Value::Null);
    assert_eq!(list_only["context_window"], Value::Null);
    assert_eq!(list_only["maximum_output"], Value::Null);
    assert_eq!(list_only["input_modes"], Value::Null);
    assert_eq!(list_only["pricing_per_million_tokens"], Value::Null);
    assert_eq!(list_only["is_in_all_extension_list"], true);
    assert_eq!(list_only["is_in_core_rpc"], false);

    let rpc_only = row(&snapshot, "core", "rpc-only");
    assert_eq!(rpc_only["is_in_all_extension_list"], false);
    assert_eq!(rpc_only["is_in_core_rpc"], true);
    assert_eq!(rpc_only["is_exact_qualification_evidence"], false);

    let alias = row(&snapshot, "openrouter", "vendor/moving");
    assert_eq!(alias["is_moving_alias"], true);
    assert_eq!(alias["is_in_all_extension_list"], true);
    assert_eq!(alias["is_in_core_rpc"], true);
    assert_eq!(alias["is_exact_qualification_evidence"], false);

    let non_reasoning = row(&snapshot, "provider", "both");
    assert_eq!(
        non_reasoning["supported_pi_thinking_levels"],
        serde_json::json!(["off"])
    );
    assert_eq!(non_reasoning["context_window"], 100000);
    assert_eq!(non_reasoning["maximum_output"], 10000);
    assert_eq!(
        non_reasoning["pricing_per_million_tokens"],
        serde_json::json!({
            "input": 1.0,
            "output": 2.0,
            "cacheRead": 0.1,
            "cacheWrite": 0.2
        })
    );

    let text = fs::read_to_string(fixture.root.join("snapshots/catalog.json")).unwrap();
    assert!(!text.contains("baseUrl"));
    assert!(!text.contains("headers"));
    assert!(!text.contains("secret"));
}

#[test]
fn tiered_pricing_is_preserved_and_validated() {
    let fixture = Fixture::new();
    assert_success(&fixture.run("tiered.json", "tiered-valid"));
    let snapshot = fixture.snapshot("tiered.json");
    assert_eq!(
        row(&snapshot, "provider", "both")["pricing_per_million_tokens"],
        serde_json::json!({
            "input": 0.0,
            "output": 0.0,
            "cacheRead": 0.1,
            "cacheWrite": 0.2,
            "tiers": [
                {
                    "inputTokensAbove": 100000,
                    "input": 0.0,
                    "output": 0.25,
                    "cacheRead": 0.3,
                    "cacheWrite": 0.4
                },
                {
                    "inputTokensAbove": 200000,
                    "input": 0.5,
                    "output": 0.0,
                    "cacheRead": 0.6,
                    "cacheWrite": 0.7
                }
            ]
        })
    );
    let preview = fixture.preview("tiered.json");
    assert_success(&preview);
    let report = serde_json::from_slice::<Value>(&preview.stdout).unwrap();
    let reasons = report["excluded"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["provider"] == "provider" && entry["model"] == "both")
        .unwrap()["reasons"]
        .clone();
    assert_eq!(
        reasons,
        serde_json::json!(["nonzero_input_price", "nonzero_output_price"])
    );

    let fixture = Fixture::new();
    assert_success(&fixture.run("free.json", "tiered-free-cache"));
    let preview = fixture.preview("free.json");
    assert_success(&preview);
    let report = serde_json::from_slice::<Value>(&preview.stdout).unwrap();
    assert!(
        report["eligible"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| { entry["provider"] == "provider" && entry["model"] == "both" })
    );

    let fixture = Fixture::new();
    assert_success(&fixture.run("flat.json", "valid"));
    let snapshot = fixture.snapshot("flat.json");
    assert!(
        row(&snapshot, "provider", "both")["pricing_per_million_tokens"]
            .get("tiers")
            .is_none()
    );
    assert_success(&fixture.preview("flat.json"));

    let fixture = Fixture::new();
    assert_success(&fixture.run("auto.json", "auto-valid"));
    let snapshot = fixture.snapshot("auto.json");
    for model in ["openrouter/auto", "openrouter/auto-beta"] {
        assert_eq!(
            row(&snapshot, "openrouter", model)["pricing_per_million_tokens"],
            Value::Null
        );
    }
    let snapshot_text = fs::read_to_string(fixture.root.join("auto.json")).unwrap();
    assert!(!snapshot_text.contains("-1000000"));
    assert!(!snapshot_text.contains("\"input\": -"));
    assert!(!snapshot_text.contains("\"output\": -"));
    let preview = fixture.preview("auto.json");
    assert_success(&preview);
    let report = serde_json::from_slice::<Value>(&preview.stdout).unwrap();
    for model in ["openrouter/auto", "openrouter/auto-beta"] {
        let reasons = report["excluded"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["provider"] == "openrouter" && entry["model"] == model)
            .unwrap()["reasons"]
            .clone();
        assert_eq!(
            reasons,
            serde_json::json!(["moving_router_or_control", "missing_price"])
        );
    }

    for scenario in [
        "tier-zero-threshold",
        "tier-duplicate-threshold",
        "tier-descending-threshold",
        "tier-missing-field",
        "tier-unknown-field",
        "tier-negative-price",
        "tier-malformed-price",
        "tier-nonfinite-price",
        "base-negative-price",
        "auto-mixed-sign",
        "auto-negative-cache",
        "auto-tiered-sentinel",
        "non-control-negative",
        "base-malformed-price",
        "base-nonfinite-price",
    ] {
        let fixture = Fixture::new();
        let output = fixture.run("rejected.json", scenario);
        assert!(!output.status.success(), "scenario {scenario} succeeded");
        assert!(!fixture.root.join("rejected.json").exists());
    }
}

#[test]
fn probe_uses_only_three_metadata_commands_and_one_non_prompt_rpc_request() {
    let fixture = Fixture::new();
    let output = fixture.run("catalog.json", "valid");
    assert_success(&output);
    let log = fs::read_to_string(&fixture.log).unwrap();
    let lines = log.lines().collect::<Vec<_>>();

    assert_eq!(
        lines
            .iter()
            .filter(|line| **line == "args:--list-models")
            .count(),
        1
    );
    assert_eq!(
        lines
            .iter()
            .filter(|line| {
                **line == "args:--mode rpc --no-session --no-context-files --no-extensions"
            })
            .count(),
        1
    );
    assert_eq!(
        lines
            .iter()
            .filter(|line| **line == "args:--version")
            .count(),
        1
    );
    assert_eq!(lines.len(), 4);
    assert!(log.contains("get_available_models"));
    assert!(!log.contains("prompt"));
    assert!(!log.contains("message"));
    assert!(!log.contains("--model"));
}

#[test]
fn identical_inputs_have_identical_output_except_observation_time() {
    let fixture = Fixture::new();
    assert_success(&fixture.run("one.json", "valid"));
    assert_success(&fixture.run("two.json", "valid"));
    let mut one = fixture.snapshot("one.json");
    let mut two = fixture.snapshot("two.json");
    one["observed_at_unix_seconds"] = Value::Null;
    two["observed_at_unix_seconds"] = Value::Null;

    assert_eq!(one, two);
}

#[test]
fn duplicate_conflicting_and_malformed_inputs_fail_without_snapshot() {
    for scenario in [
        "duplicate-list",
        "conflicting-list",
        "malformed-list",
        "duplicate-rpc",
        "malformed-rpc",
        "failed-rpc",
        "duplicate-response",
    ] {
        let fixture = Fixture::new();
        let output = fixture.run("catalog.json", scenario);
        assert!(!output.status.success(), "scenario {scenario} succeeded");
        assert!(!fixture.root.join("catalog.json").exists());
    }
}

#[test]
fn existing_output_collision_is_rejected_before_any_probe() {
    let fixture = Fixture::new();
    fs::write(fixture.root.join("catalog.json"), "owner-data\n").unwrap();

    let output = fixture.run("catalog.json", "valid");

    assert!(!output.status.success());
    assert_eq!(
        fs::read_to_string(fixture.root.join("catalog.json")).unwrap(),
        "owner-data\n"
    );
    assert!(!fixture.log.exists());
}

#[test]
fn atomic_write_leaves_one_complete_snapshot_and_no_temporary_file() {
    let fixture = Fixture::new();
    let output = fixture.run("nested/catalog.json", "valid");
    assert_success(&output);
    let directory = fixture.root.join("nested");
    let entries = fs::read_dir(&directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();

    assert_eq!(entries, [OsString::from("catalog.json")]);
    serde_json::from_slice::<Value>(&fs::read(directory.join("catalog.json")).unwrap()).unwrap();
}

#[cfg(unix)]
#[test]
fn symlink_output_and_symlink_parent_are_rejected_before_probe() {
    use std::os::unix::fs::symlink;

    for is_parent in [false, true] {
        let fixture = Fixture::new();
        if is_parent {
            let outside = fixture.root.join("outside");
            fs::create_dir(&outside).unwrap();
            symlink(&outside, fixture.root.join("linked")).unwrap();
        } else {
            symlink(
                fixture.root.join("missing-target"),
                fixture.root.join("linked.json"),
            )
            .unwrap();
        }
        let path = if is_parent {
            "linked/catalog.json"
        } else {
            "linked.json"
        };

        let output = fixture.run(path, "valid");

        assert!(!output.status.success());
        assert!(!fixture.log.exists());
    }
}

fn row<'a>(snapshot: &'a Value, provider: &str, model: &str) -> &'a Value {
    snapshot["models"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["provider"] == provider && row["model"] == model)
        .unwrap()
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn make_executable(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).unwrap();
    }
}
