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

use frontier_scheduler::next_frontier_trial;
use model::{
    ArtifactName, CaseId, FrontierCaseGroup, FrontierCaseReference, FrontierConfidenceMethod,
    FrontierEntrant, FrontierInfrastructureEvent, FrontierModelProgress, FrontierPlan,
    FrontierPolicy, FrontierRunConfiguration, FrontierRunId, FrontierRunState, FrontierRunStatus,
    FrontierScheduleAction, FrontierSuite, FrontierSuiteIdentity, FrontierTierSuite,
    HarnessIdentity, ModelIdentity, PoolPauseReason, SkillEvalError, T1ScreenSnapshotIdentity,
    Tier, Timestamp, TrialKey, TrialRecord, TrialUsage, TrialVerdict,
};

#[test]
fn every_entrant_starts_at_its_entry_tier_and_weakest_level() {
    let entrants = vec![
        entrant("luna", Tier::T1, &["off", "minimal"]),
        entrant("haiku", Tier::T1, &["off"]),
        entrant("sonnet", Tier::T2, &["off", "low"]),
        entrant("terra", Tier::T2, &["off"]),
        entrant("spark", Tier::T3, &["low", "medium"]),
        entrant("opus", Tier::T4, &["off"]),
        entrant("sol", Tier::T4, &["off"]),
        entrant("fable", Tier::T5, &["minimal", "low"]),
    ];
    for expected in &entrants {
        let (plan, suite, state) = fixture(vec![expected.clone()]);
        let (model, key, infrastructure_attempt, reservation) =
            dispatch(next_frontier_trial(&plan, &suite, &state, &[]).unwrap());
        assert_eq!(model.tier, expected.entry_tier);
        assert_eq!(model.thinking, expected.thinking_levels[0]);
        assert_eq!(key.tier, expected.entry_tier);
        assert_eq!(infrastructure_attempt, 1);
        assert_eq!(
            reservation,
            plan.policy.maximum_trial_cost_millionths_of_dollar
        );
        assert_eq!(state.models[0].next_thinking_index, Some(0));
    }
}

#[test]
fn promising_screen_expands_to_three_and_pass_stops_before_attempt_four() {
    let (plan, suite, state) = fixture(vec![entrant("luna", Tier::T1, &["off"])]);
    let model = route("luna", Tier::T1, "off");
    let screening = trials_for(&suite, &model, 0, 1, 10);
    let (_, next, _, _) = dispatch(next_frontier_trial(&plan, &suite, &state, &screening).unwrap());
    assert_eq!(next.attempt, 2);

    let confirmation = trials_for(&suite, &model, 0, 3, 10);
    assert_eq!(
        next_frontier_trial(&plan, &suite, &state, &confirmation).unwrap(),
        FrontierScheduleAction::Complete
    );
}

#[test]
fn uncertain_confirmation_expands_to_five_and_never_schedules_six() {
    let (plan, mut suite, state) = fixture(vec![entrant("luna", Tier::T1, &["off"])]);
    make_uncertain_suite(&mut suite);
    let model = route("luna", Tier::T1, "off");
    let confirmation = uncertain_trials(&suite, &model, 0, 3);
    let (_, next, _, _) =
        dispatch(next_frontier_trial(&plan, &suite, &state, &confirmation).unwrap());
    assert_eq!(next.attempt, 4);

    let maximum = uncertain_trials(&suite, &model, 0, 5);
    assert_eq!(
        next_frontier_trial(&plan, &suite, &state, &maximum).unwrap(),
        FrontierScheduleAction::Complete
    );
}

#[test]
fn failed_screen_advances_without_confirmation() {
    let (plan, suite, state) = fixture(vec![entrant("luna", Tier::T1, &["off", "minimal"])]);
    let failed = trials_for(&suite, &route("luna", Tier::T1, "off"), 0, 1, 0);
    let (_, next, _, _) = dispatch(next_frontier_trial(&plan, &suite, &state, &failed).unwrap());
    assert_eq!(next.tier, Tier::T1);
    assert_eq!(next.attempt, 1);
    assert_eq!(next.route_index, 0);
}

#[test]
fn passing_route_uses_statistics_to_select_stronger_cross_tier_level() {
    let (plan, suite, state) = fixture(vec![entrant("luna", Tier::T1, &["off", "minimal", "low"])]);
    let mut evidence = trials_for(&suite, &route("luna", Tier::T1, "off"), 0, 3, 10);
    let (_, next, _, _) = dispatch(next_frontier_trial(&plan, &suite, &state, &evidence).unwrap());
    assert_eq!(next.tier, Tier::T2);
    evidence.extend(trials_for(
        &suite,
        &route("luna", Tier::T2, "minimal"),
        0,
        3,
        10,
    ));
    let (_, next, _, _) = dispatch(next_frontier_trial(&plan, &suite, &state, &evidence).unwrap());
    assert_eq!(next.tier, Tier::T3);
}

#[test]
fn resume_reuses_terminal_trials_and_selects_the_first_missing_key() {
    let (plan, suite, state) = fixture(vec![entrant("luna", Tier::T1, &["off"])]);
    let model = route("luna", Tier::T1, "off");
    let mut evidence = trials_for(&suite, &model, 0, 1, 10);
    let missing = TrialKey {
        attempt: 2,
        ..evidence[0].key.clone()
    };
    evidence.extend(
        trials_for(&suite, &model, 0, 3, 10)
            .into_iter()
            .filter(|trial| trial.key.attempt > 1 && trial.key != missing),
    );
    let (_, next, _, _) = dispatch(next_frontier_trial(&plan, &suite, &state, &evidence).unwrap());
    assert_eq!(next, missing);
}

#[test]
fn infrastructure_failures_retry_once_then_pause_without_performance_evidence() {
    let (plan, suite, mut state) = fixture(vec![entrant("luna", Tier::T1, &["off"])]);
    let (_, key, first_attempt, reservation) =
        dispatch(next_frontier_trial(&plan, &suite, &state, &[]).unwrap());
    assert_eq!(first_attempt, 1);
    state
        .infrastructure_events
        .push(infrastructure(&key, 1, "first"));
    let (retry_model, retry_key, retry_attempt, retry_reservation) =
        dispatch(next_frontier_trial(&plan, &suite, &state, &[]).unwrap());
    assert_eq!(retry_model, route("luna", key.tier, "off"));
    assert_eq!(retry_key, key);
    assert_eq!(retry_attempt, 2);
    assert_eq!(retry_reservation, reservation);
    state
        .infrastructure_events
        .push(infrastructure(&key, 2, "second"));
    assert_eq!(
        next_frontier_trial(&plan, &suite, &state, &[]).unwrap(),
        FrontierScheduleAction::Pause {
            reason: PoolPauseReason::Infrastructure {
                message: "second".to_string(),
            },
        }
    );
}

#[test]
fn projected_spend_equal_to_the_limit_dispatches() {
    let (plan, suite, mut state) = fixture(vec![entrant("luna", Tier::T1, &["off"])]);
    state.spent_millionths_of_dollar = plan.policy.spending_limit_millionths_of_dollar
        - plan.policy.maximum_trial_cost_millionths_of_dollar;
    let (_, _, _, reservation) = dispatch(next_frontier_trial(&plan, &suite, &state, &[]).unwrap());
    assert_eq!(
        reservation,
        plan.policy.maximum_trial_cost_millionths_of_dollar
    );
}

#[test]
fn projected_spend_over_the_limit_returns_a_typed_pause() {
    let (plan, suite, mut state) = fixture(vec![entrant("luna", Tier::T1, &["off"])]);
    state.spent_millionths_of_dollar = plan.policy.spending_limit_millionths_of_dollar
        - plan.policy.maximum_trial_cost_millionths_of_dollar
        + 1;
    assert_eq!(
        next_frontier_trial(&plan, &suite, &state, &[]).unwrap(),
        FrontierScheduleAction::Pause {
            reason: PoolPauseReason::SpendingLimit {
                spent_millionths_of_dollar: state.spent_millionths_of_dollar,
                limit_millionths_of_dollar: plan.policy.spending_limit_millionths_of_dollar,
            },
        }
    );
}

#[test]
fn persisted_pause_returns_the_same_typed_pause() {
    let (plan, suite, mut state) = fixture(vec![entrant("luna", Tier::T1, &["off"])]);
    let reason = PoolPauseReason::Quota {
        model: route("luna", Tier::T1, "off"),
        reset_at: Some(timestamp()),
    };
    state.status = FrontierRunStatus::Paused;
    state.pause = Some(reason.clone());
    assert_eq!(
        next_frontier_trial(&plan, &suite, &state, &[]).unwrap(),
        FrontierScheduleAction::Pause { reason }
    );
}

#[test]
fn each_terminal_status_returns_its_exact_status() {
    let (plan, suite, state) = fixture(vec![entrant("luna", Tier::T1, &["off"])]);
    for status in [
        FrontierRunStatus::AwaitingDecision,
        FrontierRunStatus::Accepted,
        FrontierRunStatus::Rejected,
        FrontierRunStatus::Failed,
    ] {
        let mut candidate = state.clone();
        candidate.status = status;
        assert_eq!(
            next_frontier_trial(&plan, &suite, &candidate, &[]).unwrap(),
            FrontierScheduleAction::Terminal { status }
        );
    }
}

#[test]
fn selection_is_stable_under_entrant_case_and_evidence_reordering() {
    let entrants = vec![
        entrant("zeta", Tier::T1, &["off"]),
        entrant("alpha", Tier::T1, &["off"]),
    ];
    let (plan, suite, state) = fixture(entrants.clone());
    let first = next_frontier_trial(&plan, &suite, &state, &[]).unwrap();
    let (mut reordered_plan, mut reordered_suite, mut reordered_state) = fixture(entrants);
    reordered_plan.entrants.reverse();
    reordered_state.configuration.plan.entrants.reverse();
    reordered_state.models.reverse();
    for tier_suite in reordered_suite.tiers.values_mut() {
        tier_suite.cases.reverse();
    }
    let second =
        next_frontier_trial(&reordered_plan, &reordered_suite, &reordered_state, &[]).unwrap();
    assert_eq!(first, second);

    let (plan, suite, state) = fixture(vec![entrant("alpha", Tier::T1, &["off"])]);
    let mut evidence = trials_for(&suite, &route("alpha", Tier::T1, "off"), 0, 1, 10);
    let ordered = next_frontier_trial(&plan, &suite, &state, &evidence).unwrap();
    evidence.reverse();
    let reversed = next_frontier_trial(&plan, &suite, &state, &evidence).unwrap();
    assert_eq!(ordered, reversed);
}

#[test]
fn dispatch_contains_the_exact_model_key_attempt_and_reservation() {
    let (plan, suite, state) = fixture(vec![entrant("luna", Tier::T1, &["off"])]);
    assert_eq!(
        next_frontier_trial(&plan, &suite, &state, &[]).unwrap(),
        FrontierScheduleAction::Dispatch {
            model: route("luna", Tier::T1, "off"),
            key: TrialKey {
                artifact: ArtifactName("artifact".to_string()),
                tier: Tier::T1,
                route_index: 0,
                case: CaseId("adversarial".to_string()),
                attempt: 1,
            },
            infrastructure_attempt: 1,
            reserved_cost_millionths_of_dollar: 10,
        }
    );
}

#[test]
fn pending_state_computes_the_same_dispatch_as_running() {
    let (plan, suite, mut state) = fixture(vec![entrant("luna", Tier::T1, &["off"])]);
    let running = next_frontier_trial(&plan, &suite, &state, &[]).unwrap();
    state.status = FrontierRunStatus::Pending;
    let pending = next_frontier_trial(&plan, &suite, &state, &[]).unwrap();
    assert_eq!(pending, running);
}

#[test]
fn zero_spending_limit_returns_a_typed_pause() {
    let (mut plan, suite, mut state) = fixture(vec![entrant("luna", Tier::T1, &["off"])]);
    plan.policy.spending_limit_millionths_of_dollar = 0;
    state.configuration.plan = plan.clone();
    assert_eq!(
        next_frontier_trial(&plan, &suite, &state, &[]).unwrap(),
        FrontierScheduleAction::Pause {
            reason: PoolPauseReason::SpendingLimit {
                spent_millionths_of_dollar: 0,
                limit_millionths_of_dollar: 0,
            },
        }
    );
}

#[test]
fn zero_trial_cost_reservation_is_rejected() {
    let (mut plan, suite, mut state) = fixture(vec![entrant("luna", Tier::T1, &["off"])]);
    plan.policy.maximum_trial_cost_millionths_of_dollar = 0;
    state.configuration.plan = plan.clone();
    assert_eq!(
        next_frontier_trial(&plan, &suite, &state, &[]),
        Err(SkillEvalError::InvalidConfiguration(
            "frontier trial cost reservation is zero".to_string()
        ))
    );
}

#[test]
fn projected_spend_overflow_is_rejected() {
    let (mut plan, suite, mut state) = fixture(vec![entrant("luna", Tier::T1, &["off"])]);
    plan.policy.spending_limit_millionths_of_dollar = u64::MAX;
    state.configuration.plan = plan.clone();
    state.spent_millionths_of_dollar = u64::MAX - 5;
    assert_eq!(
        next_frontier_trial(&plan, &suite, &state, &[]),
        Err(SkillEvalError::InvalidConfiguration(
            "frontier projected spend overflow".to_string()
        ))
    );
}

#[test]
fn exhausted_entrants_return_complete() {
    let (plan, suite, state) = fixture(Vec::new());
    assert_eq!(
        next_frontier_trial(&plan, &suite, &state, &[]).unwrap(),
        FrontierScheduleAction::Complete
    );
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
            sha256: "suite".to_string(),
            version: 1,
        },
        capabilities: T1ScreenSnapshotIdentity {
            path: PathBuf::from("capabilities.json"),
            sha256: "capabilities".to_string(),
            version: 1,
            observed_at_unix_seconds: 1,
            pi_version: "1".to_string(),
        },
        entrants: entrants.clone(),
        judge: route("judge", Tier::T5, "high"),
        policy: policy(),
    };
    let configuration = FrontierRunConfiguration {
        run_id: FrontierRunId("run".to_string()),
        created_at: timestamp(),
        plan_path: PathBuf::from("plan.json"),
        plan_sha256: "plan".to_string(),
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

fn dispatch(action: FrontierScheduleAction) -> (ModelIdentity, TrialKey, u8, u64) {
    match action {
        FrontierScheduleAction::Dispatch {
            model,
            key,
            infrastructure_attempt,
            reserved_cost_millionths_of_dollar,
        } => (
            model,
            key,
            infrastructure_attempt,
            reserved_cost_millionths_of_dollar,
        ),
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

fn make_uncertain_suite(suite: &mut FrontierSuite) {
    for tier_suite in suite.tiers.values_mut() {
        tier_suite.group_weights_basis_points = BTreeMap::from([
            (FrontierCaseGroup::Normal, 9_700),
            (FrontierCaseGroup::Edge, 100),
            (FrontierCaseGroup::Adversarial, 100),
            (FrontierCaseGroup::Critical, 100),
        ]);
        for index in 1..10 {
            tier_suite
                .cases
                .push(case(&format!("normal-{index}"), FrontierCaseGroup::Normal));
        }
    }
}

fn case(name: &str, group: FrontierCaseGroup) -> FrontierCaseReference {
    FrontierCaseReference {
        artifact_path: PathBuf::from("artifact"),
        artifact_revision: "revision".to_string(),
        case: CaseId(name.to_string()),
        group,
        is_confirmation: group == FrontierCaseGroup::Critical,
    }
}

fn entrant(name: &str, tier: Tier, levels: &[&str]) -> FrontierEntrant {
    FrontierEntrant {
        provider: "anthropic".to_string(),
        model: name.to_string(),
        entry_tier: tier,
        thinking_levels: levels.iter().map(|level| (*level).to_string()).collect(),
        catalog_observed_at: timestamp(),
    }
}

fn route(name: &str, tier: Tier, thinking: &str) -> ModelIdentity {
    ModelIdentity {
        provider: "anthropic".to_string(),
        model: name.to_string(),
        tier,
        thinking: thinking.to_string(),
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

fn uncertain_trials(
    suite: &FrontierSuite,
    model: &ModelIdentity,
    route_index: u16,
    attempts: u16,
) -> Vec<TrialRecord> {
    let mut trials = trials_for(suite, model, route_index, attempts, 10);
    for trial in &mut trials {
        if trial.key.case.0 == "normal-1" {
            trial.verdict.score = 0;
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
            artifact: ArtifactName("artifact".to_string()),
            tier: model.tier,
            route_index,
            case: case.case.clone(),
            attempt,
        },
        model: model.clone(),
        harness: HarnessIdentity {
            runner_version: "runner".to_string(),
            pi_version: "pi".to_string(),
            artifact_revision: case.artifact_revision.clone(),
            tool_policy_digest: "policy".to_string(),
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

fn infrastructure(key: &TrialKey, attempt: u8, message: &str) -> FrontierInfrastructureEvent {
    FrontierInfrastructureEvent {
        model: route("luna", key.tier, "off"),
        artifact: key.artifact.clone(),
        case: key.case.clone(),
        attempt: key.attempt,
        infrastructure_attempt: attempt,
        message: message.to_string(),
        occurred_at: timestamp(),
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
    Timestamp("2026-08-27T00:00:00-0400".to_string())
}
