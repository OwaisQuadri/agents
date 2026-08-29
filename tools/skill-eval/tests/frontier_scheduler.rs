#![expect(
    dead_code,
    reason = "the test imports private production modules to exercise the crate-private scheduler"
)]
#![expect(
    clippy::large_enum_variant,
    reason = "the test imports frozen production model declarations without changing their shapes"
)]

#[path = "../src/frontier_scheduler.rs"]
mod frontier_scheduler;
#[path = "../src/model.rs"]
mod model;
#[path = "../src/statistics.rs"]
mod statistics;

use std::collections::BTreeMap;
use std::path::PathBuf;

use frontier_scheduler::next_frontier_wave;
use model::{
    ArtifactName, CaseId, FrontierCaseGroup, FrontierCaseReference, FrontierCellEvidence,
    FrontierCellStatus, FrontierConfidenceMethod, FrontierEntrant, FrontierInfrastructureEvent,
    FrontierModelProgress, FrontierPlan, FrontierPolicy, FrontierRunConfiguration, FrontierRunId,
    FrontierRunState, FrontierRunStatus, FrontierScheduleAction, FrontierScheduledTrial,
    FrontierSuite, FrontierSuiteIdentity, FrontierTierSuite, HarnessIdentity, ModelIdentity,
    PoolPauseReason, SkillEvalError, T1ScreenSnapshotIdentity, Tier, Timestamp, TrialKey,
    TrialRecord, TrialUsage, TrialVerdict,
};

#[test]
fn wave_is_bounded_and_balances_ready_rows() {
    let entrants = vec![
        entrant("zeta", Tier::T2, &["off"]),
        entrant("alpha", Tier::T1, &["off"]),
    ];
    let (plan, suite, state) = fixture(entrants);

    let (trials, reservation) = dispatch(next_frontier_wave(&plan, &suite, &state, &[]).unwrap());

    assert_eq!(trials.len(), 6);
    assert_eq!(reservation, 10);
    assert_eq!(
        trials
            .iter()
            .filter(|trial| trial.model.model == "alpha")
            .count(),
        3
    );
    assert_eq!(
        trials
            .iter()
            .filter(|trial| trial.model.model == "zeta")
            .count(),
        3
    );
    assert!(trials.iter().all(|trial| trial.key.attempt == 1));
}

#[test]
fn pass_climbs_tier_at_the_same_thinking_and_failure_moves_right() {
    let (plan, suite, state) = fixture(vec![entrant("luna", Tier::T1, &["off", "minimal"])]);
    let mut evidence = trials_for(&suite, &route("luna", Tier::T1, "off"), 0, 3, 10);
    let (wave, _) = dispatch(next_frontier_wave(&plan, &suite, &state, &evidence).unwrap());
    assert!(wave.iter().all(|trial| {
        trial.model.tier == Tier::T2 && trial.model.thinking == "off" && trial.key.attempt == 1
    }));

    evidence.extend(trials_for(&suite, &route("luna", Tier::T2, "off"), 0, 1, 0));
    let (wave, _) = dispatch(next_frontier_wave(&plan, &suite, &state, &evidence).unwrap());
    assert!(wave.iter().all(|trial| {
        trial.model.tier == Tier::T2 && trial.model.thinking == "minimal" && trial.key.attempt == 1
    }));
}

#[test]
fn tier_five_pass_and_final_thinking_failure_stop_the_row() {
    let (plan, suite, state) = fixture(vec![entrant("pass", Tier::T5, &["off", "minimal"])]);
    let passed = trials_for(&suite, &route("pass", Tier::T5, "off"), 0, 3, 10);
    assert_eq!(
        next_frontier_wave(&plan, &suite, &state, &passed).unwrap(),
        FrontierScheduleAction::Complete
    );

    let (plan, suite, state) = fixture(vec![entrant("fail", Tier::T5, &["off", "minimal"])]);
    let mut failed = trials_for(&suite, &route("fail", Tier::T5, "off"), 0, 1, 0);
    failed.extend(trials_for(
        &suite,
        &route("fail", Tier::T5, "minimal"),
        0,
        1,
        0,
    ));
    assert_eq!(
        next_frontier_wave(&plan, &suite, &state, &failed).unwrap(),
        FrontierScheduleAction::Complete
    );
}

#[test]
fn attempts_are_barriers_and_partial_resume_emits_only_missing_cases() {
    let (plan, suite, state) = fixture(vec![entrant("luna", Tier::T1, &["off"])]);
    let model = route("luna", Tier::T1, "off");
    let mut evidence = trials_for(&suite, &model, 0, 1, 10);
    let (attempt_two, _) = dispatch(next_frontier_wave(&plan, &suite, &state, &evidence).unwrap());
    assert_eq!(attempt_two.len(), 4);
    assert!(attempt_two.iter().all(|trial| trial.key.attempt == 2));

    let partial = trial(&suite.tiers[&Tier::T1].cases[0], &model, 0, 2, 10);
    evidence.push(partial.clone());
    let (remaining, _) = dispatch(next_frontier_wave(&plan, &suite, &state, &evidence).unwrap());
    assert_eq!(remaining.len(), 3);
    assert!(remaining.iter().all(|trial| trial.key.attempt == 2));
    assert!(remaining.iter().all(|trial| trial.key != partial.key));

    evidence.push(trial(&suite.tiers[&Tier::T1].cases[1], &model, 0, 3, 10));
    assert!(next_frontier_wave(&plan, &suite, &state, &evidence).is_err());
}

#[test]
fn rows_can_emit_different_current_attempts_without_crossing_their_own_barriers() {
    let entrants = vec![
        entrant("alpha", Tier::T1, &["off"]),
        entrant("zeta", Tier::T1, &["off"]),
    ];
    let (plan, suite, state) = fixture(entrants);
    let evidence = trials_for(&suite, &route("alpha", Tier::T1, "off"), 0, 1, 10);

    let (wave, _) = dispatch(next_frontier_wave(&plan, &suite, &state, &evidence).unwrap());

    assert_eq!(wave.len(), 6);
    assert_eq!(
        wave.iter()
            .filter(|trial| trial.model.model == "alpha")
            .count(),
        3
    );
    assert_eq!(
        wave.iter()
            .filter(|trial| trial.model.model == "zeta")
            .count(),
        3
    );
    assert!(
        wave.iter()
            .filter(|trial| trial.model.model == "alpha")
            .all(|trial| trial.key.attempt == 2)
    );
    assert!(
        wave.iter()
            .filter(|trial| trial.model.model == "zeta")
            .all(|trial| trial.key.attempt == 1)
    );
}

#[test]
fn quota_skipped_row_consumes_partial_trials_and_other_rows_continue() {
    let entrants = vec![
        entrant("alpha", Tier::T1, &["off"]),
        entrant("zeta", Tier::T1, &["off"]),
    ];
    let (plan, suite, mut state) = fixture(entrants);
    let alpha = route("alpha", Tier::T1, "off");
    state.cells.push(FrontierCellEvidence {
        model: alpha.clone(),
        status: FrontierCellStatus::Skipped,
        set_aside_reason: None,
        completed_trials: 0,
        expected_trials: 0,
        failed_trials: 0,
        score: None,
        total_usage: zero_usage(),
    });
    let partial = vec![trial(&suite.tiers[&Tier::T1].cases[0], &alpha, 0, 1, 10)];

    let (wave, _) = dispatch(next_frontier_wave(&plan, &suite, &state, &partial).unwrap());

    assert_eq!(wave.len(), 4);
    assert!(wave.iter().all(|trial| trial.model.model == "zeta"));
}

#[test]
fn infrastructure_retries_are_per_key_and_bounded() {
    let (plan, suite, mut state) = fixture(vec![entrant("luna", Tier::T1, &["off"])]);
    let model = route("luna", Tier::T1, "off");
    let mut evidence = trials_for(&suite, &model, 0, 1, 10);
    let missing = evidence.remove(0);
    state
        .infrastructure_events
        .push(infrastructure(&model, &missing.key, 1, "first"));

    let (wave, _) = dispatch(next_frontier_wave(&plan, &suite, &state, &evidence).unwrap());
    assert_eq!(wave.len(), 1);
    assert_eq!(wave[0].key, missing.key);
    assert_eq!(wave[0].infrastructure_attempt, 2);

    state
        .infrastructure_events
        .push(infrastructure(&model, &missing.key, 2, "second"));
    assert_eq!(
        next_frontier_wave(&plan, &suite, &state, &evidence).unwrap(),
        FrontierScheduleAction::Pause {
            reason: PoolPauseReason::Infrastructure {
                message: "second".to_owned(),
            },
        }
    );
}

#[test]
fn stable_order_survives_entrant_case_and_evidence_reordering() {
    let entrants = vec![
        entrant("zeta", Tier::T1, &["off"]),
        entrant("alpha", Tier::T1, &["off"]),
    ];
    let (plan, suite, state) = fixture(entrants.clone());
    let first = next_frontier_wave(&plan, &suite, &state, &[]).unwrap();
    let (mut plan, mut suite, mut state) = fixture(entrants);
    plan.entrants.reverse();
    state.configuration.plan.entrants.reverse();
    state.models.reverse();
    for tier_suite in suite.tiers.values_mut() {
        tier_suite.cases.reverse();
    }
    assert_eq!(
        first,
        next_frontier_wave(&plan, &suite, &state, &[]).unwrap()
    );
}

#[test]
fn whole_wave_equal_to_limit_dispatches_and_one_more_pauses() {
    let (plan, suite, mut state) = fixture(vec![entrant("luna", Tier::T1, &["off"])]);
    state.spent_millionths_of_dollar = 60;
    let (wave, reservation) = dispatch(next_frontier_wave(&plan, &suite, &state, &[]).unwrap());
    assert_eq!(
        wave.len() as u64 * reservation + state.spent_millionths_of_dollar,
        100
    );

    state.spent_millionths_of_dollar = 61;
    assert_eq!(
        next_frontier_wave(&plan, &suite, &state, &[]).unwrap(),
        FrontierScheduleAction::Pause {
            reason: PoolPauseReason::SpendingLimit {
                spent_millionths_of_dollar: 61,
                limit_millionths_of_dollar: 100,
            },
        }
    );
}

#[test]
fn whole_wave_reservation_and_projected_spend_overflow_are_rejected() {
    let (mut plan, suite, mut state) = fixture(vec![entrant("luna", Tier::T1, &["off"])]);
    plan.policy.maximum_trial_cost_millionths_of_dollar = u64::MAX / 2 + 1;
    plan.policy.spending_limit_millionths_of_dollar = u64::MAX;
    state.configuration.plan = plan.clone();
    assert_eq!(
        next_frontier_wave(&plan, &suite, &state, &[]),
        Err(SkillEvalError::InvalidConfiguration(
            "frontier wave reservation overflow".to_owned()
        ))
    );

    plan.policy.maximum_trial_cost_millionths_of_dollar = 10;
    state.configuration.plan = plan.clone();
    state.spent_millionths_of_dollar = u64::MAX - 20;
    assert_eq!(
        next_frontier_wave(&plan, &suite, &state, &[]),
        Err(SkillEvalError::InvalidConfiguration(
            "frontier projected spend overflow".to_owned()
        ))
    );
}

#[test]
fn persisted_pause_and_terminal_statuses_do_not_dispatch() {
    let (plan, suite, mut state) = fixture(vec![entrant("luna", Tier::T1, &["off"])]);
    let reason = PoolPauseReason::Quota {
        model: route("luna", Tier::T1, "off"),
        reset_at: Some(timestamp()),
    };
    state.status = FrontierRunStatus::Paused;
    state.pause = Some(reason.clone());
    assert_eq!(
        next_frontier_wave(&plan, &suite, &state, &[]).unwrap(),
        FrontierScheduleAction::Pause { reason }
    );
    for status in [
        FrontierRunStatus::AwaitingDecision,
        FrontierRunStatus::Accepted,
        FrontierRunStatus::Rejected,
        FrontierRunStatus::Failed,
    ] {
        state.status = status;
        state.pause = None;
        assert_eq!(
            next_frontier_wave(&plan, &suite, &state, &[]).unwrap(),
            FrontierScheduleAction::Terminal { status }
        );
    }
}

fn fixture(entrants: Vec<FrontierEntrant>) -> (FrontierPlan, FrontierSuite, FrontierRunState) {
    let suite = FrontierSuite {
        version: 1,
        tiers: [Tier::T1, Tier::T2, Tier::T3, Tier::T4, Tier::T5]
            .into_iter()
            .map(|tier| (tier, tier_suite()))
            .collect(),
    };
    let plan = FrontierPlan {
        version: 1,
        suite: FrontierSuiteIdentity {
            path: PathBuf::from("suite.json"),
            sha256: "suite".to_owned(),
            version: 1,
        },
        capabilities: T1ScreenSnapshotIdentity {
            path: PathBuf::from("capabilities.json"),
            sha256: "capabilities".to_owned(),
            version: 1,
            observed_at_unix_seconds: 1,
            pi_version: "1".to_owned(),
        },
        entrants: entrants.clone(),
        judge: route("judge", Tier::T5, "high"),
        policy: policy(),
    };
    let configuration = FrontierRunConfiguration {
        run_id: FrontierRunId("run".to_owned()),
        created_at: timestamp(),
        plan_path: PathBuf::from("plan.json"),
        plan_sha256: "plan".to_owned(),
        plan: plan.clone(),
    };
    let models = entrants
        .iter()
        .map(|entrant| FrontierModelProgress {
            provider: entrant.provider.clone(),
            model: entrant.model.clone(),
            entry_tier: entrant.entry_tier,
            selected_routes: Vec::new(),
            next_tier: Some(entrant.entry_tier),
            next_thinking_index: Some(0),
            is_exhausted: false,
        })
        .collect();
    let state = FrontierRunState {
        configuration,
        status: FrontierRunStatus::Running,
        models,
        cells: Vec::new(),
        infrastructure_events: Vec::new(),
        pause: None,
        decision: None,
        spent_millionths_of_dollar: 0,
    };
    (plan, suite, state)
}

fn dispatch(action: FrontierScheduleAction) -> (Vec<FrontierScheduledTrial>, u64) {
    match action {
        FrontierScheduleAction::Dispatch {
            trials,
            reserved_cost_per_trial_millionths_of_dollar,
        } => (trials, reserved_cost_per_trial_millionths_of_dollar),
        other => panic!("expected dispatch, got {other:?}"),
    }
}

fn tier_suite() -> FrontierTierSuite {
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
        artifact_path: PathBuf::from("artifact"),
        artifact_revision: "revision".to_owned(),
        case: CaseId(name.to_owned()),
        group,
        is_confirmation: group == FrontierCaseGroup::Critical,
    }
}

fn entrant(name: &str, tier: Tier, levels: &[&str]) -> FrontierEntrant {
    FrontierEntrant {
        provider: "anthropic".to_owned(),
        model: name.to_owned(),
        entry_tier: tier,
        thinking_levels: levels.iter().map(|level| (*level).to_owned()).collect(),
        catalog_observed_at: timestamp(),
    }
}

fn route(name: &str, tier: Tier, thinking: &str) -> ModelIdentity {
    ModelIdentity {
        provider: "anthropic".to_owned(),
        model: name.to_owned(),
        tier,
        thinking: thinking.to_owned(),
    }
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
        maximum_trial_cost_millionths_of_dollar: 10,
        spending_limit_millionths_of_dollar: 100,
        is_provider_limit_enforced: true,
        is_first_party_only: true,
    }
}

fn trials_for(
    suite: &FrontierSuite,
    model: &ModelIdentity,
    route_index: u16,
    attempts: u16,
    score: u8,
) -> Vec<TrialRecord> {
    let mut trials = Vec::new();
    for attempt in 1..=attempts {
        for case in &suite.tiers[&model.tier].cases {
            trials.push(trial(case, model, route_index, attempt, score));
        }
    }
    trials
}

fn trial(
    case: &FrontierCaseReference,
    model: &ModelIdentity,
    route_index: u16,
    attempt: u16,
    score: u8,
) -> TrialRecord {
    TrialRecord {
        key: TrialKey {
            artifact: ArtifactName("artifact".to_owned()),
            tier: model.tier,
            route_index,
            case: case.case.clone(),
            attempt,
        },
        model: model.clone(),
        harness: HarnessIdentity {
            runner_version: "runner".to_owned(),
            pi_version: "pi".to_owned(),
            artifact_revision: case.artifact_revision.clone(),
            tool_policy_digest: "policy".to_owned(),
        },
        artifact_path: PathBuf::from("artifact.out"),
        transcript_path: PathBuf::from("transcript.jsonl"),
        candidate_usage: usage(),
        judge_model: route("judge", Tier::T5, "high"),
        judge_usage: usage(),
        verdict: TrialVerdict {
            score,
            is_catastrophic: false,
            failure_mode: None,
            checks: Vec::new(),
        },
    }
}

fn infrastructure(
    model: &ModelIdentity,
    key: &TrialKey,
    attempt: u8,
    message: &str,
) -> FrontierInfrastructureEvent {
    FrontierInfrastructureEvent {
        model: model.clone(),
        artifact: key.artifact.clone(),
        case: key.case.clone(),
        attempt: key.attempt,
        infrastructure_attempt: attempt,
        failure_stage: None,
        charged_millionths_of_dollar: 0,
        message: message.to_owned(),
        occurred_at: timestamp(),
    }
}

fn zero_usage() -> TrialUsage {
    TrialUsage {
        input_tokens: 0,
        output_tokens: 0,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
        turns: 0,
        tool_calls: 0,
        elapsed_milliseconds: 0,
        cost_millionths_of_dollar: 0,
    }
}

fn usage() -> TrialUsage {
    TrialUsage {
        input_tokens: 1,
        output_tokens: 1,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
        turns: 1,
        tool_calls: 0,
        elapsed_milliseconds: 1,
        cost_millionths_of_dollar: 1,
    }
}

fn timestamp() -> Timestamp {
    Timestamp("2026-08-27T00:00:00-0400".to_owned())
}
