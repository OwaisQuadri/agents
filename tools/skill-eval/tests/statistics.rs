#![expect(
    dead_code,
    reason = "the test imports private production modules to exercise crate-private statistics"
)]

// TODO(AGNT-0032.T90): Prove full-stage evidence under qualification repeat counts.
#[path = "../src/model.rs"]
mod model;
#[path = "../src/statistics.rs"]
mod statistics;

use std::path::PathBuf;

use model::{
    ArtifactName, CaseId, CheckResult, CheckStatus, ConfidenceInterval, HarnessIdentity,
    ModelIdentity, PoolEntrantEvidence, PoolPolicy, PoolStage, SkillEvalError, Tier, TrialKey,
    TrialRecord, TrialUsage, TrialVerdict,
};
use statistics::{evaluate_calibration, rank_pool};

#[test]
fn calibration_floor_accepts_exact_score_and_reliability_boundaries() {
    let mut trials = trials();
    trials[0].verdict.checks.push(failed_check());

    let evidence = evaluate_calibration(&model(), &expected_cases(), &trials, &policy()).unwrap();

    assert_eq!(evidence.stage, PoolStage::Calibration);
    assert_eq!(evidence.requested_model, model());
    assert_eq!(evidence.effective_model, model());
    assert!(evidence.is_passing);
    assert_eq!(evidence.completed_trials, 4);
    assert_eq!(evidence.expected_trials, 4);
    assert_eq!(evidence.failed_trials, 1);
    assert_eq!(evidence.catastrophic_trials, 0);
    assert_eq!(evidence.score.lower, 0.7);
    assert_eq!(evidence.score.estimate, 0.7);
    assert_eq!(evidence.score.upper, 0.7);
    assert_eq!(evidence.total_usage, usage(8));
}

#[test]
fn calibration_floor_returns_evidence_for_quality_and_reliability_failures() {
    let mut low_quality_policy = policy();
    low_quality_policy.minimum_reliability_basis_points = 0;
    let mut low_quality = trials();
    for trial in &mut low_quality {
        trial.verdict.score = 6;
    }
    let quality = evaluate_calibration(
        &model(),
        &expected_cases(),
        &low_quality,
        &low_quality_policy,
    )
    .unwrap();
    assert!(!quality.is_passing);
    assert_eq!(quality.failed_trials, 4);

    let mut unreliable = trials();
    unreliable[0].verdict.score = 10;
    unreliable[0].verdict.checks.push(failed_check());
    let mut strict_policy = policy();
    strict_policy.minimum_reliability_basis_points = 7_501;
    let reliability =
        evaluate_calibration(&model(), &expected_cases(), &unreliable, &strict_policy).unwrap();
    assert!(!reliability.is_passing);
    assert_eq!(reliability.failed_trials, 1);
    assert_eq!(reliability.score.estimate, 0.775);
}

#[test]
fn calibration_floor_rejects_invalid_expected_cases_and_case_coverage() {
    assert_invalid(evaluate_calibration(&model(), &[], &trials(), &policy()));
    assert_invalid(evaluate_calibration(
        &model(),
        &[CaseId("a".to_string()), CaseId("a".to_string())],
        &trials(),
        &policy(),
    ));

    let wholly_missing = vec![trial("a", 1), trial("a", 2)];
    assert_invalid(evaluate_calibration(
        &model(),
        &expected_cases(),
        &wholly_missing,
        &policy(),
    ));

    let mut unknown = trials();
    unknown[2].key.case = CaseId("unknown".to_string());
    assert_invalid(evaluate_calibration(
        &model(),
        &expected_cases(),
        &unknown,
        &policy(),
    ));

    let mut incomplete = trials();
    incomplete.pop();
    assert_invalid(evaluate_calibration(
        &model(),
        &expected_cases(),
        &incomplete,
        &policy(),
    ));

    let mut duplicate = trials();
    duplicate[1].key = duplicate[0].key.clone();
    assert_invalid(evaluate_calibration(
        &model(),
        &expected_cases(),
        &duplicate,
        &policy(),
    ));

    let mut invalid_attempt = trials();
    invalid_attempt[1].key.attempt = 3;
    assert_invalid(evaluate_calibration(
        &model(),
        &expected_cases(),
        &invalid_attempt,
        &policy(),
    ));
}

#[test]
fn calibration_floor_rejects_mixed_exam_harness_and_identity() {
    let changes: [fn(&mut TrialRecord); 6] = [
        |trial| trial.key.artifact = ArtifactName("other".to_string()),
        |trial| trial.harness.artifact_revision = "other".to_string(),
        |trial| trial.harness.runner_version = "other".to_string(),
        |trial| trial.harness.pi_version = "other".to_string(),
        |trial| trial.model.model = "other".to_string(),
        |trial| trial.key.tier = Tier::T3,
    ];
    for change in changes {
        let mut mixed = trials();
        change(&mut mixed[1]);
        assert_invalid(evaluate_calibration(
            &model(),
            &expected_cases(),
            &mixed,
            &policy(),
        ));
    }

    let mut case_harness_drift = trials();
    case_harness_drift[1].harness.tool_policy_digest = "other".to_string();
    assert_invalid(evaluate_calibration(
        &model(),
        &expected_cases(),
        &case_harness_drift,
        &policy(),
    ));

    let mut requested = model();
    requested.thinking = "high".to_string();
    assert_invalid(evaluate_calibration(
        &requested,
        &expected_cases(),
        &trials(),
        &policy(),
    ));
}

#[test]
fn calibration_floor_requires_one_external_judge_identity() {
    let mut judge_drift = trials();
    judge_drift[1].judge_model.model = "other-judge".to_string();
    assert_invalid(evaluate_calibration(
        &model(),
        &expected_cases(),
        &judge_drift,
        &policy(),
    ));

    let mut self_judged = trials();
    self_judged[0].judge_model = model();
    self_judged[0].judge_model.tier = Tier::T5;
    self_judged[0].judge_model.thinking = "high".to_string();
    assert_invalid(evaluate_calibration(
        &model(),
        &expected_cases(),
        &self_judged,
        &policy(),
    ));
}

#[test]
fn calibration_floor_rejects_malformed_scores_and_policy() {
    let mut malformed_score = trials();
    malformed_score[0].verdict.score = 11;
    assert_invalid(evaluate_calibration(
        &model(),
        &expected_cases(),
        &malformed_score,
        &policy(),
    ));

    let policies = [
        policy_with(|policy| policy.calibration_repeats_per_case = 0),
        policy_with(|policy| policy.qualification_repeats_per_case = 0),
        policy_with(|policy| policy.promotion_count = 1),
        policy_with(|policy| policy.minimum_score = 11),
        policy_with(|policy| policy.minimum_reliability_basis_points = 10_001),
        policy_with(|policy| policy.spending_limit_millionths_of_dollar = 0),
        policy_with(|policy| policy.is_provider_limit_enforced = false),
    ];
    for malformed_policy in policies {
        assert_invalid(evaluate_calibration(
            &model(),
            &expected_cases(),
            &trials(),
            &malformed_policy,
        ));
    }
}

#[test]
fn calibration_floor_returns_failing_evidence_for_a_catastrophic_trial() {
    let mut catastrophic = trials();
    catastrophic[0].verdict.score = 10;
    catastrophic[0].verdict.is_catastrophic = true;

    let evidence =
        evaluate_calibration(&model(), &expected_cases(), &catastrophic, &policy()).unwrap();

    assert!(!evidence.is_passing);
    assert_eq!(evidence.failed_trials, 0);
    assert_eq!(evidence.catastrophic_trials, 1);
}

#[test]
fn calibration_floor_preserves_per_case_harnesses_with_varied_tool_policies() {
    let mut varied = trials();
    varied[2].harness.tool_policy_digest = "case-b".to_string();
    varied[3].harness.tool_policy_digest = "case-b".to_string();
    let expected_harnesses = vec![varied[0].harness.clone(), varied[2].harness.clone()];

    let evidence = evaluate_calibration(&model(), &expected_cases(), &varied, &policy()).unwrap();

    assert!(evidence.is_passing);
    assert_eq!(evidence.judge_model, varied[0].judge_model);
    assert_eq!(evidence.harnesses, expected_harnesses);
}

#[test]
fn calibration_floor_aggregates_candidate_judge_and_total_usage_separately() {
    let mut split = trials();
    for trial in &mut split {
        trial.candidate_usage = usage(2);
        trial.judge_usage = usage(3);
    }

    let evidence = evaluate_calibration(&model(), &expected_cases(), &split, &policy()).unwrap();

    assert_eq!(evidence.candidate_usage, usage(8));
    assert_eq!(evidence.judge_usage, usage(12));
    assert_eq!(evidence.total_usage, usage(20));
}

#[test]
fn calibration_floor_rejects_usage_arithmetic_overflow() {
    let mut candidate = trials();
    candidate[0].candidate_usage.input_tokens = u64::MAX;
    assert_invalid(evaluate_calibration(
        &model(),
        &expected_cases(),
        &candidate,
        &policy(),
    ));

    let mut judge = trials();
    judge[0].judge_usage.input_tokens = u64::MAX;
    assert_invalid(evaluate_calibration(
        &model(),
        &expected_cases(),
        &judge,
        &policy(),
    ));

    let mut total = trials();
    for trial in &mut total {
        trial.candidate_usage = usage(0);
        trial.judge_usage = usage(0);
    }
    total[0].candidate_usage.input_tokens = u64::MAX;
    total[0].judge_usage.input_tokens = 1;
    assert_invalid(evaluate_calibration(
        &model(),
        &expected_cases(),
        &total,
        &policy(),
    ));
}

#[test]
fn ranked_pool_uses_candidate_task_cost_before_other_metrics() {
    let calibration = promoted_pair_calibration();
    let qualification = vec![
        pool_evidence(PoolStage::Qualification, "alpha", true, 6, 60, 1, 0),
        pool_evidence(PoolStage::Qualification, "beta", true, 12, 6, 0, 0),
    ];

    let pool = rank_pool(Tier::T2, &calibration, &qualification, &policy()).unwrap();

    assert_eq!(pool.ranked, vec![pool_model("alpha"), pool_model("beta")]);
}

#[test]
fn ranked_pool_uses_candidate_latency_after_task_cost() {
    let calibration = promoted_pair_calibration();
    let qualification = vec![
        pool_evidence(PoolStage::Qualification, "alpha", true, 6, 6, 1, 0),
        pool_evidence(PoolStage::Qualification, "beta", true, 6, 12, 0, 0),
    ];

    let pool = rank_pool(Tier::T2, &calibration, &qualification, &policy()).unwrap();

    assert_eq!(pool.ranked, vec![pool_model("alpha"), pool_model("beta")]);
}

#[test]
fn ranked_pool_uses_failure_rate_after_cost_and_latency() {
    let calibration = promoted_pair_calibration();
    let qualification = vec![
        pool_evidence(PoolStage::Qualification, "beta", true, 6, 6, 1, 0),
        pool_evidence(PoolStage::Qualification, "alpha", true, 6, 6, 0, 0),
    ];

    let pool = rank_pool(Tier::T2, &calibration, &qualification, &policy()).unwrap();

    assert_eq!(pool.ranked, vec![pool_model("alpha"), pool_model("beta")]);
}

#[test]
fn ranked_pool_uses_exact_identity_as_the_final_tie_break() {
    let calibration = promoted_pair_calibration();
    let qualification = vec![
        pool_evidence(PoolStage::Qualification, "beta", true, 6, 6, 0, 0),
        pool_evidence(PoolStage::Qualification, "alpha", true, 6, 6, 0, 0),
    ];

    let pool = rank_pool(Tier::T2, &calibration, &qualification, &policy()).unwrap();

    assert_eq!(pool.ranked, vec![pool_model("alpha"), pool_model("beta")]);
}

#[test]
fn ranked_pool_ignores_judge_overhead() {
    let calibration = promoted_pair_calibration();
    let qualification = vec![
        pool_evidence(PoolStage::Qualification, "alpha", true, 6, 6, 0, 1_000),
        pool_evidence(PoolStage::Qualification, "beta", true, 12, 12, 0, 0),
    ];

    let pool = rank_pool(Tier::T2, &calibration, &qualification, &policy()).unwrap();

    assert_eq!(pool.ranked, vec![pool_model("alpha"), pool_model("beta")]);
}

#[test]
fn ranked_pool_promotes_the_best_two_passing_calibration_entrants() {
    let calibration = vec![
        pool_evidence(PoolStage::Calibration, "expensive", true, 40, 4, 0, 0),
        pool_evidence(PoolStage::Calibration, "slower", true, 20, 8, 0, 0),
        pool_evidence(PoolStage::Calibration, "best", true, 20, 4, 1, 0),
    ];

    let pool = rank_pool(Tier::T2, &calibration, &[], &policy()).unwrap();

    assert_eq!(
        pool.promoted,
        vec![pool_model("best"), pool_model("slower")]
    );
    assert!(!pool.is_complete);
    assert!(pool.ranked.is_empty());
}

#[test]
fn ranked_pool_is_incomplete_when_fewer_than_two_calibration_entrants_pass() {
    let calibration = vec![
        pool_evidence(PoolStage::Calibration, "alpha", true, 4, 4, 0, 0),
        pool_evidence(PoolStage::Calibration, "beta", false, 8, 8, 2, 0),
        pool_evidence(PoolStage::Calibration, "gamma", false, 12, 12, 2, 0),
    ];

    let pool = rank_pool(Tier::T2, &calibration, &[], &policy()).unwrap();

    assert_eq!(pool.promoted, vec![pool_model("alpha")]);
    assert!(!pool.is_complete);
    assert!(pool.ranked.is_empty());
}

#[test]
fn ranked_pool_is_incomplete_for_a_missing_or_failed_finalist() {
    let calibration = promoted_pair_calibration();
    let alpha = pool_evidence(PoolStage::Qualification, "alpha", true, 6, 6, 0, 0);

    let missing = rank_pool(
        Tier::T2,
        &calibration,
        std::slice::from_ref(&alpha),
        &policy(),
    )
    .unwrap();
    assert!(!missing.is_complete);
    assert!(missing.ranked.is_empty());

    let failed = rank_pool(
        Tier::T2,
        &calibration,
        &[
            alpha,
            pool_evidence(PoolStage::Qualification, "beta", false, 6, 6, 2, 0),
        ],
        &policy(),
    )
    .unwrap();
    assert!(!failed.is_complete);
    assert!(failed.ranked.is_empty());
}

#[test]
fn ranked_pool_never_backfills_third_place_after_qualification() {
    let calibration = vec![
        pool_evidence(PoolStage::Calibration, "alpha", true, 4, 4, 0, 0),
        pool_evidence(PoolStage::Calibration, "beta", true, 8, 8, 0, 0),
        pool_evidence(PoolStage::Calibration, "gamma", true, 12, 12, 0, 0),
    ];
    let qualification = vec![
        pool_evidence(PoolStage::Qualification, "alpha", true, 6, 6, 0, 0),
        pool_evidence(PoolStage::Qualification, "beta", false, 6, 6, 2, 0),
    ];

    let pool = rank_pool(Tier::T2, &calibration, &qualification, &policy()).unwrap();

    assert_eq!(pool.promoted, vec![pool_model("alpha"), pool_model("beta")]);
    assert!(!pool.promoted.contains(&pool_model("gamma")));
    assert!(!pool.is_complete);
    assert!(pool.ranked.is_empty());
}

#[test]
fn ranked_pool_rejects_malformed_evidence() {
    let calibration = promoted_pair_calibration();
    let qualification = promoted_pair_qualification();

    let mut wrong_stage = calibration.clone();
    wrong_stage[0].stage = PoolStage::Qualification;
    assert_invalid_rank(rank_pool(Tier::T2, &wrong_stage, &qualification, &policy()));

    let mut wrong_tier = calibration.clone();
    wrong_tier[0].requested_model.tier = Tier::T3;
    wrong_tier[0].effective_model.tier = Tier::T3;
    assert_invalid_rank(rank_pool(Tier::T2, &wrong_tier, &qualification, &policy()));

    let mut duplicate = calibration.clone();
    duplicate[2] = duplicate[0].clone();
    assert_invalid_rank(rank_pool(Tier::T2, &duplicate, &qualification, &policy()));

    let unknown = vec![
        qualification[0].clone(),
        pool_evidence(PoolStage::Qualification, "gamma", true, 6, 6, 0, 0),
    ];
    assert_invalid_rank(rank_pool(Tier::T2, &calibration, &unknown, &policy()));

    let duplicate_finalist = vec![qualification[0].clone(), qualification[0].clone()];
    assert_invalid_rank(rank_pool(
        Tier::T2,
        &calibration,
        &duplicate_finalist,
        &policy(),
    ));

    let too_many = vec![
        qualification[0].clone(),
        qualification[1].clone(),
        pool_evidence(PoolStage::Qualification, "gamma", true, 6, 6, 0, 0),
    ];
    assert_invalid_rank(rank_pool(Tier::T2, &calibration, &too_many, &policy()));

    let mut impossible_counts = calibration.clone();
    impossible_counts[0].failed_trials = impossible_counts[0].completed_trials + 1;
    assert_invalid_rank(rank_pool(
        Tier::T2,
        &impossible_counts,
        &qualification,
        &policy(),
    ));

    let mut zero_denominator = calibration.clone();
    zero_denominator[0].completed_trials = 0;
    zero_denominator[0].expected_trials = 0;
    assert_invalid_rank(rank_pool(
        Tier::T2,
        &zero_denominator,
        &qualification,
        &policy(),
    ));

    let mut mixed_identity = calibration.clone();
    mixed_identity[0].effective_model.thinking = "high".to_string();
    assert_invalid_rank(rank_pool(
        Tier::T2,
        &mixed_identity,
        &qualification,
        &policy(),
    ));

    let mut self_judged = calibration.clone();
    self_judged[0].judge_model.provider = self_judged[0].effective_model.provider.clone();
    self_judged[0].judge_model.model = self_judged[0].effective_model.model.clone();
    assert_invalid_rank(rank_pool(Tier::T2, &self_judged, &qualification, &policy()));

    let mut inconsistent_usage = calibration.clone();
    inconsistent_usage[0].total_usage.cost_millionths_of_dollar += 1;
    assert_invalid_rank(rank_pool(
        Tier::T2,
        &inconsistent_usage,
        &qualification,
        &policy(),
    ));

    let mut invalid_metrics = calibration.clone();
    invalid_metrics[0].score.estimate = f64::NAN;
    assert_invalid_rank(rank_pool(
        Tier::T2,
        &invalid_metrics,
        &qualification,
        &policy(),
    ));

    let mut mixed_harnesses = calibration.clone();
    mixed_harnesses[0].harnesses[0].artifact_revision = "other".to_string();
    assert_invalid_rank(rank_pool(
        Tier::T2,
        &mixed_harnesses,
        &qualification,
        &policy(),
    ));

    let mut overflow = calibration.clone();
    overflow[0].candidate_usage.input_tokens = u64::MAX;
    overflow[0].judge_usage.input_tokens = 1;
    assert_invalid_rank(rank_pool(Tier::T2, &overflow, &qualification, &policy()));

    for malformed_policy in [
        policy_with(|policy| policy.calibration_repeats_per_case = 0),
        policy_with(|policy| policy.qualification_repeats_per_case = 0),
        policy_with(|policy| policy.promotion_count = 1),
        policy_with(|policy| policy.minimum_score = 11),
        policy_with(|policy| policy.minimum_reliability_basis_points = 10_001),
        policy_with(|policy| policy.spending_limit_millionths_of_dollar = 0),
        policy_with(|policy| policy.is_provider_limit_enforced = false),
    ] {
        assert_invalid_rank(rank_pool(
            Tier::T2,
            &calibration,
            &qualification,
            &malformed_policy,
        ));
    }
}

#[test]
fn ranked_pool_preserves_complete_pool_evidence_and_identities() {
    let calibration = promoted_pair_calibration();
    let qualification = vec![
        pool_evidence(PoolStage::Qualification, "beta", true, 12, 12, 0, 0),
        pool_evidence(PoolStage::Qualification, "alpha", true, 6, 6, 0, 0),
    ];

    let pool = rank_pool(Tier::T2, &calibration, &qualification, &policy()).unwrap();

    assert_eq!(pool.tier, Tier::T2);
    assert_eq!(pool.calibration, calibration);
    assert_eq!(pool.promoted, vec![pool_model("alpha"), pool_model("beta")]);
    assert_eq!(pool.qualification, qualification);
    assert_eq!(pool.ranked, vec![pool_model("alpha"), pool_model("beta")]);
    assert!(pool.is_complete);
}

fn promoted_pair_calibration() -> Vec<PoolEntrantEvidence> {
    vec![
        pool_evidence(PoolStage::Calibration, "alpha", true, 4, 4, 0, 0),
        pool_evidence(PoolStage::Calibration, "beta", true, 8, 8, 0, 0),
        pool_evidence(PoolStage::Calibration, "gamma", false, 12, 12, 2, 0),
    ]
}

fn promoted_pair_qualification() -> Vec<PoolEntrantEvidence> {
    vec![
        pool_evidence(PoolStage::Qualification, "alpha", true, 6, 6, 0, 0),
        pool_evidence(PoolStage::Qualification, "beta", true, 12, 12, 0, 0),
    ]
}

fn pool_evidence(
    stage: PoolStage,
    name: &str,
    is_passing: bool,
    candidate_cost: u64,
    candidate_latency: u64,
    failed_trials: u32,
    judge_overhead: u64,
) -> PoolEntrantEvidence {
    let repeats = match stage {
        PoolStage::Calibration => policy().calibration_repeats_per_case,
        PoolStage::Qualification => policy().qualification_repeats_per_case,
    };
    let completed_trials = u32::from(repeats) * 2;
    let candidate_usage = pool_usage(candidate_cost, candidate_latency);
    let judge_usage = pool_usage(judge_overhead, judge_overhead);
    let total_usage = pool_usage(
        candidate_cost + judge_overhead,
        candidate_latency + judge_overhead,
    );

    PoolEntrantEvidence {
        stage,
        requested_model: pool_model(name),
        effective_model: pool_model(name),
        judge_model: ModelIdentity {
            tier: Tier::T5,
            provider: "judge-provider".to_string(),
            model: "judge-model".to_string(),
            thinking: "high".to_string(),
        },
        harnesses: pool_harnesses(stage),
        is_passing,
        completed_trials,
        expected_trials: completed_trials,
        failed_trials,
        catastrophic_trials: 0,
        score: ConfidenceInterval {
            lower: 0.8,
            estimate: 0.8,
            upper: 0.8,
        },
        candidate_usage,
        judge_usage,
        total_usage,
    }
}

fn pool_model(name: &str) -> ModelIdentity {
    ModelIdentity {
        tier: Tier::T2,
        provider: "provider".to_string(),
        model: name.to_string(),
        thinking: "low".to_string(),
    }
}

fn pool_harnesses(stage: PoolStage) -> Vec<HarnessIdentity> {
    let artifact_revision = match stage {
        PoolStage::Calibration => "calibration-exam",
        PoolStage::Qualification => "qualification-exam",
    };
    ["case-a", "case-b"]
        .into_iter()
        .map(|tool_policy_digest| HarnessIdentity {
            runner_version: "runner-1".to_string(),
            pi_version: "pi-1".to_string(),
            artifact_revision: artifact_revision.to_string(),
            tool_policy_digest: tool_policy_digest.to_string(),
        })
        .collect()
}

fn pool_usage(cost: u64, elapsed: u64) -> TrialUsage {
    TrialUsage {
        input_tokens: 0,
        output_tokens: 0,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
        turns: 0,
        tool_calls: 0,
        elapsed_milliseconds: elapsed,
        cost_millionths_of_dollar: cost,
    }
}

fn assert_invalid_rank(result: Result<model::RankedPool, SkillEvalError>) {
    assert!(matches!(
        result,
        Err(SkillEvalError::InvalidConfiguration(_))
    ));
}

fn assert_invalid(result: Result<PoolEntrantEvidence, SkillEvalError>) {
    assert!(matches!(
        result,
        Err(SkillEvalError::InvalidConfiguration(_))
    ));
}

fn policy_with(change: impl FnOnce(&mut PoolPolicy)) -> PoolPolicy {
    let mut result = policy();
    change(&mut result);
    result
}

fn policy() -> PoolPolicy {
    PoolPolicy {
        calibration_repeats_per_case: 2,
        qualification_repeats_per_case: 3,
        promotion_count: 2,
        minimum_score: 7,
        minimum_reliability_basis_points: 7_500,
        maximum_catalog_age_seconds: 7_200,
        spending_limit_millionths_of_dollar: 10_000_000,
        is_provider_limit_enforced: true,
    }
}

fn expected_cases() -> Vec<CaseId> {
    vec![CaseId("a".to_string()), CaseId("b".to_string())]
}

fn trials() -> Vec<TrialRecord> {
    vec![trial("a", 1), trial("a", 2), trial("b", 1), trial("b", 2)]
}

fn trial(case: &str, attempt: u16) -> TrialRecord {
    TrialRecord {
        key: TrialKey {
            artifact: ArtifactName("calibration-exam".to_string()),
            tier: Tier::T2,
            case: CaseId(case.to_string()),
            attempt,
        },
        model: model(),
        harness: HarnessIdentity {
            runner_version: "runner-1".to_string(),
            pi_version: "pi-1".to_string(),
            artifact_revision: "exam-1".to_string(),
            tool_policy_digest: "case-a".to_string(),
        },
        artifact_path: PathBuf::from("artifact.txt"),
        transcript_path: PathBuf::from("transcript.jsonl"),
        candidate_usage: usage(1),
        judge_model: ModelIdentity {
            tier: Tier::T5,
            provider: "judge-provider".to_string(),
            model: "judge-model".to_string(),
            thinking: "high".to_string(),
        },
        judge_usage: usage(1),
        verdict: TrialVerdict {
            score: 7,
            is_catastrophic: false,
            failure_mode: None,
            checks: Vec::new(),
        },
    }
}

fn model() -> ModelIdentity {
    ModelIdentity {
        tier: Tier::T2,
        provider: "provider".to_string(),
        model: "model".to_string(),
        thinking: "low".to_string(),
    }
}

fn failed_check() -> CheckResult {
    CheckResult {
        name: "deterministic".to_string(),
        status: CheckStatus::Failed,
        detail: None,
    }
}

fn usage(value: u64) -> TrialUsage {
    TrialUsage {
        input_tokens: value,
        output_tokens: value,
        cache_read_tokens: value,
        cache_write_tokens: value,
        turns: value as u32,
        tool_calls: value as u32,
        elapsed_milliseconds: value,
        cost_millionths_of_dollar: value,
    }
}
