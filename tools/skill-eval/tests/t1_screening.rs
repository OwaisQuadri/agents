#[path = "../src/model.rs"]
mod model;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Value, json};

use model::{ModelIdentity, RunId, T1ScreenExclusionReason, T1ScreenModelOutcome, Tier};

static FIXTURE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(rows: Vec<Value>) -> Self {
        let root = std::env::temp_dir().join(format!(
            "skill-eval-t1-screening-{}-{}",
            std::process::id(),
            FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("config")).unwrap();
        fs::create_dir(root.join("skills")).unwrap();
        fs::write(root.join("config/model-tiers.json"), "{}\n").unwrap();
        let fixture = Self { root };
        fixture.write_snapshot(snapshot(rows));
        fixture
    }

    fn write_snapshot(&self, snapshot: Value) {
        fs::write(
            self.root.join("capabilities.json"),
            serde_json::to_vec_pretty(&snapshot).unwrap(),
        )
        .unwrap();
    }

    fn run(&self, arguments: &[&str]) -> Output {
        self.command().args(arguments).output().unwrap()
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_skill-eval"));
        command.current_dir(&self.root);
        command
    }

    fn preview(&self) -> Value {
        let output = self.run(&[
            "t1-screen-preview",
            "--capabilities",
            "capabilities.json",
            "--format",
            "json",
        ]);
        assert_success(&output);
        assert_eq!(
            output.stdout.iter().filter(|byte| **byte == b'\n').count(),
            1
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}

#[test]
fn every_row_gets_one_result_and_every_reason_is_stable() {
    let mut rows = Vec::new();
    rows.push(changed_row("alpha", "00-missing-list", |row| {
        row["is_in_all_extension_list"] = json!(false);
        row["is_exact_qualification_evidence"] = json!(false);
    }));
    rows.push(changed_row("alpha", "01-missing-rpc", |row| {
        row["is_in_core_rpc"] = json!(false);
        row["is_exact_qualification_evidence"] = json!(false);
    }));
    rows.push(changed_row("alpha", "02-moving-alias", |row| {
        row["is_moving_alias"] = json!(true);
        row["is_exact_qualification_evidence"] = json!(false);
    }));
    rows.push(changed_row("alpha", "03-not-exact", |row| {
        row["is_exact_qualification_evidence"] = json!(false);
    }));
    rows.push(changed_row("alpha", "04-missing-price", |row| {
        row["pricing_per_million_tokens"] = Value::Null;
    }));
    rows.push(changed_row("alpha", "05-paid-input", |row| {
        row["pricing_per_million_tokens"]["input"] = json!(0.01);
    }));
    rows.push(changed_row("alpha", "06-paid-output", |row| {
        row["pricing_per_million_tokens"]["output"] = json!(0.02);
    }));
    rows.push(changed_row("alpha", "07-no-text", |row| {
        row["input_modes"] = json!(["image"]);
    }));
    rows.push(changed_row("alpha", "08-no-levels", |row| {
        row["supported_pi_thinking_levels"] = Value::Null;
    }));
    rows.push(changed_row("alpha", "09-malformed-levels", |row| {
        row["supported_pi_thinking_levels"] = json!(["high", "low"]);
    }));
    for model in ["auto", "openrouter/free", "openrouter/fusion"] {
        rows.push(base_row("openrouter", model));
    }
    let fixture = Fixture::new(rows);
    let report = fixture.preview();

    assert_eq!(report["total_rows"], 13);
    assert_eq!(report["eligible_count"], 0);
    assert_eq!(report["excluded_count"], 13);
    let reasons = excluded_reasons(&report);
    assert_reason(&reasons, "alpha/00-missing-list", "missing_list");
    assert_reason(&reasons, "alpha/01-missing-rpc", "missing_rpc");
    assert_reason(&reasons, "alpha/02-moving-alias", "moving_alias");
    assert_reason(&reasons, "alpha/03-not-exact", "not_exact_evidence");
    assert_reason(&reasons, "alpha/04-missing-price", "missing_price");
    assert_reason(&reasons, "alpha/05-paid-input", "nonzero_input_price");
    assert_reason(&reasons, "alpha/06-paid-output", "nonzero_output_price");
    assert_reason(&reasons, "alpha/07-no-text", "missing_text_input");
    assert_reason(&reasons, "alpha/08-no-levels", "missing_thinking_levels");
    assert_reason(
        &reasons,
        "alpha/09-malformed-levels",
        "malformed_thinking_levels",
    );
    for identity in [
        "openrouter/auto",
        "openrouter/openrouter/free",
        "openrouter/openrouter/fusion",
    ] {
        assert_reason(&reasons, identity, "moving_router_or_control");
    }
}

#[test]
fn fixed_preview_tool_claims_and_thinking_holes_do_not_filter_candidates() {
    let preview = changed_row("alpha", "fixed-preview-free", |row| {
        row["display_name"] = json!("External claim says tools are unsupported");
        row["supported_pi_thinking_levels"] = json!(["off", "high", "xhigh"]);
    });
    let ordinary = changed_row("alpha", "ordinary", |row| {
        row["supported_pi_thinking_levels"] = json!(["minimal", "medium", "max"]);
    });
    let fixture = Fixture::new(vec![preview, ordinary]);
    let report = fixture.preview();

    assert_eq!(report["eligible_count"], 2);
    assert_eq!(report["excluded_count"], 0);
    assert_eq!(report["eligible"][0]["model"], "fixed-preview-free");
    assert_eq!(report["eligible"][0]["is_preview"], true);
    assert_eq!(
        report["eligible"][0]["supported_pi_thinking_levels"],
        json!(["off", "high", "xhigh"])
    );
    assert_eq!(
        report["eligible"][1]["supported_pi_thinking_levels"],
        json!(["minimal", "medium", "max"])
    );
}

#[test]
fn matrix_screens_every_supported_thinking_level() {
    let first = changed_row("alpha", "first", |row| {
        row["supported_pi_thinking_levels"] =
            json!(["off", "minimal", "low", "medium", "high", "xhigh", "max"]);
    });
    let second = changed_row("alpha", "second", |row| {
        row["supported_pi_thinking_levels"] = json!(["minimal", "medium", "max"]);
    });
    let fixture = Fixture::new(vec![first, second]);
    let report = fixture.preview();

    assert_eq!(report["eligible_count"], 2);
    assert_eq!(
        report["eligible"][0]["supported_pi_thinking_levels"],
        json!(["off", "minimal", "low", "medium", "high", "xhigh", "max"])
    );
    assert_eq!(
        report["eligible"][1]["supported_pi_thinking_levels"],
        json!(["minimal", "medium", "max"])
    );
    assert_eq!(
        report["candidate_calls"],
        json!({"minimum": 50, "maximum": 50})
    );
    assert_eq!(report["judge_calls"], report["candidate_calls"]);
}

#[test]
fn exact_complete_call_projection() {
    let first = changed_row("alpha", "first", |row| {
        row["supported_pi_thinking_levels"] = json!(["off", "high", "xhigh"]);
    });
    let second = changed_row("alpha", "second", |row| {
        row["supported_pi_thinking_levels"] = json!(["minimal", "low"]);
    });
    let fixture = Fixture::new(vec![first, second]);
    let report = fixture.preview();

    assert_eq!(report["exam_case_count"], 5);
    assert_eq!(
        report["candidate_calls"],
        json!({"minimum": 25, "maximum": 25})
    );
    assert_eq!(report["judge_calls"], report["candidate_calls"]);
    assert_eq!(report["projected_candidate_money_cost_usd"], 0);
    assert_eq!(
        report["is_judge_money_projected_from_candidate_price"],
        false
    );
    assert_eq!(
        report["is_owner_approved_judge_cap_required_before_execution"],
        true
    );
    assert!(
        report["judge_money_note"]
            .as_str()
            .unwrap()
            .contains("owner-approved cap before execution")
    );

    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new(env!("CARGO_BIN_EXE_skill-eval"))
        .current_dir(repository)
        .args([
            "t1-screen-preview",
            "--capabilities",
            "research/model-routing/pi-model-capabilities.json",
            "--format",
            "json",
        ])
        .output()
        .unwrap();
    assert_success(&output);
    let frozen: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        frozen["candidate_calls"],
        json!({"minimum": 495, "maximum": 495})
    );
    assert_eq!(frozen["judge_calls"], frozen["candidate_calls"]);
}

#[test]
fn generated_379_row_24_zero_cost_baseline_needs_no_second_snapshot() {
    let mut rows = (0..355)
        .map(|index| {
            changed_row("alpha", &format!("paid-{index:03}"), |row| {
                row["pricing_per_million_tokens"]["input"] = json!(1);
                row["pricing_per_million_tokens"]["output"] = json!(1);
            })
        })
        .collect::<Vec<_>>();
    rows.extend((0..21).map(|index| base_row("openrouter", &format!("fixed-{index:03}:free"))));
    rows.extend([
        base_row("openrouter", "auto"),
        base_row("openrouter", "openrouter/free"),
        base_row("openrouter", "openrouter/fusion"),
    ]);
    let zero_cost_count = rows
        .iter()
        .filter(|row| {
            row["pricing_per_million_tokens"]["input"] == 0
                && row["pricing_per_million_tokens"]["output"] == 0
        })
        .count();
    assert_eq!(zero_cost_count, 24);
    let fixture = Fixture::new(rows);
    let report = fixture.preview();

    assert_eq!(report["total_rows"], 379);
    assert_eq!(report["eligible_count"], 21);
    assert_eq!(report["excluded_count"], 358);
    assert_eq!(
        report["candidate_calls"],
        json!({"minimum": 105, "maximum": 105})
    );
}

#[test]
fn serialization_and_classification_order_are_deterministic() {
    let rows = vec![
        base_row("alpha", "eligible-a"),
        changed_row("alpha", "excluded-b", |row| {
            row["pricing_per_million_tokens"] = Value::Null;
        }),
        base_row("beta", "eligible-c"),
        changed_row("beta", "excluded-d", |row| {
            row["input_modes"] = json!(["image"]);
        }),
    ];
    let fixture = Fixture::new(rows);
    let first = fixture.run(&[
        "t1-screen-preview",
        "--capabilities",
        "capabilities.json",
        "--format",
        "json",
    ]);
    let second = fixture.run(&[
        "t1-screen-preview",
        "--capabilities",
        "capabilities.json",
        "--format",
        "json",
    ]);
    assert_success(&first);
    assert_success(&second);
    assert_eq!(first.stdout, second.stdout);
    let report: Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(
        identities(&report["eligible"]),
        ["alpha/eligible-a", "beta/eligible-c"]
    );
    assert_eq!(
        identities(&report["excluded"]),
        ["alpha/excluded-b", "beta/excluded-d"]
    );
    assert_eq!(report["snapshot"]["sha256"].as_str().unwrap().len(), 64);
}

#[test]
fn parser_accepts_only_one_safe_capability_path_and_text_or_json() {
    let fixture = Fixture::new(vec![base_row("alpha", "fixed")]);
    for arguments in [
        vec!["t1-screen-preview"],
        vec![
            "t1-screen-preview",
            "--capabilities",
            "../capabilities.json",
        ],
        vec![
            "t1-screen-preview",
            "--capabilities",
            "capabilities.json",
            "--format",
            "jsonl",
        ],
        vec![
            "t1-screen-preview",
            "--capabilities",
            "capabilities.json",
            "--capabilities",
            "capabilities.json",
        ],
        vec![
            "t1-screen-preview",
            "--capabilities",
            "capabilities.json",
            "--format",
            "text",
            "--format",
            "json",
        ],
        vec![
            "t1-screen-preview",
            "--capabilities",
            "capabilities.json",
            "--runs-root",
            "runs",
        ],
    ] {
        assert!(
            !fixture.run(&arguments).status.success(),
            "accepted {arguments:?}"
        );
    }
    for format in ["text", "json"] {
        assert_success(&fixture.run(&[
            "t1-screen-preview",
            "--capabilities",
            "capabilities.json",
            "--format",
            format,
        ]));
    }
}

#[test]
fn malformed_snapshots_fail_closed() {
    let fixture = Fixture::new(vec![base_row("alpha", "fixed")]);
    let valid = snapshot(vec![base_row("alpha", "fixed")]);
    let mut cases = Vec::new();

    let mut unknown_field = valid.clone();
    unknown_field["unknown"] = json!(true);
    cases.push(unknown_field);

    let mut unknown_row_field = valid.clone();
    unknown_row_field["models"][0]["tool_support"] = json!(false);
    cases.push(unknown_row_field);

    let mut count_mismatch = valid.clone();
    count_mismatch["counts"]["union"] = json!(2);
    cases.push(count_mismatch);

    let mut unsupported_version = valid.clone();
    unsupported_version["snapshot_version"] = json!(2);
    cases.push(unsupported_version);

    let mut malformed_price = valid.clone();
    malformed_price["models"][0]["pricing_per_million_tokens"]["input"] = json!(-1);
    cases.push(malformed_price);

    let mut malformed_level_type = valid.clone();
    malformed_level_type["models"][0]["supported_pi_thinking_levels"] = json!([1]);
    cases.push(malformed_level_type);

    let mut unsorted = snapshot(vec![
        base_row("alpha", "earlier"),
        base_row("beta", "later"),
    ]);
    unsorted["models"].as_array_mut().unwrap().swap(0, 1);
    cases.push(unsorted);
    cases.push(snapshot(vec![
        base_row("alpha", "same"),
        base_row("alpha", "same"),
    ]));

    for case in cases {
        fixture.write_snapshot(case);
        let output = fixture.run(&[
            "t1-screen-preview",
            "--capabilities",
            "capabilities.json",
            "--format",
            "json",
        ]);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn route_failure_outcome_is_distinct_from_scored_exhaustion() {
    let failed = T1ScreenModelOutcome::InfrastructureFailed {
        model: ModelIdentity {
            tier: Tier::T1,
            provider: "provider".to_owned(),
            model: "model".to_owned(),
            thinking: "high".to_owned(),
        },
        child_run_id: RunId("child-1".to_owned()),
    };
    let failed_json = serde_json::to_value(&failed).unwrap();
    let exhausted_json = serde_json::to_value(T1ScreenModelOutcome::Exhausted).unwrap();
    assert_eq!(failed_json["kind"], "infrastructure_failed");
    assert_eq!(failed_json["child_run_id"], "child-1");
    assert_eq!(exhausted_json["kind"], "exhausted");
    assert_ne!(failed_json, exhausted_json);
}

#[test]
fn unknown_exclusion_reason_input_is_rejected() {
    assert!(serde_json::from_str::<T1ScreenExclusionReason>("\"unknown_reason\"").is_err());
}

#[cfg(unix)]
#[test]
fn preview_makes_no_write_and_no_pi_call() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = Fixture::new(vec![base_row("alpha", "fixed")]);
    let bin = fixture.root.join("bin");
    fs::create_dir(&bin).unwrap();
    let pi = bin.join("pi");
    fs::write(
        &pi,
        "#!/bin/sh\nprintf called > \"$FAKE_PI_LOG\"\nexit 97\n",
    )
    .unwrap();
    let mut permissions = fs::metadata(&pi).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&pi, permissions).unwrap();
    let log = fixture.root.join("pi-called");
    let current_path = std::env::var_os("PATH").unwrap_or_default();
    let path = std::iter::once(bin).chain(std::env::split_paths(&current_path));
    let before = tree(&fixture.root);
    let output = fixture
        .command()
        .args([
            "t1-screen-preview",
            "--capabilities",
            "capabilities.json",
            "--format",
            "json",
        ])
        .env("PATH", std::env::join_paths(path).unwrap())
        .env("FAKE_PI_LOG", &log)
        .output()
        .unwrap();
    assert_success(&output);
    assert!(!log.exists());
    assert_eq!(tree(&fixture.root), before);
}

#[cfg(unix)]
#[test]
fn symlink_capability_paths_are_rejected() {
    use std::os::unix::fs::symlink;

    let fixture = Fixture::new(vec![base_row("alpha", "fixed")]);
    symlink(
        fixture.root.join("capabilities.json"),
        fixture.root.join("linked.json"),
    )
    .unwrap();
    let output = fixture.run(&[
        "t1-screen-preview",
        "--capabilities",
        "linked.json",
        "--format",
        "json",
    ]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
}

fn base_row(provider: &str, model: &str) -> Value {
    json!({
        "provider": provider,
        "model": model,
        "display_name": format!("{provider}/{model}"),
        "reasoning": true,
        "supported_pi_thinking_levels": ["off"],
        "context_window": 100000,
        "maximum_output": 10000,
        "input_modes": ["text"],
        "pricing_per_million_tokens": {
            "input": 0,
            "output": 0,
            "cacheRead": 0,
            "cacheWrite": 0
        },
        "is_in_all_extension_list": true,
        "is_in_core_rpc": true,
        "is_moving_alias": false,
        "is_exact_qualification_evidence": true
    })
}

fn changed_row(provider: &str, model: &str, change: impl FnOnce(&mut Value)) -> Value {
    let mut row = base_row(provider, model);
    change(&mut row);
    row
}

fn snapshot(mut rows: Vec<Value>) -> Value {
    rows.sort_by_key(|row| {
        format!(
            "{}/{}",
            row["provider"].as_str().unwrap(),
            row["model"].as_str().unwrap()
        )
    });
    let all_extension_list = rows
        .iter()
        .filter(|row| row["is_in_all_extension_list"] == true)
        .count();
    let core_rpc = rows
        .iter()
        .filter(|row| row["is_in_core_rpc"] == true)
        .count();
    let list_only = rows
        .iter()
        .filter(|row| row["is_in_all_extension_list"] == true && row["is_in_core_rpc"] == false)
        .count();
    let rpc_only = rows
        .iter()
        .filter(|row| row["is_in_all_extension_list"] == false && row["is_in_core_rpc"] == true)
        .count();
    let moving_aliases = rows
        .iter()
        .filter(|row| row["is_moving_alias"] == true)
        .count();
    json!({
        "snapshot_version": 1,
        "observed_at_unix_seconds": 1_787_707_885_u64,
        "pi_version": "fixture-pi-1",
        "probe_commands": [
            {
                "purpose": "all_extension_availability",
                "program": "pi",
                "arguments": ["--list-models"],
                "extension_mode": "normal_deployed_discovery",
                "request_id": null,
                "request_type": null
            },
            {
                "purpose": "core_model_metadata",
                "program": "pi",
                "arguments": [
                    "--mode",
                    "rpc",
                    "--no-session",
                    "--no-context-files",
                    "--no-extensions"
                ],
                "extension_mode": "disabled",
                "request_id": "skill-eval-models",
                "request_type": "get_available_models"
            },
            {
                "purpose": "pi_version",
                "program": "pi",
                "arguments": ["--version"],
                "extension_mode": null,
                "request_id": null,
                "request_type": null
            }
        ],
        "counts": {
            "union": rows.len(),
            "all_extension_list": all_extension_list,
            "core_rpc": core_rpc,
            "list_only": list_only,
            "rpc_only": rpc_only,
            "moving_aliases": moving_aliases
        },
        "models": rows
    })
}

fn excluded_reasons(report: &Value) -> BTreeMap<String, BTreeSet<String>> {
    report["excluded"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            let identity = format!(
                "{}/{}",
                row["provider"].as_str().unwrap(),
                row["model"].as_str().unwrap()
            );
            let reasons = row["reasons"]
                .as_array()
                .unwrap()
                .iter()
                .map(|reason| reason.as_str().unwrap().to_owned())
                .collect();
            (identity, reasons)
        })
        .collect()
}

fn assert_reason(reasons: &BTreeMap<String, BTreeSet<String>>, identity: &str, reason: &str) {
    assert!(reasons.get(identity).unwrap().contains(reason));
}

fn identities(rows: &Value) -> Vec<String> {
    rows.as_array()
        .unwrap()
        .iter()
        .map(|row| {
            format!(
                "{}/{}",
                row["provider"].as_str().unwrap(),
                row["model"].as_str().unwrap()
            )
        })
        .collect()
}

fn tree(root: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            paths.push(path.strip_prefix(root).unwrap().to_path_buf());
            if path.is_dir() {
                pending.push(path);
            }
        }
    }
    paths.sort();
    paths
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
