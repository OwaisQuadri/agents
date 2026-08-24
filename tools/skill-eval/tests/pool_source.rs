#[path = "../src/model.rs"]
mod model;
#[path = "../src/pool_source.rs"]
mod pool_source;
#[path = "../src/ports.rs"]
mod ports;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use model::{SkillEvalError, Tier};
use pool_source::FilePoolPlanSource;
use ports::PoolPlanSource;
use serde_json::Value;

static TEMPORARY_REPOSITORY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TemporaryRepository {
    root: PathBuf,
}

impl TemporaryRepository {
    fn with_plan(value: &Value) -> Self {
        let sequence = TEMPORARY_REPOSITORY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "skill-eval-pool-source-{}-{sequence}",
            std::process::id()
        ));
        let plans = root.join("plans");
        fs::create_dir_all(&plans).expect("temporary repository should be created");
        fs::write(
            plans.join("plan.json"),
            serde_json::to_vec_pretty(value).expect("plan should serialize"),
        )
        .expect("temporary plan should be written");
        Self { root }
    }

    fn source(&self) -> FilePoolPlanSource {
        FilePoolPlanSource::new(&self.root).expect("temporary repository should load")
    }
}

impl Drop for TemporaryRepository {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).expect("temporary repository should be removed");
    }
}

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pool-source/repository")
}

fn valid_value() -> Value {
    serde_json::from_str(include_str!(
        "fixtures/pool-source/repository/plans/valid.json"
    ))
    .expect("valid fixture should parse")
}

fn invalid_message(error: SkillEvalError) -> String {
    match error {
        SkillEvalError::InvalidConfiguration(message) => message,
        other => panic!("expected invalid configuration, got {other:?}"),
    }
}

fn load_modified(value: Value) -> Result<model::PoolPlan, SkillEvalError> {
    let repository = TemporaryRepository::with_plan(&value);
    repository
        .source()
        .load_pool_plan(Path::new("plans/plan.json"))
}

#[test]
fn valid_frozen_plan_loads_exact_three_model_pools_and_control() {
    let source = FilePoolPlanSource::new(&fixture_root()).expect("fixture root should load");
    let plan = source
        .load_pool_plan(Path::new("plans/valid.json"))
        .expect("valid frozen plan should load");

    assert_eq!(plan.entrants.len(), 5);
    for tier in [Tier::T1, Tier::T2, Tier::T3, Tier::T4, Tier::T5] {
        assert_eq!(plan.entrants[&tier].len(), 3);
        assert!(
            plan.entrants[&tier]
                .iter()
                .all(|entrant| entrant.model.tier == tier)
        );
    }
    assert_eq!(plan.control.tier, Tier::T1);
    assert_eq!(plan.control.provider, "openrouter");
    assert_eq!(plan.control.model, "openrouter/free");
    assert_eq!(plan.control.thinking, "low");
    assert_eq!(plan.policy.calibration_repeats_per_case, 1);
    assert_eq!(plan.policy.qualification_repeats_per_case, 3);
    assert_eq!(plan.policy.promotion_count, 2);
    assert_eq!(plan.policy.minimum_score, 8);
    assert_eq!(plan.policy.minimum_reliability_basis_points, 9500);
    assert_eq!(plan.policy.spending_limit_millionths_of_dollar, 10_000_000);
    assert!(plan.policy.is_provider_limit_enforced);

    let first_party = &plan.entrants[&Tier::T3][0].model;
    let proxy = &plan.entrants[&Tier::T3][1].model;
    assert_eq!(first_party.provider, "anthropic");
    assert_eq!(first_party.model, "claude-sonnet-4-5-20250929");
    assert_eq!(proxy.provider, "openrouter");
    assert_eq!(proxy.model, "anthropic/claude-sonnet-4-5-20250929");
    assert_eq!(first_party.thinking, "medium");
    assert_eq!(proxy.thinking, "medium");
}

#[test]
// TODO(AGNT-0032.T101): Prove exact freshness boundaries against the runtime clock.
fn old_well_formed_observations_are_left_for_runtime_freshness_checks() {
    let source = FilePoolPlanSource::new(&fixture_root()).expect("fixture root should load");
    let plan = source
        .load_pool_plan(Path::new("plans/valid.json"))
        .expect("loader should not impose a catalog maximum age");

    assert_eq!(
        plan.entrants[&Tier::T1][0].catalog_observed_at.0,
        "2000-12-01T10:00:00-0500"
    );
}

#[test]
fn moving_aliases_and_duplicate_identities_are_rejected() {
    let mut alias = valid_value();
    alias["entrants"]["T1"][0]["model"]["model"] = Value::from("vendor/model-latest");
    let message = invalid_message(load_modified(alias).expect_err("alias should fail"));
    assert!(message.contains("moving alias"));

    let mut duplicate = valid_value();
    duplicate["entrants"]["T2"][1]["model"] = duplicate["entrants"]["T2"][0]["model"].clone();
    let message = invalid_message(load_modified(duplicate).expect_err("duplicate should fail"));
    assert!(message.contains("duplicate model identity"));
}

#[test]
fn every_tier_and_exactly_three_entrants_are_required() {
    let mut missing_tier = valid_value();
    missing_tier["entrants"]
        .as_object_mut()
        .expect("entrants should be an object")
        .remove("T5");
    let message =
        invalid_message(load_modified(missing_tier).expect_err("missing tier should fail"));
    assert!(message.contains("missing field `T5`"));

    let mut two = valid_value();
    two["entrants"]["T1"]
        .as_array_mut()
        .expect("T1 should be an array")
        .pop();
    let message = invalid_message(load_modified(two).expect_err("two entrants should fail"));
    assert!(message.contains("exactly 3 entrants"));

    let mut four = valid_value();
    let extra = four["entrants"]["T1"][0].clone();
    four["entrants"]["T1"]
        .as_array_mut()
        .expect("T1 should be an array")
        .push(extra);
    let message = invalid_message(load_modified(four).expect_err("four entrants should fail"));
    assert!(message.contains("exactly 3 entrants"));
}

#[test]
fn observation_times_must_be_present_and_strictly_well_formed() {
    let mut missing = valid_value();
    missing["entrants"]["T1"][0]
        .as_object_mut()
        .expect("entrant should be an object")
        .remove("catalog_observed_at");
    let message = invalid_message(load_modified(missing).expect_err("missing time should fail"));
    assert!(message.contains("missing field `catalog_observed_at`"));

    for malformed in [
        "2026-02-29T12:00:00-0500",
        "2026-01-01 12:00:00-0500",
        "2026-01-01T12:00:00Z",
        "2026-01-01T12:00:00+1401",
    ] {
        let mut value = valid_value();
        value["entrants"]["T1"][0]["catalog_observed_at"] = Value::from(malformed);
        let message =
            invalid_message(load_modified(value).expect_err("malformed time should fail"));
        assert!(message.contains("catalog_observed_at"));
    }
}

#[test]
fn control_is_read_only_t1_and_never_ranked() {
    let mut writable = valid_value();
    writable["control"]["is_read_only"] = Value::from(false);
    let message =
        invalid_message(load_modified(writable).expect_err("writable control should fail"));
    assert!(message.contains("read-only"));

    let mut higher = valid_value();
    higher["control"]["maximum_tier"] = Value::from("T2");
    let message = invalid_message(load_modified(higher).expect_err("higher control should fail"));
    assert!(message.contains("maximum tier must be T1"));

    let mut ranked = valid_value();
    ranked["entrants"]["T1"][0]["model"] = ranked["control"]["model"].clone();
    let message = invalid_message(load_modified(ranked).expect_err("ranked control should fail"));
    assert!(message.contains("cannot be a ranked entrant"));
}

#[test]
fn invalid_policy_values_and_missing_provider_cap_are_rejected() {
    for (field, value, expected) in [
        ("calibration_repeats_per_case", 0_u64, "must be positive"),
        ("qualification_repeats_per_case", 0, "must be positive"),
        ("promotion_count", 3, "must be 2"),
        ("minimum_score", 11, "must not exceed 10"),
        (
            "minimum_reliability_basis_points",
            10_001,
            "must not exceed 10000",
        ),
        ("spending_limit_millionths_of_dollar", 0, "must be positive"),
    ] {
        let mut plan = valid_value();
        plan["policy"][field] = Value::from(value);
        let message = invalid_message(load_modified(plan).expect_err("invalid policy should fail"));
        assert!(message.contains(expected), "unexpected message: {message}");
    }

    let mut uncapped = valid_value();
    uncapped["policy"]["is_provider_limit_enforced"] = Value::from(false);
    let message =
        invalid_message(load_modified(uncapped).expect_err("uncapped paid plan should fail"));
    assert!(message.contains("provider-enforced spending limit"));
}

#[test]
fn paths_runtime_identity_unknown_fields_and_credentials_fail_closed() {
    let source = FilePoolPlanSource::new(&fixture_root()).expect("fixture root should load");
    let message = invalid_message(
        source
            .load_pool_plan(Path::new("../outside.json"))
            .expect_err("escaping path should fail"),
    );
    assert!(message.contains("invalid component"));

    for (field, value) in [
        ("run_id", "pool-plan-001"),
        ("created_at", "2001-01-02T03:04:05-0400"),
    ] {
        let mut runtime_identity = valid_value();
        runtime_identity
            .as_object_mut()
            .expect("plan should be an object")
            .insert(field.to_owned(), Value::from(value));
        let message = invalid_message(
            load_modified(runtime_identity).expect_err("runtime identity field should fail"),
        );
        assert!(
            message.contains(&format!("unknown field `{field}`")),
            "unexpected message: {message}"
        );
    }

    let mut unknown = valid_value();
    unknown
        .as_object_mut()
        .expect("plan should be an object")
        .insert("note".to_owned(), Value::from("unexpected"));
    let message = invalid_message(load_modified(unknown).expect_err("unknown field should fail"));
    assert!(message.contains("unknown field `note`"));

    let mut credential = valid_value();
    credential
        .as_object_mut()
        .expect("plan should be an object")
        .insert("api_key".to_owned(), Value::from("synthetic-not-a-secret"));
    let message = invalid_message(load_modified(credential).expect_err("credential should fail"));
    assert!(message.contains("unknown field `api_key`"));
}
