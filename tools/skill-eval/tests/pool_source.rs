#[path = "../src/model.rs"]
mod model;
#[path = "../src/pool_source.rs"]
mod pool_source;
#[path = "../src/ports.rs"]
mod ports;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use model::{PoolPlan, SkillEvalError, Tier, Timestamp};
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

fn load_modified(value: Value) -> Result<PoolPlan, SkillEvalError> {
    let repository = TemporaryRepository::with_plan(&value);
    repository
        .source()
        .load_pool_plan(Path::new("plans/plan.json"))
}

fn valid_plan() -> PoolPlan {
    FilePoolPlanSource::new(&fixture_root())
        .expect("fixture root should load")
        .load_pool_plan(Path::new("plans/valid.json"))
        .expect("valid frozen plan should load")
}

fn set_all_observations(plan: &mut PoolPlan, value: &str) {
    for entrants in plan.entrants.values_mut() {
        for entrant in entrants {
            entrant.catalog_observed_at = Timestamp(value.to_owned());
        }
    }
}

fn set_first_thinking(value: &mut Value, start: &str, levels: &[&str]) {
    value["entrants"]["T1"][0]["model"]["thinking"] = Value::from(start);
    value["entrants"]["T1"][0]["thinking_levels"] =
        Value::from(levels.iter().copied().map(Value::from).collect::<Vec<_>>());
}

#[test]
fn valid_frozen_plan_loads_three_model_pools_and_control() {
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
    assert_eq!(
        plan.policy.calibration_minimum_reliability_basis_points,
        8_000
    );
    assert_eq!(
        plan.policy.qualification_minimum_reliability_basis_points,
        10_000
    );
    assert_eq!(plan.policy.maximum_catalog_age_seconds, 7_200);
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
fn candidate_timeout_accepts_legacy_unbounded_and_positive_but_rejects_zero() {
    let legacy = valid_plan();
    assert!(
        legacy
            .entrants
            .values()
            .flatten()
            .all(|entrant| { entrant.candidate_timeout_seconds.is_none() })
    );

    let mut bounded = valid_value();
    bounded["entrants"]["T2"][0]["candidate_timeout_seconds"] = Value::from(17);
    let bounded = load_modified(bounded).expect("a positive candidate timeout should load");
    assert_eq!(
        bounded.entrants[&Tier::T2][0].candidate_timeout_seconds,
        Some(17)
    );

    let mut zero = valid_value();
    zero["entrants"]["T2"][0]["candidate_timeout_seconds"] = Value::from(0);
    let message = invalid_message(load_modified(zero).expect_err("zero timeout should fail"));
    assert!(message.contains("candidate_timeout_seconds must be positive"));
}

#[test]
fn thinking_levels_accept_all_canonical_values_and_preserve_fixed_models() {
    for start in ["low", "medium", "high"] {
        let mut value = valid_value();
        set_first_thinking(&mut value, start, &["low", "medium", "high"]);
        let plan = load_modified(value).expect("each list position should be a valid start");
        let entrant = &plan.entrants[&Tier::T1][0];
        assert_eq!(entrant.model.thinking, start);
        assert_eq!(
            entrant
                .thinking_levels
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["low", "medium", "high"]
        );
    }

    let standard_order = ["off", "minimal", "low", "medium", "high", "xhigh", "max"];
    for levels in standard_order.windows(3) {
        let mut value = valid_value();
        set_first_thinking(&mut value, levels[1], levels);
        let plan = load_modified(value).expect("adjacent thinking levels should load");
        let entrant = &plan.entrants[&Tier::T1][0];
        assert_eq!(entrant.model.thinking, levels[1]);
        assert_eq!(
            entrant.thinking_levels,
            levels.iter().map(ToString::to_string).collect::<Vec<_>>()
        );
    }

    let mut value = valid_value();
    set_first_thinking(&mut value, "medium", &standard_order);
    let plan = load_modified(value).expect("all seven canonical thinking levels should load");
    assert_eq!(
        plan.entrants[&Tier::T1][0].thinking_levels,
        standard_order.map(ToString::to_string)
    );

    let plan = valid_plan();
    let fixed = &plan.entrants[&Tier::T1][0];
    assert_eq!(fixed.model.thinking, "off");
    assert_eq!(
        fixed
            .thinking_levels
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["off"]
    );
}

#[test]
fn retained_lower_plan_field_fails_closed() {
    let absent = valid_plan();
    assert!(
        absent
            .entrants
            .values()
            .flatten()
            .all(|entrant| entrant.retained_lower_thinking_level.is_none())
    );

    let mut valid = valid_value();
    set_first_thinking(&mut valid, "medium", &["off", "medium", "high"]);
    valid["entrants"]["T1"][0]["retained_lower_thinking_level"] = Value::from("off");
    let plan = load_modified(valid).expect("a declared lower route should load");
    assert_eq!(
        plan.entrants[&Tier::T1][0]
            .retained_lower_thinking_level
            .as_deref(),
        Some("off")
    );

    for (label, retained, levels, expected) in [
        ("strongest only", "high", vec!["low", "high"], "below"),
        ("unsupported", "ultra", vec!["low", "high"], "exactly once"),
        ("moving", "latest", vec!["low", "high"], "exactly once"),
        ("foreign", "medium", vec!["low", "high"], "exactly once"),
    ] {
        let mut value = valid_value();
        set_first_thinking(&mut value, "low", &levels);
        value["entrants"]["T1"][0]["retained_lower_thinking_level"] = Value::from(retained);
        let message = invalid_message(load_modified(value).expect_err(label));
        assert!(message.contains(expected), "{label}: {message}");
    }

    let mut malformed = valid_value();
    malformed["entrants"]["T1"][0]["retained_lower_thinking_level"] = Value::from(1);
    assert!(invalid_message(load_modified(malformed).unwrap_err()).contains("malformed"));

    let mut duplicate = valid_value();
    set_first_thinking(&mut duplicate, "low", &["low", "low", "high"]);
    duplicate["entrants"]["T1"][0]["retained_lower_thinking_level"] = Value::from("low");
    assert!(invalid_message(load_modified(duplicate).unwrap_err()).contains("duplicate"));

    let mut unordered = valid_value();
    set_first_thinking(&mut unordered, "low", &["medium", "low", "high"]);
    unordered["entrants"]["T1"][0]["retained_lower_thinking_level"] = Value::from("low");
    assert!(invalid_message(load_modified(unordered).unwrap_err()).contains("ordered"));
}

#[test]
fn thinking_levels_survive_normalized_serialization_round_trip() {
    let plan = valid_plan();
    let encoded = serde_json::to_vec(&plan).expect("normalized pool plan should serialize");
    let decoded: PoolPlan =
        serde_json::from_slice(&encoded).expect("normalized pool plan should deserialize");

    assert_eq!(decoded, plan);
    for entrants in decoded.entrants.values() {
        for entrant in entrants {
            assert_eq!(
                entrant
                    .thinking_levels
                    .iter()
                    .filter(|level| level.as_str() == entrant.model.thinking.as_str())
                    .count(),
                1
            );
        }
    }
}

#[test]
fn thinking_levels_reject_every_invalid_shape() {
    for (label, start, levels, expected) in [
        ("empty", "off", Vec::new(), "one to 7"),
        ("duplicate", "low", vec!["low", "low"], "duplicate"),
        (
            "descending",
            "low",
            vec!["medium", "low"],
            "cheapest to strongest",
        ),
        ("unsupported", "low", vec!["low", "ultra"], "unsupported"),
        (
            "oversized",
            "low",
            vec![
                "off", "minimal", "low", "medium", "high", "xhigh", "max", "max",
            ],
            "one to 7",
        ),
        (
            "absent start",
            "medium",
            vec!["off", "minimal", "low"],
            "exactly once",
        ),
    ] {
        let mut value = valid_value();
        set_first_thinking(&mut value, start, &levels);
        let message =
            invalid_message(load_modified(value).expect_err("invalid thinking levels should fail"));
        assert!(message.contains(expected), "{label}: {message}");
    }
}

#[test]
fn thinking_levels_reject_missing_unknown_and_malformed_model_data() {
    let mut missing = valid_value();
    missing["entrants"]["T1"][0]
        .as_object_mut()
        .expect("entrant should be an object")
        .remove("thinking_levels");
    let message = invalid_message(load_modified(missing).expect_err("missing list should fail"));
    assert!(message.contains("missing field `thinking_levels`"));

    let mut unknown = valid_value();
    unknown["entrants"]["T1"][0]
        .as_object_mut()
        .expect("entrant should be an object")
        .insert("thinking_limit".to_owned(), Value::from(3));
    let message = invalid_message(load_modified(unknown).expect_err("unknown field should fail"));
    assert!(message.contains("unknown field `thinking_limit`"));

    let mut non_string = valid_value();
    non_string["entrants"]["T1"][0]["thinking_levels"] = Value::from(vec![1]);
    let message = invalid_message(
        load_modified(non_string).expect_err("non-string thinking level should fail"),
    );
    assert!(message.contains("malformed"));

    for (field, malformed) in [("provider", ""), ("model", "vendor//model")] {
        let mut value = valid_value();
        value["entrants"]["T1"][0]["model"][field] = Value::from(malformed);
        let message =
            invalid_message(load_modified(value).expect_err("malformed model should fail"));
        assert!(
            message.contains("malformed model identity"),
            "{field}: {message}"
        );
    }

    let mut unsupported_start = valid_value();
    unsupported_start["entrants"]["T1"][0]["model"]["thinking"] = Value::from("ultra");
    let message = invalid_message(
        load_modified(unsupported_start).expect_err("unsupported start should fail"),
    );
    assert!(message.contains("invalid thinking value"));
}

#[test]
fn old_well_formed_observations_are_left_for_runtime_freshness_checks() {
    let plan = valid_plan();

    assert_eq!(
        plan.entrants[&Tier::T1][0].catalog_observed_at.0,
        "2000-12-01T10:00:00-0500"
    );
}

#[test]
fn freshness_accepts_now_and_exact_maximum_age_without_changing_the_plan() {
    let source = FilePoolPlanSource::new(&fixture_root()).expect("fixture root should load");
    let now = Timestamp("2024-03-01T09:30:00+0930".to_owned());

    let mut current = valid_plan();
    set_all_observations(&mut current, "2024-02-29T19:00:00-0500");
    let current_before = current.clone();
    source
        .validate_pool_plan_freshness(&current, &now)
        .expect("equivalent offset times should be current");
    assert_eq!(current, current_before);

    let mut boundary = valid_plan();
    set_all_observations(&mut boundary, "2024-02-29T17:00:00-0500");
    let boundary_before = boundary.clone();
    source
        .validate_pool_plan_freshness(&boundary, &now)
        .expect("exact maximum age should be fresh");
    assert_eq!(boundary, boundary_before);
}

#[test]
fn freshness_rejects_future_and_stale_observations_from_any_entrant() {
    let source = FilePoolPlanSource::new(&fixture_root()).expect("fixture root should load");
    let now = Timestamp("2024-03-01T00:00:00+0000".to_owned());

    let mut future = valid_plan();
    set_all_observations(&mut future, "2024-03-01T00:00:00+0000");
    future.entrants.get_mut(&Tier::T5).unwrap()[2].catalog_observed_at =
        Timestamp("2024-03-01T00:00:01+0000".to_owned());
    let future_before = future.clone();
    let message = invalid_message(
        source
            .validate_pool_plan_freshness(&future, &now)
            .expect_err("future observation should fail"),
    );
    assert!(message.contains("future"));
    assert_eq!(future, future_before);

    let mut stale = valid_plan();
    set_all_observations(&mut stale, "2024-02-29T22:00:00+0000");
    stale.entrants.get_mut(&Tier::T5).unwrap()[2].catalog_observed_at =
        Timestamp("2024-02-29T21:59:59+0000".to_owned());
    let stale_before = stale.clone();
    let message = invalid_message(
        source
            .validate_pool_plan_freshness(&stale, &now)
            .expect_err("over-age observation should fail"),
    );
    assert!(message.contains("maximum_catalog_age_seconds"));
    assert_eq!(stale, stale_before);
}

#[test]
fn freshness_rejects_malformed_times_zero_age_and_timestamp_overflow() {
    let source = FilePoolPlanSource::new(&fixture_root()).expect("fixture root should load");
    let mut plan = valid_plan();
    set_all_observations(&mut plan, "2024-03-01T00:00:00+0000");

    let message = invalid_message(
        source
            .validate_pool_plan_freshness(&plan, &Timestamp("not-a-time".to_owned()))
            .expect_err("malformed runtime timestamp should fail"),
    );
    assert!(message.contains("runtime timestamp"));

    let mut zero_age = plan.clone();
    zero_age.policy.maximum_catalog_age_seconds = 0;
    let zero_age_before = zero_age.clone();
    let message = invalid_message(
        source
            .validate_pool_plan_freshness(
                &zero_age,
                &Timestamp("2024-03-01T00:00:00+0000".to_owned()),
            )
            .expect_err("zero maximum age should fail"),
    );
    assert!(message.contains("maximum_catalog_age_seconds must be positive"));
    assert_eq!(zero_age, zero_age_before);

    let mut invalid_plan_time = plan.clone();
    invalid_plan_time.entrants.get_mut(&Tier::T4).unwrap()[1].catalog_observed_at =
        Timestamp("2023-02-29T00:00:00+0000".to_owned());
    let invalid_plan_before = invalid_plan_time.clone();
    let message = invalid_message(
        source
            .validate_pool_plan_freshness(
                &invalid_plan_time,
                &Timestamp("2024-03-01T00:00:00+0000".to_owned()),
            )
            .expect_err("invalid plan timestamp should fail"),
    );
    assert!(message.contains("invalid date"));
    assert_eq!(invalid_plan_time, invalid_plan_before);

    let message = invalid_message(
        source
            .validate_pool_plan_freshness(&plan, &Timestamp("0001-01-01T00:00:00+0001".to_owned()))
            .expect_err("runtime timestamp underflow should fail"),
    );
    assert!(message.contains("timestamp arithmetic overflow"));

    let mut overflowing_plan_time = plan.clone();
    overflowing_plan_time.entrants.get_mut(&Tier::T3).unwrap()[1].catalog_observed_at =
        Timestamp("9999-12-31T23:59:59-0001".to_owned());
    let overflowing_plan_before = overflowing_plan_time.clone();
    let message = invalid_message(
        source
            .validate_pool_plan_freshness(
                &overflowing_plan_time,
                &Timestamp("2024-03-01T00:00:00+0000".to_owned()),
            )
            .expect_err("plan timestamp overflow should fail"),
    );
    assert!(message.contains("timestamp arithmetic overflow"));
    assert_eq!(overflowing_plan_time, overflowing_plan_before);
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
fn accepts_four_entrants() {
    let mut value = valid_value();
    let extra = value["entrants"]["T1"][0].clone();
    value["entrants"]["T1"]
        .as_array_mut()
        .expect("T1 should be an array")
        .push(extra);
    value["entrants"]["T1"][3]["model"]["model"] = Value::from("fixture-t1-fourth");

    let plan = load_modified(value).expect("four unique entrants should load");

    assert_eq!(plan.entrants[&Tier::T1].len(), 4);
    assert_eq!(plan.entrants[&Tier::T1][3].model.model, "fixture-t1-fourth");
}

#[test]
fn every_tier_and_at_least_three_entrants_are_required() {
    let mut missing_tier = valid_value();
    missing_tier["entrants"]
        .as_object_mut()
        .expect("entrants should be an object")
        .remove("T5");
    let message =
        invalid_message(load_modified(missing_tier).expect_err("missing tier should fail"));
    assert!(message.contains("missing field `T5`"));

    for entrant_count in 0..3 {
        let mut value = valid_value();
        value["entrants"]["T1"]
            .as_array_mut()
            .expect("T1 should be an array")
            .truncate(entrant_count);
        let message = invalid_message(
            load_modified(value).expect_err("fewer than three entrants should fail"),
        );
        assert!(message.contains("at least 3 entrants"));
    }
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
            "calibration_minimum_reliability_basis_points",
            7_999,
            "must be 8000",
        ),
        (
            "qualification_minimum_reliability_basis_points",
            9_999,
            "must be 10000",
        ),
        ("maximum_catalog_age_seconds", 0, "must be positive"),
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
