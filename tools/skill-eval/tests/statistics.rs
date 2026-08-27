#![expect(
    dead_code,
    reason = "the test imports private production modules to exercise crate-private statistics"
)]

#[path = "../src/model.rs"]
mod model;
#[path = "../src/statistics.rs"]
mod statistics;

use std::path::PathBuf;

use model::{
    ArtifactName, CaseId, CheckResult, CheckStatus, ConfidenceInterval, HarnessIdentity,
    ModelIdentity, PoolEntrant, PoolEntrantEvidence, PoolPolicy, PoolStage, SkillEvalError,
    ThinkingDecision, Tier, Timestamp, TrialKey, TrialRecord, TrialUsage, TrialVerdict,
};
use statistics::{
    evaluate_calibration, evaluate_qualification, rank_pool as rank_frozen_pool,
    select_qualification_thinking_level, select_thinking_level,
};

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
fn calibration_and_qualification_use_separate_reliability_floors() {
    let mut split_policy = policy();
    split_policy.qualification_repeats_per_case = 2;
    split_policy.calibration_minimum_reliability_basis_points = 7_500;
    split_policy.qualification_minimum_reliability_basis_points = 10_000;
    let mut evidence = trials();
    evidence[0].verdict.checks.push(failed_check());

    let calibration =
        evaluate_calibration(&model(), &expected_cases(), &evidence, &split_policy).unwrap();
    let qualification =
        evaluate_qualification(&model(), &expected_cases(), &evidence, &split_policy).unwrap();

    assert!(calibration.is_passing);
    assert!(!qualification.is_passing);
    assert_eq!(calibration.failed_trials, 1);
    assert_eq!(qualification.failed_trials, 1);
}

#[test]
fn calibration_floor_returns_evidence_for_quality_and_reliability_failures() {
    let mut low_quality_policy = policy();
    low_quality_policy.calibration_minimum_reliability_basis_points = 0;
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
    strict_policy.calibration_minimum_reliability_basis_points = 7_501;
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
        policy_with(|policy| policy.calibration_minimum_reliability_basis_points = 10_001),
        policy_with(|policy| policy.qualification_minimum_reliability_basis_points = 10_001),
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
fn qualification_evidence_uses_full_repeats_and_preserves_exact_split_evidence() {
    let mut full = qualification_trials();
    full[5].verdict.score = 10;
    full[5].verdict.checks.push(failed_check());
    for trial in &mut full {
        trial.candidate_usage = usage(2);
        trial.judge_usage = usage(3);
    }

    let evidence = evaluate_qualification(&model(), &expected_cases(), &full, &policy()).unwrap();

    assert_eq!(evidence.stage, PoolStage::Qualification);
    assert_eq!(evidence.requested_model, model());
    assert_eq!(evidence.effective_model, model());
    assert_eq!(evidence.judge_model, full[0].judge_model);
    assert_eq!(evidence.completed_trials, 6);
    assert_eq!(evidence.expected_trials, 6);
    assert_eq!(evidence.failed_trials, 1);
    assert_eq!(evidence.catastrophic_trials, 0);
    assert!(!evidence.is_passing);
    assert_eq!(evidence.candidate_usage, usage(12));
    assert_eq!(evidence.judge_usage, usage(18));
    assert_eq!(evidence.total_usage, usage(30));
}

#[test]
fn qualification_evidence_rejects_calibration_repeat_counts() {
    assert_invalid(evaluate_qualification(
        &model(),
        &expected_cases(),
        &trials(),
        &policy(),
    ));
}

#[test]
fn qualification_evidence_applies_identity_harness_floor_and_catastrophic_rules() {
    let mut identity_drift = qualification_trials();
    identity_drift[5].model.thinking = "high".to_string();
    assert_invalid(evaluate_qualification(
        &model(),
        &expected_cases(),
        &identity_drift,
        &policy(),
    ));

    let mut harness_drift = qualification_trials();
    harness_drift[1].harness.tool_policy_digest = "other".to_string();
    assert_invalid(evaluate_qualification(
        &model(),
        &expected_cases(),
        &harness_drift,
        &policy(),
    ));

    let mut catastrophic = qualification_trials();
    catastrophic[0].verdict.is_catastrophic = true;
    let evidence =
        evaluate_qualification(&model(), &expected_cases(), &catastrophic, &policy()).unwrap();
    assert!(!evidence.is_passing);
    assert_eq!(evidence.catastrophic_trials, 1);
}

#[test]
fn screens_every_supported_thinking_level() {
    let levels = ["off", "minimal", "low", "medium", "high", "xhigh", "max"];
    let entrant = thinking_entrant("medium", &levels);
    let evidence = levels
        .iter()
        .enumerate()
        .map(|(index, level)| thinking_evidence(level, index == 1 || index == 4))
        .collect::<Vec<_>>();

    for completed in 0..levels.len() {
        assert_eq!(
            select_thinking_level(&entrant, &evidence[..completed]).unwrap(),
            next_thinking(u8::try_from(completed).unwrap())
        );
    }
    assert_eq!(
        select_thinking_level(&entrant, &evidence).unwrap(),
        complete_thinking(Some(thinking_model("minimal")))
    );

    let all_failed = levels
        .iter()
        .map(|level| thinking_evidence(level, false))
        .collect::<Vec<_>>();
    assert_eq!(
        select_thinking_level(&entrant, &all_failed).unwrap(),
        complete_thinking(None)
    );

    let fixed = thinking_entrant("off", &["off"]);
    assert_eq!(
        select_thinking_level(&fixed, &[]).unwrap(),
        next_thinking(0)
    );
    assert_eq!(
        select_thinking_level(&fixed, &[thinking_evidence("off", true)]).unwrap(),
        complete_thinking(Some(thinking_model("off")))
    );
}

#[test]
fn retained_lower_route_continues_stronger() {
    let mut entrant = thinking_entrant("medium", &["off", "medium", "high"]);
    entrant.retained_lower_thinking_level = Some("off".to_owned());
    let calibration = vec![
        thinking_evidence("off", true),
        thinking_evidence("medium", true),
        thinking_evidence("high", true),
    ];

    assert_eq!(
        select_qualification_thinking_level(&entrant, &calibration, &[]).unwrap(),
        ThinkingDecision {
            selected: None,
            retained_lower: None,
            next_thinking_index: Some(0),
            is_complete: false,
        }
    );

    let lower = qualification_thinking_evidence("off", 0);
    assert_eq!(
        select_qualification_thinking_level(&entrant, &calibration, std::slice::from_ref(&lower))
            .unwrap(),
        ThinkingDecision {
            selected: None,
            retained_lower: Some(thinking_model("off")),
            next_thinking_index: Some(1),
            is_complete: false,
        }
    );

    let medium = qualification_thinking_evidence("medium", 1);
    let high = qualification_thinking_evidence("high", 0);
    assert_eq!(
        select_qualification_thinking_level(
            &entrant,
            &calibration,
            &[lower.clone(), medium, high],
        )
        .unwrap(),
        ThinkingDecision {
            selected: Some(thinking_model("high")),
            retained_lower: Some(thinking_model("off")),
            next_thinking_index: None,
            is_complete: true,
        }
    );

    let failed_lower = qualification_thinking_evidence("off", 1);
    assert_eq!(
        select_qualification_thinking_level(&entrant, &calibration, &[failed_lower]).unwrap(),
        ThinkingDecision {
            selected: None,
            retained_lower: None,
            next_thinking_index: Some(1),
            is_complete: false,
        }
    );

    let failed_screen = vec![
        thinking_evidence("off", false),
        thinking_evidence("medium", true),
        thinking_evidence("high", true),
    ];
    assert_eq!(
        select_qualification_thinking_level(&entrant, &failed_screen, &[]).unwrap(),
        ThinkingDecision {
            selected: None,
            retained_lower: None,
            next_thinking_index: Some(1),
            is_complete: false,
        }
    );
    assert_invalid_thinking(select_qualification_thinking_level(
        &entrant,
        &failed_screen,
        &[lower],
    ));

    let mut beta = thinking_entrant("low", &["low", "high"]);
    beta.model.model = "beta".to_owned();
    let mut gamma = thinking_entrant("low", &["low"]);
    gamma.model.model = "gamma".to_owned();
    let entrants = [entrant.clone(), beta, gamma];
    let retarget = |mut evidence: PoolEntrantEvidence, model: &str| {
        evidence.requested_model.model = model.to_owned();
        evidence.effective_model.model = model.to_owned();
        evidence
    };
    let frozen_calibration = vec![
        thinking_evidence("off", true),
        thinking_evidence("medium", true),
        thinking_evidence("high", true),
        retarget(thinking_evidence("low", true), "beta"),
        retarget(thinking_evidence("high", true), "beta"),
        retarget(thinking_evidence("low", true), "gamma"),
    ];
    let retained = qualification_thinking_evidence("off", 0);
    let stronger_failure = qualification_thinking_evidence("medium", 1);
    let selected_stronger = qualification_thinking_evidence("high", 0);
    let beta_final = retarget(qualification_thinking_evidence("low", 0), "beta");
    let gamma_final = retarget(qualification_thinking_evidence("low", 0), "gamma");
    let qualification = vec![
        retained.clone(),
        stronger_failure,
        selected_stronger,
        beta_final.clone(),
        gamma_final,
    ];
    let ranking_policy = policy_with(|policy| policy.calibration_repeats_per_case = 1);
    let ranked = rank_frozen_pool(
        Tier::T2,
        &entrants,
        &frozen_calibration,
        &qualification,
        &ranking_policy,
    )
    .unwrap();
    assert_eq!(ranked.retained_lower_routes, vec![thinking_model("off")]);
    assert!(ranked.ranked.contains(&thinking_model("high")));
    assert!(!ranked.ranked.contains(&thinking_model("off")));

    let mut wrong_level = qualification.clone();
    wrong_level.remove(0);
    assert_invalid_rank(rank_frozen_pool(
        Tier::T2,
        &entrants,
        &frozen_calibration,
        &wrong_level,
        &ranking_policy,
    ));

    let mut foreign = qualification.clone();
    foreign.push(retarget(
        qualification_thinking_evidence("low", 0),
        "foreign",
    ));
    assert_invalid_rank(rank_frozen_pool(
        Tier::T2,
        &entrants,
        &frozen_calibration,
        &foreign,
        &ranking_policy,
    ));

    let mut normal_forged = qualification.clone();
    normal_forged.push(retarget(qualification_thinking_evidence("high", 0), "beta"));
    assert_invalid_rank(rank_frozen_pool(
        Tier::T2,
        &entrants,
        &frozen_calibration,
        &normal_forged,
        &ranking_policy,
    ));

    assert_invalid_rank(rank_frozen_pool(
        Tier::T2,
        &entrants,
        &frozen_calibration[..frozen_calibration.len() - 1],
        &qualification,
        &ranking_policy,
    ));
    let mut incomplete_retained = qualification;
    incomplete_retained[0].completed_trials = 14;
    assert_invalid_rank(rank_frozen_pool(
        Tier::T2,
        &entrants,
        &frozen_calibration,
        &incomplete_retained,
        &ranking_policy,
    ));
}

#[test]
fn qualification_rejects_incomplete_calibration_shape() {
    let entrant = thinking_entrant("low", &["low"]);
    let valid = vec![thinking_evidence("low", true)];

    assert_eq!(
        select_qualification_thinking_level(&entrant, &valid, &[]).unwrap(),
        next_thinking(0)
    );

    let mut wrong_harness_count = valid.clone();
    wrong_harness_count[0].harnesses.pop();
    assert_invalid_thinking(select_qualification_thinking_level(
        &entrant,
        &wrong_harness_count,
        &[],
    ));

    let mut wrong_completed_count = valid.clone();
    wrong_completed_count[0].completed_trials = 4;
    assert_invalid_thinking(select_qualification_thinking_level(
        &entrant,
        &wrong_completed_count,
        &[],
    ));

    let mut wrong_expected_count = valid.clone();
    wrong_expected_count[0].expected_trials = 4;
    assert_invalid_thinking(select_qualification_thinking_level(
        &entrant,
        &wrong_expected_count,
        &[],
    ));
}

#[test]
fn qualification_starts_at_cheapest_screening_pass() {
    let entrant = thinking_entrant("medium", &["low", "medium", "high"]);
    let calibration = vec![
        thinking_evidence("low", false),
        thinking_evidence("medium", true),
        thinking_evidence("high", true),
    ];

    assert_eq!(
        select_qualification_thinking_level(&entrant, &calibration, &[]).unwrap(),
        next_thinking(1)
    );
}

#[test]
fn qualification_failure_advances_thinking() {
    let entrant = thinking_entrant("medium", &["low", "medium", "high"]);
    let calibration = vec![
        thinking_evidence("low", false),
        thinking_evidence("medium", true),
        thinking_evidence("high", false),
    ];
    let qualification = vec![qualification_thinking_evidence("medium", 1)];

    assert_eq!(
        select_qualification_thinking_level(&entrant, &calibration, &qualification).unwrap(),
        next_thinking(2)
    );
}

#[test]
fn first_fully_reliable_level_stops_model() {
    let entrant = thinking_entrant("medium", &["low", "medium", "high"]);
    let calibration = vec![
        thinking_evidence("low", false),
        thinking_evidence("medium", true),
        thinking_evidence("high", false),
    ];
    let qualification = vec![
        qualification_thinking_evidence("medium", 1),
        qualification_thinking_evidence("high", 0),
    ];

    assert_eq!(
        select_qualification_thinking_level(&entrant, &calibration, &qualification).unwrap(),
        complete_thinking(Some(thinking_model("high")))
    );
}

#[test]
fn qualification_history_rejects_invalid_shapes() {
    let entrant = thinking_entrant("medium", &["low", "medium", "high"]);
    let complete = vec![
        thinking_evidence("low", false),
        thinking_evidence("medium", true),
        thinking_evidence("high", false),
    ];
    let no_pass = vec![
        thinking_evidence("low", false),
        thinking_evidence("medium", false),
        thinking_evidence("high", false),
    ];
    assert_invalid_thinking(select_qualification_thinking_level(
        &entrant,
        &complete[..2],
        &[],
    ));
    assert_invalid_thinking(select_qualification_thinking_level(&entrant, &no_pass, &[]));

    let mut incomplete_calibration = complete.clone();
    incomplete_calibration[0].completed_trials = 4;
    assert_invalid_thinking(select_qualification_thinking_level(
        &entrant,
        &incomplete_calibration,
        &[],
    ));

    let histories = [
        vec![qualification_thinking_evidence("high", 1)],
        vec![
            qualification_thinking_evidence("medium", 1),
            qualification_thinking_evidence("low", 1),
        ],
        vec![
            qualification_thinking_evidence("medium", 1),
            qualification_thinking_evidence("medium", 1),
        ],
        vec![
            qualification_thinking_evidence("medium", 0),
            qualification_thinking_evidence("high", 0),
        ],
    ];
    for history in histories {
        assert_invalid_thinking(select_qualification_thinking_level(
            &entrant, &complete, &history,
        ));
    }

    let mut incomplete = qualification_thinking_evidence("medium", 1);
    incomplete.completed_trials = 14;
    assert_invalid_thinking(select_qualification_thinking_level(
        &entrant,
        &complete,
        &[incomplete],
    ));

    let mut foreign = qualification_thinking_evidence("medium", 1);
    foreign.requested_model.model = "foreign".to_owned();
    foreign.effective_model.model = "foreign".to_owned();
    assert_invalid_thinking(select_qualification_thinking_level(
        &entrant,
        &complete,
        &[foreign],
    ));
}

#[test]
fn thinking_evidence_rejects_invalid_history() {
    for entrant in [
        thinking_entrant("low", &[]),
        thinking_entrant(
            "low",
            &[
                "off", "minimal", "low", "medium", "high", "xhigh", "max", "max",
            ],
        ),
        thinking_entrant("low", &["low", "low"]),
        thinking_entrant("low", &["medium", "low"]),
        thinking_entrant("low", &["unknown"]),
        thinking_entrant("medium", &["low", "high"]),
    ] {
        assert_invalid_thinking(select_thinking_level(&entrant, &[]));
    }

    let entrant = thinking_entrant("medium", &["low", "medium", "high"]);
    let mut invalid_histories = vec![
        vec![thinking_evidence("medium", true)],
        vec![
            thinking_evidence("low", false),
            thinking_evidence("high", true),
        ],
        vec![
            thinking_evidence("medium", false),
            thinking_evidence("low", true),
        ],
        vec![
            thinking_evidence("low", true),
            thinking_evidence("low", true),
        ],
        vec![thinking_evidence("xhigh", true)],
    ];

    let mut item = thinking_evidence("low", true);
    item.stage = PoolStage::Qualification;
    invalid_histories.push(vec![item]);

    let mut item = thinking_evidence("low", true);
    item.requested_model.tier = Tier::T3;
    item.effective_model.tier = Tier::T3;
    invalid_histories.push(vec![item]);

    let mut item = thinking_evidence("low", true);
    item.requested_model.provider = "foreign".to_string();
    item.effective_model.provider = "foreign".to_string();
    invalid_histories.push(vec![item]);

    let mut item = thinking_evidence("low", true);
    item.requested_model.model = "foreign".to_string();
    item.effective_model.model = "foreign".to_string();
    invalid_histories.push(vec![item]);

    let mut item = thinking_evidence("low", true);
    item.effective_model.thinking = "medium".to_string();
    invalid_histories.push(vec![item]);

    let mut item = thinking_evidence("low", true);
    item.judge_model.provider = item.effective_model.provider.clone();
    item.judge_model.model = item.effective_model.model.clone();
    invalid_histories.push(vec![item]);

    let mut item = thinking_evidence("low", true);
    item.harnesses.clear();
    invalid_histories.push(vec![item]);

    let mut item = thinking_evidence("low", true);
    item.completed_trials -= 1;
    invalid_histories.push(vec![item]);

    let mut item = thinking_evidence("low", true);
    item.failed_trials = item.completed_trials + 1;
    invalid_histories.push(vec![item]);

    let mut item = thinking_evidence("low", true);
    item.catastrophic_trials = 1;
    invalid_histories.push(vec![item]);

    let mut item = thinking_evidence("low", true);
    item.score.estimate = f64::NAN;
    invalid_histories.push(vec![item]);

    let mut item = thinking_evidence("low", true);
    item.total_usage.input_tokens += 1;
    invalid_histories.push(vec![item]);

    let mut item = thinking_evidence("low", true);
    item.candidate_usage.input_tokens = u64::MAX;
    item.judge_usage.input_tokens = 1;
    invalid_histories.push(vec![item]);

    for evidence in invalid_histories {
        assert_invalid_thinking(select_thinking_level(&entrant, &evidence));
    }
}

#[test]
fn accepts_four_entrants() {
    let calibration = (0..4)
        .map(|index| {
            pool_evidence(
                PoolStage::Calibration,
                ["alpha", "beta", "gamma", "delta"][index],
                true,
                u64::try_from(index + 1).unwrap(),
                1,
                0,
                0,
            )
        })
        .collect::<Vec<_>>();
    let qualification = (0..4)
        .map(|index| {
            pool_evidence(
                PoolStage::Qualification,
                ["alpha", "beta", "gamma", "delta"][index],
                true,
                u64::try_from(index + 1).unwrap(),
                1,
                0,
                0,
            )
        })
        .collect::<Vec<_>>();

    let pool = rank_pool(Tier::T2, &calibration, &qualification, &policy()).unwrap();

    assert_eq!(pool.calibration, calibration);
    assert_eq!(pool.qualification, qualification);
    assert_eq!(pool.promoted.len(), 4);
    assert_eq!(pool.ranked.len(), 4);
    assert!(pool.is_complete);

    let missing = rank_pool(Tier::T2, &calibration, &qualification[..3], &policy()).unwrap();
    assert!(!missing.is_complete);
    assert!(missing.ranked.is_empty());
    assert_eq!(missing.qualification.len(), 3);

    let mut reordered = qualification.clone();
    reordered.swap(2, 3);
    assert_invalid_rank(rank_pool(Tier::T2, &calibration, &reordered, &policy()));

    let mut duplicate = qualification.clone();
    duplicate[3] = duplicate[2].clone();
    assert_invalid_rank(rank_pool(Tier::T2, &calibration, &duplicate, &policy()));

    let mut foreign = qualification.clone();
    foreign[3].requested_model.model = "foreign".to_owned();
    foreign[3].effective_model.model = "foreign".to_owned();
    assert_invalid_rank(rank_pool(Tier::T2, &calibration, &foreign, &policy()));

    for entrant_count in 0..3 {
        assert_invalid_rank(rank_pool(
            Tier::T2,
            &calibration[..entrant_count],
            &qualification[..entrant_count],
            &policy(),
        ));
    }
}

#[test]
fn ranked_pool_uses_candidate_task_cost_before_other_metrics() {
    let calibration = promoted_pair_calibration();
    let qualification = vec![
        pool_evidence(PoolStage::Qualification, "alpha", true, 6, 60, 0, 0),
        pool_evidence(PoolStage::Qualification, "beta", true, 12, 6, 0, 0),
    ];

    let pool = rank_pool(Tier::T2, &calibration, &qualification, &policy()).unwrap();

    assert_eq!(pool.ranked, vec![pool_model("alpha"), pool_model("beta")]);
}

#[test]
fn ranked_pool_uses_candidate_latency_after_task_cost() {
    let calibration = promoted_pair_calibration();
    let qualification = vec![
        pool_evidence(PoolStage::Qualification, "alpha", true, 6, 6, 0, 0),
        pool_evidence(PoolStage::Qualification, "beta", true, 6, 12, 0, 0),
    ];

    let pool = rank_pool(Tier::T2, &calibration, &qualification, &policy()).unwrap();

    assert_eq!(pool.ranked, vec![pool_model("alpha"), pool_model("beta")]);
}

#[test]
fn ranked_pool_rejects_a_passing_identity_below_15_of_15() {
    let calibration = promoted_pair_calibration();
    let qualification = vec![
        pool_evidence(PoolStage::Qualification, "alpha", true, 6, 6, 0, 0),
        pool_evidence(PoolStage::Qualification, "beta", true, 6, 6, 1, 0),
    ];

    assert_invalid_rank(rank_pool(Tier::T2, &calibration, &qualification, &policy()));
}

#[test]
fn ranked_pool_uses_exact_identity_as_the_final_tie_break() {
    let calibration = promoted_pair_calibration();
    let qualification = vec![
        pool_evidence(PoolStage::Qualification, "alpha", true, 6, 6, 0, 0),
        pool_evidence(PoolStage::Qualification, "beta", true, 6, 6, 0, 0),
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
fn ranked_pool_qualifies_every_passing_calibration_model_in_plan_order() {
    let calibration = vec![
        pool_evidence(PoolStage::Calibration, "expensive", true, 40, 4, 0, 0),
        pool_evidence(PoolStage::Calibration, "slower", true, 20, 8, 0, 0),
        pool_evidence(PoolStage::Calibration, "best", true, 20, 4, 1, 0),
    ];

    let pool = rank_pool(Tier::T2, &calibration, &[], &policy()).unwrap();

    assert_eq!(
        pool.promoted,
        vec![
            pool_model("expensive"),
            pool_model("slower"),
            pool_model("best"),
        ]
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
    assert_eq!(failed.ranked, vec![pool_model("alpha")]);
}

#[test]
fn ranked_pool_does_not_hide_an_unfinished_calibration_passer() {
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

    assert_eq!(
        pool.promoted,
        vec![pool_model("alpha"), pool_model("beta"), pool_model("gamma"),]
    );
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
        policy_with(|policy| policy.calibration_minimum_reliability_basis_points = 10_001),
        policy_with(|policy| policy.qualification_minimum_reliability_basis_points = 10_001),
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
        pool_evidence(PoolStage::Qualification, "alpha", true, 6, 6, 0, 0),
        pool_evidence(PoolStage::Qualification, "beta", true, 12, 12, 0, 0),
    ];

    let pool = rank_pool(Tier::T2, &calibration, &qualification, &policy()).unwrap();

    assert_eq!(pool.tier, Tier::T2);
    assert_eq!(pool.calibration, calibration);
    assert_eq!(pool.promoted, vec![pool_model("alpha"), pool_model("beta")]);
    assert_eq!(pool.qualification, qualification);
    assert_eq!(pool.ranked, vec![pool_model("alpha"), pool_model("beta")]);
    assert!(pool.is_complete);
}

fn thinking_entrant(start: &str, levels: &[&str]) -> PoolEntrant {
    PoolEntrant {
        model: thinking_model(start),
        thinking_levels: levels.iter().map(|level| (*level).to_string()).collect(),
        retained_lower_thinking_level: None,
        candidate_timeout_seconds: None,
        catalog_observed_at: Timestamp("2026-08-24T11:59:00-0400".to_string()),
    }
}

fn thinking_evidence(level: &str, is_passing: bool) -> PoolEntrantEvidence {
    let failed_trials = u32::from(!is_passing) * 2;
    let mut evidence = pool_evidence(
        PoolStage::Calibration,
        "adaptive",
        is_passing,
        4,
        4,
        failed_trials,
        0,
    );
    evidence.requested_model.thinking = level.to_string();
    evidence.effective_model.thinking = level.to_string();
    evidence.harnesses = thinking_harnesses();
    evidence.completed_trials = 5;
    evidence.expected_trials = 5;
    evidence
}

fn thinking_harnesses() -> Vec<HarnessIdentity> {
    ['a', 'b', 'c', 'd', 'e']
        .into_iter()
        .map(|case| HarnessIdentity {
            runner_version: "runner-1".to_string(),
            pi_version: "pi-1".to_string(),
            artifact_revision: "calibration-exam".to_string(),
            tool_policy_digest: format!("case-{case}"),
        })
        .collect()
}

fn qualification_thinking_evidence(level: &str, failed_trials: u32) -> PoolEntrantEvidence {
    let mut evidence = pool_evidence(
        PoolStage::Qualification,
        "adaptive",
        failed_trials == 0,
        15,
        15,
        failed_trials,
        0,
    );
    evidence.requested_model.thinking = level.to_owned();
    evidence.effective_model.thinking = level.to_owned();
    evidence
}

fn thinking_model(level: &str) -> ModelIdentity {
    ModelIdentity {
        tier: Tier::T2,
        provider: "provider".to_string(),
        model: "adaptive".to_string(),
        thinking: level.to_string(),
    }
}

fn next_thinking(index: u8) -> ThinkingDecision {
    ThinkingDecision {
        selected: None,
        retained_lower: None,
        next_thinking_index: Some(index),
        is_complete: false,
    }
}

fn complete_thinking(selected: Option<ModelIdentity>) -> ThinkingDecision {
    ThinkingDecision {
        selected,
        retained_lower: None,
        next_thinking_index: None,
        is_complete: true,
    }
}

fn assert_invalid_thinking(result: Result<ThinkingDecision, SkillEvalError>) {
    assert!(matches!(
        result,
        Err(SkillEvalError::InvalidConfiguration(_))
    ));
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
    let case_count = match stage {
        PoolStage::Calibration => 2,
        PoolStage::Qualification => 5,
    };
    let completed_trials = u32::from(repeats) * case_count;
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
    let cases = match stage {
        PoolStage::Calibration => &['a', 'b'][..],
        PoolStage::Qualification => &['a', 'b', 'c', 'd', 'e'][..],
    };
    cases
        .iter()
        .map(|case| HarnessIdentity {
            runner_version: "runner-1".to_string(),
            pi_version: "pi-1".to_string(),
            artifact_revision: artifact_revision.to_string(),
            tool_policy_digest: format!("case-{case}"),
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

fn rank_pool(
    tier: Tier,
    calibration: &[PoolEntrantEvidence],
    qualification: &[PoolEntrantEvidence],
    policy: &PoolPolicy,
) -> Result<model::RankedPool, SkillEvalError> {
    let mut entrants = Vec::<PoolEntrant>::new();
    for evidence in calibration {
        if let Some(entrant) = entrants.last_mut().filter(|entrant| {
            entrant.model.tier == evidence.requested_model.tier
                && entrant.model.provider == evidence.requested_model.provider
                && entrant.model.model == evidence.requested_model.model
        }) {
            entrant
                .thinking_levels
                .push(evidence.requested_model.thinking.clone());
        } else {
            entrants.push(PoolEntrant {
                model: evidence.requested_model.clone(),
                thinking_levels: vec![evidence.requested_model.thinking.clone()],
                retained_lower_thinking_level: None,
                candidate_timeout_seconds: None,
                catalog_observed_at: Timestamp(String::new()),
            });
        }
    }
    rank_frozen_pool(tier, &entrants, calibration, qualification, policy)
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
        calibration_minimum_reliability_basis_points: 7_500,
        qualification_minimum_reliability_basis_points: 10_000,
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

fn qualification_trials() -> Vec<TrialRecord> {
    vec![
        trial("a", 1),
        trial("a", 2),
        trial("a", 3),
        trial("b", 1),
        trial("b", 2),
        trial("b", 3),
    ]
}

fn trial(case: &str, attempt: u16) -> TrialRecord {
    TrialRecord {
        key: TrialKey {
            artifact: ArtifactName("calibration-exam".to_string()),
            tier: Tier::T2,
            route_index: 0,
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
