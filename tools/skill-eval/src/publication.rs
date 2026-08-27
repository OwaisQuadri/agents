use std::collections::BTreeSet;

use crate::model::{
    ArtifactChange, ArtifactKind, ArtifactReport, ArtifactStatus, Decision, EvidenceRole,
    PublicationGate, PublicationStatus, QualificationReport, RunStatus, SkillEvalError,
    TierDestination, TierStatus,
};

pub(crate) fn evaluate_publication_gate(
    change: &ArtifactChange,
    report: &QualificationReport,
) -> Result<PublicationGate, SkillEvalError> {
    if change.own_eval.artifact_revision != change.candidate_revision {
        return Ok(blocked(change, "own-eval evidence is stale"));
    }
    if report.change.as_ref() != Some(change) {
        return Ok(blocked(change, "qualification report is stale"));
    }

    let matching = report
        .artifacts
        .iter()
        .filter(|artifact| artifact.artifact == change.artifact)
        .collect::<Vec<_>>();
    let [artifact] = matching.as_slice() else {
        return Ok(blocked(
            change,
            "qualification report must contain exactly one matching artifact",
        ));
    };
    if artifact.kind != change.kind {
        return Ok(blocked(
            change,
            "qualification artifact kind does not match",
        ));
    }
    if is_blocked(report, artifact) {
        return Ok(blocked(
            change,
            "qualification or owner decision blocks publication",
        ));
    }

    let Some(boundary) = artifact.boundary.as_ref() else {
        return Ok(waiting(
            change,
            PublicationStatus::AwaitingQualification,
            "supported qualification evidence is not available",
        ));
    };
    let accepted_matches = artifact
        .tiers
        .iter()
        .filter(|evidence| *evidence == &boundary.accepted)
        .count();
    let is_failing_supported = boundary.failing.as_ref().is_none_or(|failing| {
        failing.role == EvidenceRole::Candidate
            && failing.status == TierStatus::Failed
            && artifact
                .tiers
                .iter()
                .filter(|evidence| *evidence == failing)
                .count()
                == 1
    });
    let is_current_evidence = artifact
        .reference
        .iter()
        .chain(&artifact.tiers)
        .all(|evidence| {
            !evidence.harnesses.is_empty()
                && evidence
                    .harnesses
                    .iter()
                    .all(|harness| harness.artifact_revision == change.candidate_revision)
        });
    let is_supported = boundary.accepted.role == EvidenceRole::Candidate
        && boundary.accepted.status == TierStatus::Accepted
        && accepted_matches == 1
        && is_failing_supported
        && is_current_evidence;
    if !is_supported {
        return Ok(blocked(
            change,
            "qualification boundary does not match candidate artifact evidence",
        ));
    }

    let Some(decision) = artifact.decision.as_ref() else {
        return Ok(waiting(
            change,
            PublicationStatus::AwaitingDecision,
            "supported evidence awaits an owner decision",
        ));
    };
    if decision.artifact != change.artifact || decision.decision != Decision::Accepted {
        return Ok(blocked(change, "owner decision blocks publication"));
    }
    if artifact.status != ArtifactStatus::Accepted || report.status != RunStatus::Completed {
        return Ok(blocked(
            change,
            "accepted decision does not match qualification state",
        ));
    }

    if !is_valid_required_destinations(artifact) {
        return Ok(blocked(
            change,
            "qualification report has invalid required destinations",
        ));
    }

    let required = artifact
        .required_destinations
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let assigned = decision
        .assignments
        .iter()
        .map(|assignment| assignment.destination.clone())
        .collect::<BTreeSet<_>>();
    let is_exact_assignment = required.len() == artifact.required_destinations.len()
        && assigned.len() == decision.assignments.len()
        && assigned == required
        && decision
            .assignments
            .iter()
            .all(|assignment| assignment.tier == boundary.accepted.tier);
    if !is_exact_assignment {
        return Ok(blocked(
            change,
            "accepted assignments do not match required destinations and boundary tier",
        ));
    }

    Ok(PublicationGate {
        change: change.clone(),
        status: PublicationStatus::Ready,
        assignments: decision.assignments.clone(),
        reason: None,
    })
}

fn is_valid_required_destinations(artifact: &ArtifactReport) -> bool {
    let destinations = &artifact.required_destinations;
    let unique = destinations.iter().collect::<BTreeSet<_>>();
    if unique.len() != destinations.len() {
        return false;
    }

    match artifact.kind {
        ArtifactKind::Skill => {
            destinations
                .iter()
                .filter(|destination| **destination == TierDestination::SkillMinimum)
                .count()
                == 1
                && destinations
                    .iter()
                    .filter(|destination| **destination == TierDestination::SkillTarget)
                    .count()
                    <= 1
                && destinations.iter().all(|destination| {
                    matches!(
                        destination,
                        TierDestination::SkillMinimum | TierDestination::SkillTarget
                    )
                })
        }
        ArtifactKind::Agent => destinations.as_slice() == [TierDestination::Agent],
        ArtifactKind::Workflow => {
            destinations
                .iter()
                .filter(|destination| **destination == TierDestination::WorkflowOrchestrator)
                .count()
                == 1
                && destinations.iter().any(|destination| {
                    matches!(
                        destination,
                        TierDestination::WorkflowNode { node } if !node.is_empty()
                    )
                })
                && destinations.iter().all(|destination| {
                    matches!(destination, TierDestination::WorkflowOrchestrator)
                        || matches!(
                            destination,
                            TierDestination::WorkflowNode { node } if !node.is_empty()
                        )
                })
        }
    }
}

fn is_blocked(report: &QualificationReport, artifact: &ArtifactReport) -> bool {
    matches!(report.status, RunStatus::Paused | RunStatus::Failed)
        || matches!(
            artifact.status,
            ArtifactStatus::Rejected | ArtifactStatus::Paused | ArtifactStatus::NeedsReview
        )
        || artifact
            .decision
            .as_ref()
            .is_some_and(|decision| decision.decision == Decision::Rejected)
        || artifact.boundary.is_none()
            && artifact.tiers.iter().any(|evidence| {
                matches!(
                    evidence.status,
                    TierStatus::Failed | TierStatus::Paused | TierStatus::NeedsReview
                )
            })
}

fn waiting(change: &ArtifactChange, status: PublicationStatus, reason: &str) -> PublicationGate {
    PublicationGate {
        change: change.clone(),
        status,
        assignments: Vec::new(),
        reason: Some(reason.to_owned()),
    }
}

fn blocked(change: &ArtifactChange, reason: &str) -> PublicationGate {
    waiting(change, PublicationStatus::Blocked, reason)
}
