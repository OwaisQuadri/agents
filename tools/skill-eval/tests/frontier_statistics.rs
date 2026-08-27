#![expect(
    dead_code,
    reason = "the test imports private production modules to exercise crate-private statistics"
)]
#![expect(
    clippy::large_enum_variant,
    reason = "the test imports frozen production model declarations without changing their shapes"
)]

#[path = "../src/model.rs"]
mod model;
#[path = "../src/statistics.rs"]
mod statistics;

use std::collections::BTreeMap;
use std::path::PathBuf;

use model::{
    ArtifactName, CaseId, FrontierBaselineChange, FrontierCaseGroup, FrontierCaseReference,
    FrontierCellEvidence, FrontierCellStatus, FrontierConfidenceMethod, FrontierEntrant,
    FrontierModelProgress, FrontierModelReport, FrontierPolicy, FrontierScore, FrontierTierSuite,
    HarnessIdentity, ModelIdentity, Tier, Timestamp, TrialKey, TrialRecord, TrialUsage,
    TrialVerdict,
};
use statistics::{advance_frontier_model, evaluate_frontier_cell, rank_frontier_pools};

#[test]
fn frontier_cell_is_deterministic_and_passes_complete_evidence() {
    let suite = suite();
    let trials = trials(&suite, &route("alpha", Tier::T1, "low"), 3, 10);

    let first = evaluate_frontier_cell(&suite, &trials[0].model, &trials, &policy()).unwrap();
    let second = evaluate_frontier_cell(&suite, &trials[0].model, &trials, &policy()).unwrap();

    assert_eq!(first, second);
    assert_eq!(first.status, FrontierCellStatus::Passed);
    assert_eq!(first.completed_trials, 12);
    assert_eq!(first.expected_trials, 12);
    assert_eq!(first.failed_trials, 0);
    assert_eq!(first.score.unwrap().weighted_pass_basis_points, 10_000);
}

#[test]
fn frontier_cell_rejects_malformed_attempts_and_missing_groups() {
    let suite = suite();
    let model = route("alpha", Tier::T1, "low");
    let mut malformed = trials(&suite, &model, 3, 10);
    malformed[1].key.attempt = 4;
    assert!(evaluate_frontier_cell(&suite, &model, &malformed, &policy()).is_err());

    let mut incomplete_suite = suite;
    incomplete_suite
        .cases
        .retain(|case| case.group != FrontierCaseGroup::Edge);
    let incomplete = trials(&incomplete_suite, &model, 1, 10);
    assert!(evaluate_frontier_cell(&incomplete_suite, &model, &incomplete, &policy()).is_err());
}

#[test]
fn frontier_cell_fails_a_critical_trial_and_applies_weighted_threshold() {
    let suite = suite();
    let model = route("alpha", Tier::T1, "low");
    let mut critical = trials(&suite, &model, 1, 10);
    let critical_trial = critical
        .iter_mut()
        .find(|trial| trial.key.case.0 == "critical")
        .unwrap();
    critical_trial.verdict.score = 6;
    let evidence = evaluate_frontier_cell(&suite, &model, &critical, &policy()).unwrap();
    assert_eq!(evidence.status, FrontierCellStatus::Failed);
    let score = evidence.score.unwrap();
    assert_eq!(score.critical_passed_trials, 0);
    assert_eq!(score.critical_expected_trials, 1);

    let mut weighted_suite = suite;
    weighted_suite.group_weights_basis_points = BTreeMap::from([
        (FrontierCaseGroup::Normal, 8_500),
        (FrontierCaseGroup::Edge, 500),
        (FrontierCaseGroup::Adversarial, 500),
        (FrontierCaseGroup::Critical, 500),
    ]);
    let mut weighted = trials(&weighted_suite, &model, 1, 10);
    weighted
        .iter_mut()
        .find(|trial| trial.key.case.0 == "normal")
        .unwrap()
        .verdict
        .score = 6;
    let evidence = evaluate_frontier_cell(&weighted_suite, &model, &weighted, &policy()).unwrap();
    assert_eq!(evidence.score.unwrap().weighted_pass_basis_points, 1_500);
    assert_eq!(evidence.status, FrontierCellStatus::Failed);
}

#[test]
fn frontier_cell_uses_the_bootstrap_lower_bound_for_pending_and_terminal_results() {
    let mut suite = suite();
    suite.group_weights_basis_points = BTreeMap::from([
        (FrontierCaseGroup::Normal, 9_700),
        (FrontierCaseGroup::Edge, 100),
        (FrontierCaseGroup::Adversarial, 100),
        (FrontierCaseGroup::Critical, 100),
    ]);
    for index in 1..10 {
        suite
            .cases
            .push(case(&format!("normal-{index}"), FrontierCaseGroup::Normal));
    }
    let model = route("alpha", Tier::T1, "low");
    let mut screening = trials(&suite, &model, 1, 10);
    screening
        .iter_mut()
        .find(|trial| trial.key.case.0 == "normal-1")
        .unwrap()
        .verdict
        .score = 6;
    let evidence = evaluate_frontier_cell(&suite, &model, &screening, &policy()).unwrap();
    let score = evidence.score.unwrap();
    assert_eq!(score.weighted_pass_basis_points, 9_030);
    assert!(score.lower_bound_basis_points < 8_000);
    assert_eq!(evidence.status, FrontierCellStatus::Pending);

    let mut maximum = trials(&suite, &model, 5, 10);
    for trial in &mut maximum {
        if trial.key.case.0 == "normal-1" {
            trial.verdict.score = 6;
        }
    }
    let evidence = evaluate_frontier_cell(&suite, &model, &maximum, &policy()).unwrap();
    assert_eq!(evidence.status, FrontierCellStatus::Indeterminate);
}

#[test]
fn frontier_cell_rejects_usage_overflow() {
    let suite = suite();
    let model = route("alpha", Tier::T1, "low");
    let mut evidence = trials(&suite, &model, 1, 10);
    evidence[0].candidate_usage.input_tokens = u64::MAX;
    evidence[1].candidate_usage.input_tokens = 1;

    assert!(evaluate_frontier_cell(&suite, &model, &evidence, &policy()).is_err());
}

#[test]
fn frontier_progression_advances_levels_and_uses_the_next_stronger_cross_tier_level() {
    let entrant = entrant();
    let progress = initial_progress(&entrant);
    let failed = cell(
        route("alpha", Tier::T1, "low"),
        FrontierCellStatus::Failed,
        8_000,
        1,
    );
    let progress =
        advance_frontier_model(&entrant, &progress, std::slice::from_ref(&failed)).unwrap();
    assert_eq!(progress.next_tier, Some(Tier::T1));
    assert_eq!(progress.next_thinking_index, Some(1));

    let passed = cell(
        route("alpha", Tier::T1, "medium"),
        FrontierCellStatus::Passed,
        9_000,
        1,
    );
    let progress =
        advance_frontier_model(&entrant, &progress, &[failed.clone(), passed.clone()]).unwrap();
    assert_eq!(progress.selected_routes, vec![passed.model.clone()]);
    assert_eq!(progress.next_tier, Some(Tier::T2));
    assert_eq!(progress.next_thinking_index, Some(2));

    let duplicate = vec![failed.clone(), failed];
    assert!(advance_frontier_model(&entrant, &initial_progress(&entrant), &duplicate).is_err());
}

#[test]
fn frontier_progression_marks_level_and_tier_exhaustion() {
    let entrant = entrant();
    let cells = vec![
        cell(
            route("alpha", Tier::T1, "low"),
            FrontierCellStatus::Failed,
            8_000,
            1,
        ),
        cell(
            route("alpha", Tier::T1, "medium"),
            FrontierCellStatus::Failed,
            8_000,
            1,
        ),
        cell(
            route("alpha", Tier::T1, "high"),
            FrontierCellStatus::Failed,
            8_000,
            1,
        ),
    ];
    let progress = advance_frontier_model(&entrant, &initial_progress(&entrant), &cells).unwrap();
    assert!(progress.is_exhausted);
    assert_eq!(progress.next_tier, None);
    assert_eq!(progress.next_thinking_index, None);

    let t5_entrant = FrontierEntrant {
        entry_tier: Tier::T5,
        thinking_levels: vec!["low".to_string()],
        ..entrant
    };
    let passed = cell(
        route("alpha", Tier::T5, "low"),
        FrontierCellStatus::Passed,
        10_000,
        1,
    );
    let progress = advance_frontier_model(
        &t5_entrant,
        &initial_progress(&t5_entrant),
        std::slice::from_ref(&passed),
    )
    .unwrap();
    assert!(progress.is_exhausted);
    assert_eq!(progress.selected_routes, vec![passed.model]);
}

#[test]
fn frontier_ranking_orders_quality_before_cost_and_marks_only_the_top_five() {
    let mut reports = vec![
        report("cheap", 9_000, 1),
        report("quality", 10_000, 100),
        report("tie-cheap", 9_000, 2),
        report("four", 8_900, 1),
        report("five", 8_800, 1),
        report("six", 8_700, 1),
    ];
    let ignored_capability_neutral_membership = model::FrontierPoolMembership {
        model: reports[0].selected_routes[0].clone(),
        rank: 99,
        is_active: false,
    };
    reports[0]
        .pool_memberships
        .insert(Tier::T1, ignored_capability_neutral_membership);

    let pools = rank_frontier_pools(&reports, 5).unwrap();
    let ranked = &pools[&Tier::T1];
    assert_eq!(ranked[0].model.model, "quality");
    assert_eq!(ranked[1].model.model, "cheap");
    assert_eq!(ranked[2].model.model, "tie-cheap");
    assert_eq!(ranked.iter().filter(|item| item.is_active).count(), 5);
    assert!(!ranked[5].is_active);
}

#[test]
fn frontier_ranking_rejects_duplicates_incomplete_usage_and_zero_active_size() {
    let report = report("alpha", 9_000, 1);
    assert!(rank_frontier_pools(std::slice::from_ref(&report), 0).is_err());
    assert!(rank_frontier_pools(&[report.clone(), report.clone()], 5).is_err());

    let mut incomplete = report;
    incomplete.total_usage.input_tokens += 1;
    assert!(rank_frontier_pools(&[incomplete], 5).is_err());
}

fn suite() -> FrontierTierSuite {
    FrontierTierSuite {
        group_weights_basis_points: BTreeMap::from([
            (FrontierCaseGroup::Normal, 2_500),
            (FrontierCaseGroup::Edge, 2_500),
            (FrontierCaseGroup::Adversarial, 2_500),
            (FrontierCaseGroup::Critical, 2_500),
        ]),
        cases: vec![
            case("normal", FrontierCaseGroup::Normal),
            case("edge", FrontierCaseGroup::Edge),
            case("adversarial", FrontierCaseGroup::Adversarial),
            case("critical", FrontierCaseGroup::Critical),
        ],
    }
}

fn case(name: &str, group: FrontierCaseGroup) -> FrontierCaseReference {
    FrontierCaseReference {
        artifact_path: PathBuf::from(format!("artifact-{name}")),
        artifact_revision: format!("revision-{name}"),
        case: CaseId(name.to_string()),
        group,
        is_confirmation: group == FrontierCaseGroup::Critical,
    }
}

fn trials(
    suite: &FrontierTierSuite,
    model: &ModelIdentity,
    attempts: u16,
    score: u8,
) -> Vec<TrialRecord> {
    suite
        .cases
        .iter()
        .flat_map(|case| {
            (1..=attempts).map(move |attempt| TrialRecord {
                key: TrialKey {
                    artifact: ArtifactName(case.artifact_path.to_string_lossy().into_owned()),
                    tier: model.tier,
                    route_index: 0,
                    case: case.case.clone(),
                    attempt,
                },
                model: model.clone(),
                harness: HarnessIdentity {
                    runner_version: "runner".to_string(),
                    pi_version: "pi".to_string(),
                    artifact_revision: case.artifact_revision.clone(),
                    tool_policy_digest: format!("policy-{}", case.case.0),
                },
                artifact_path: PathBuf::from("result"),
                transcript_path: PathBuf::from("transcript"),
                candidate_usage: usage(1),
                judge_model: route("judge", Tier::T5, "high"),
                judge_usage: usage(1),
                verdict: TrialVerdict {
                    score,
                    is_catastrophic: false,
                    failure_mode: None,
                    checks: Vec::new(),
                },
            })
        })
        .collect()
}

fn policy() -> FrontierPolicy {
    FrontierPolicy {
        screening_trials_per_case: 1,
        confirmation_trials_per_case: 3,
        maximum_trials_per_case: 5,
        minimum_trial_score: 7,
        minimum_weighted_pass_basis_points: 8_500,
        minimum_lower_bound_basis_points: 8_000,
        confidence_level_basis_points: 9_500,
        confidence_method: FrontierConfidenceMethod::StratifiedBootstrap,
        confidence_resamples: 2_000,
        maximum_infrastructure_attempts: 2,
        maximum_catalog_age_seconds: 3_600,
        active_pool_size: 5,
        spending_limit_millionths_of_dollar: 1_000_000,
        is_provider_limit_enforced: true,
        is_first_party_only: true,
    }
}

fn route(name: &str, tier: Tier, thinking: &str) -> ModelIdentity {
    ModelIdentity {
        tier,
        provider: "provider".to_string(),
        model: name.to_string(),
        thinking: thinking.to_string(),
    }
}

fn entrant() -> FrontierEntrant {
    FrontierEntrant {
        provider: "provider".to_string(),
        model: "alpha".to_string(),
        entry_tier: Tier::T1,
        thinking_levels: vec!["low".to_string(), "medium".to_string(), "high".to_string()],
        catalog_observed_at: Timestamp("2030-01-01T00:00:00+0000".to_string()),
    }
}

fn initial_progress(entrant: &FrontierEntrant) -> FrontierModelProgress {
    FrontierModelProgress {
        provider: entrant.provider.clone(),
        model: entrant.model.clone(),
        entry_tier: entrant.entry_tier,
        selected_routes: Vec::new(),
        next_tier: Some(entrant.entry_tier),
        next_thinking_index: Some(0),
        is_exhausted: false,
    }
}

fn cell(
    model: ModelIdentity,
    status: FrontierCellStatus,
    weighted_pass_basis_points: u16,
    cost: u64,
) -> FrontierCellEvidence {
    FrontierCellEvidence {
        model,
        status,
        completed_trials: 4,
        expected_trials: 4,
        failed_trials: u32::from(status != FrontierCellStatus::Passed),
        score: Some(FrontierScore {
            weighted_pass_basis_points,
            lower_bound_basis_points: weighted_pass_basis_points.min(8_500),
            critical_passed_trials: 1,
            critical_expected_trials: 1,
            is_group_coverage_complete: true,
        }),
        total_usage: usage(cost),
    }
}

fn report(name: &str, quality: u16, cost: u64) -> FrontierModelReport {
    let route = route(name, Tier::T1, "low");
    let cell = cell(route.clone(), FrontierCellStatus::Passed, quality, cost);
    FrontierModelReport {
        provider: route.provider.clone(),
        model: route.model.clone(),
        cells: vec![cell.clone()],
        highest_passing_tier: Some(Tier::T1),
        selected_routes: vec![route],
        pool_memberships: BTreeMap::new(),
        baseline_change: FrontierBaselineChange::NotCompared,
        total_usage: cell.total_usage,
    }
}

fn usage(value: u64) -> TrialUsage {
    TrialUsage {
        input_tokens: value,
        output_tokens: value,
        cache_read_tokens: value,
        cache_write_tokens: value,
        turns: u32::try_from(value).unwrap_or(u32::MAX),
        tool_calls: u32::try_from(value).unwrap_or(u32::MAX),
        elapsed_milliseconds: value,
        cost_millionths_of_dollar: value,
    }
}
