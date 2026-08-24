use std::cell::RefCell;
use std::collections::BTreeSet;
use std::env;
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::judge::PiJudge;
use crate::model::{
    ArtifactChange, ArtifactDefinition, ArtifactName, AuditBriefRequest, CaseId, CliCommand,
    CliRequest, Decision, ExecutionDefinition, HarnessIdentity, ModelIdentity, OutputFormat,
    OwnEvalEvidence, PromptJudgeRequest, QualificationPolicy, QualificationPurpose,
    QualificationReport, QualifyRequest, RunEvent, RunId, SkillEvalError, Tier, TierAssignment,
    TierDestination, Timestamp, TrialRecord, TrialSelector,
};
use crate::models::ConfiguredModelResolver;
use crate::pi_runner::PiCandidateRunner;
use crate::ports::{
    ArtifactSource, CandidateRunner, Clock, HarnessResolver, Judge, ModelResolver, ProgressSink,
    QualificationRuntime, RunIdSource, RunStore, TierWriter, Verifier,
};
use crate::service::{
    apply_tier_assignments, build_report, evaluate_publication_gate, inspect_trial, judge_prompt,
    prepare_audit_briefs, record_decision, resume_qualification, start_qualification,
};
use crate::source::FileArtifactSource;
use crate::store::FileRunStore;
use crate::tier_writer::FileTierWriter;
use crate::verifier::FileVerifier;

const DEFAULT_RUNS_ROOT: &str = ".map/skill-eval/runs";
const DEFAULT_REPEATS: u16 = 3;
const DEFAULT_SCORE: u8 = 8;
const DEFAULT_MARGIN: f64 = 1.0;
const DEFAULT_CONFIDENCE: f64 = 0.95;
const DEFAULT_JUDGE_TIMEOUT: u32 = 120;
const RUNNER_VERSION: &str = env!("CARGO_PKG_VERSION");

thread_local! {
    static RUN_ID_FILE: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

// TODO(AGNT-0032.T91): Parse and render pool start, resume, and report commands.
pub(crate) fn parse_arguments(arguments: &[OsString]) -> Result<CliRequest, SkillEvalError> {
    RUN_ID_FILE.with(|slot| *slot.borrow_mut() = None);
    let values = arguments
        .iter()
        .map(|value| {
            value.clone().into_string().map_err(|_| {
                SkillEvalError::InvalidArguments("arguments must be valid UTF-8".to_owned())
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let (command, tail) = values
        .split_first()
        .ok_or_else(|| invalid("missing command"))?;
    let mut parser = ArgumentParser::new(tail);
    let request = match command.as_str() {
        "qualify" => parse_qualify(&mut parser)?,
        "report" => CliCommand::Report {
            run_id: parse_run_only(&mut parser)?,
        },
        "inspect" => parse_inspect(&mut parser)?,
        "resume" => CliCommand::Resume {
            run_id: parse_run_only(&mut parser)?,
        },
        "decide" => parse_decide(&mut parser)?,
        "apply" => parse_apply(&mut parser)?,
        "audit-briefs" => parse_audit(&mut parser)?,
        "judge" => parse_judge(&mut parser)?,
        _ => return Err(invalid(format!("unknown command {command:?}"))),
    };
    parser.finish()?;
    Ok(CliRequest {
        runs_root: parser.runs_root,
        output_format: parser.output_format,
        command: request,
    })
}

fn parse_qualify(parser: &mut ArgumentParser<'_>) -> Result<CliCommand, SkillEvalError> {
    let mut roots = Vec::new();
    let mut is_all_skills = false;
    let mut is_dry_run = false;
    let mut change_root = None;
    let mut incumbent_revision = None;
    let mut candidate_revision = None;
    let mut own_eval = None;
    let mut start_tier = Tier::T2;
    let mut reference_tier = Tier::T4;
    let mut repeats = DEFAULT_REPEATS;
    let mut minimum_score = DEFAULT_SCORE;
    let mut margin = DEFAULT_MARGIN;
    let mut confidence = DEFAULT_CONFIDENCE;

    while parser.peek().is_some() {
        let flag = parser.peek().expect("checked above").to_owned();
        match flag.as_str() {
            "--skill" | "--artifact" => roots.push(PathBuf::from(parser.value()?)),
            "--all-skills" => {
                parser.take_once("--all-skills")?;
                is_all_skills = true;
            }
            "--dry-run" => {
                parser.take_once("--dry-run")?;
                is_dry_run = true;
            }
            "--change-artifact" => {
                change_root = Some(PathBuf::from(parser.value_once("--change-artifact")?))
            }
            "--incumbent-revision" => {
                incumbent_revision = Some(parser.value_once("--incumbent-revision")?.to_owned())
            }
            "--candidate-revision" => {
                candidate_revision = Some(parser.value_once("--candidate-revision")?.to_owned())
            }
            "--own-eval" => own_eval = Some(PathBuf::from(parser.value_once("--own-eval")?)),
            "--start-tier" => start_tier = parse_tier(parser.value_once("--start-tier")?)?,
            "--reference-tier" => {
                reference_tier = parse_tier(parser.value_once("--reference-tier")?)?
            }
            "--run-id-file" => {
                let path = PathBuf::from(parser.value_once("--run-id-file")?);
                validate_output_path(&path, "run-id file")?;
                RUN_ID_FILE.with(|slot| *slot.borrow_mut() = Some(path));
            }
            "--trials" => repeats = parse_number(parser.value_once("--trials")?, "trials")?,
            "--minimum-score" => {
                minimum_score =
                    parse_number(parser.value_once("--minimum-score")?, "minimum score")?
            }
            "--noninferiority-margin" => {
                margin = parse_float(
                    parser.value_once("--noninferiority-margin")?,
                    "noninferiority margin",
                )?
            }
            "--confidence" => {
                confidence = parse_float(parser.value_once("--confidence")?, "confidence")?
            }
            _ if parser.take_common()? => {}
            _ => break,
        }
    }
    if is_all_skills && !roots.is_empty() {
        return Err(invalid("--all-skills conflicts with explicit artifacts"));
    }
    if is_all_skills {
        roots = all_skill_roots()?;
    }
    if let Some(root) = &change_root
        && !roots.contains(root)
    {
        roots.push(root.clone());
    }
    if roots.is_empty() {
        return Err(invalid("qualify requires at least one artifact"));
    }
    ensure_unique_paths(&roots)?;

    let change_values = [
        change_root.is_some(),
        incumbent_revision.is_some(),
        candidate_revision.is_some(),
        own_eval.is_some(),
    ];
    if change_values.iter().any(|value| *value) && !change_values.iter().all(|value| *value) {
        return Err(invalid(
            "changed qualification requires --change-artifact, --incumbent-revision, --candidate-revision, and --own-eval",
        ));
    }
    let change = if let (Some(root), Some(incumbent), Some(candidate), Some(evidence_path)) = (
        change_root,
        incumbent_revision,
        candidate_revision,
        own_eval,
    ) {
        let artifact = FileArtifactSource.load(&root)?;
        Some(ArtifactChange {
            artifact: artifact.name,
            kind: artifact.kind,
            incumbent_revision: incumbent,
            candidate_revision: candidate.clone(),
            own_eval: OwnEvalEvidence {
                artifact_revision: candidate,
                path: evidence_path,
            },
        })
    } else {
        None
    };
    let judge_tier = Tier::T5;
    let candidate_tiers = tier_search_order(start_tier, reference_tier, judge_tier)?;
    Ok(CliCommand::Qualify {
        request: QualifyRequest {
            artifact_roots: roots,
            change,
            policy: QualificationPolicy {
                purpose: QualificationPurpose::Artifact,
                candidate_tiers,
                reference_tier,
                judge_tier,
                repeats_per_case: repeats,
                minimum_score,
                noninferiority_margin: margin,
                confidence_level: confidence,
            },
            is_dry_run,
        },
    })
}

fn parse_run_only(parser: &mut ArgumentParser<'_>) -> Result<RunId, SkillEvalError> {
    let mut run_id = None;
    while parser.peek().is_some() {
        let flag = parser.peek().expect("checked above").to_owned();
        match flag.as_str() {
            "--run" => run_id = Some(parse_run_id(parser.value_once("--run")?)?),
            _ if parser.take_common()? => {}
            _ => break,
        }
    }
    run_id.ok_or_else(|| invalid("command requires --run"))
}

fn parse_inspect(parser: &mut ArgumentParser<'_>) -> Result<CliCommand, SkillEvalError> {
    let mut run_id = None;
    let mut artifact = None;
    let mut tier = None;
    let mut case = None;
    let mut attempt = None;
    while parser.peek().is_some() {
        let flag = parser.peek().expect("checked above").to_owned();
        match flag.as_str() {
            "--run" => run_id = Some(parse_run_id(parser.value_once("--run")?)?),
            "--skill" | "--artifact" => {
                artifact = Some(ArtifactName(
                    parser.value_once("--artifact/--skill")?.to_owned(),
                ))
            }
            "--tier" => tier = Some(parse_tier(parser.value_once("--tier")?)?),
            "--case" => {
                case = Some(CaseId(
                    nonempty(parser.value_once("--case")?, "case")?.to_owned(),
                ))
            }
            "--trial" | "--attempt" => {
                attempt = Some(parse_number(
                    parser.value_once("--trial/--attempt")?,
                    "trial",
                )?)
            }
            _ if parser.take_common()? => {}
            _ => break,
        }
    }
    Ok(CliCommand::Inspect {
        selector: TrialSelector {
            run_id: run_id.ok_or_else(|| invalid("inspect requires --run"))?,
            artifact: artifact.ok_or_else(|| invalid("inspect requires --skill or --artifact"))?,
            tier: tier.ok_or_else(|| invalid("inspect requires --tier"))?,
            case: case.ok_or_else(|| invalid("inspect requires --case"))?,
            attempt: attempt.ok_or_else(|| invalid("inspect requires --trial"))?,
        },
    })
}

fn parse_decide(parser: &mut ArgumentParser<'_>) -> Result<CliCommand, SkillEvalError> {
    let mut run_id = None;
    let mut artifact = None;
    let mut decision = None;
    let mut assignments = Vec::new();
    let mut reason = None;
    while parser.peek().is_some() {
        let flag = parser.peek().expect("checked above").to_owned();
        match flag.as_str() {
            "--run" => run_id = Some(parse_run_id(parser.value_once("--run")?)?),
            "--artifact" | "--skill" => {
                artifact = Some(ArtifactName(
                    parser.value_once("--artifact/--skill")?.to_owned(),
                ))
            }
            "--accept" => {
                parser.take();
                set_decision(&mut decision, Decision::Accepted)?;
            }
            "--reject" => {
                parser.take();
                set_decision(&mut decision, Decision::Rejected)?;
            }
            "--assign" => assignments.push(parse_assignment(parser.value()?)?),
            "--reason" => {
                reason = Some(nonempty(parser.value_once("--reason")?, "reason")?.to_owned())
            }
            _ if parser.take_common()? => {}
            _ => break,
        }
    }
    let decision = decision.ok_or_else(|| invalid("decide requires --accept or --reject"))?;
    if decision == Decision::Accepted && assignments.is_empty() {
        return Err(invalid("--accept requires at least one --assign"));
    }
    if decision == Decision::Rejected && (!assignments.is_empty() || reason.is_none()) {
        return Err(invalid(
            "--reject requires --reason and does not accept --assign",
        ));
    }
    ensure_unique_assignments(&assignments)?;
    Ok(CliCommand::Decide {
        run_id: run_id.ok_or_else(|| invalid("decide requires --run"))?,
        artifact: artifact.ok_or_else(|| invalid("decide requires --artifact"))?,
        decision,
        assignments,
        reason,
    })
}

fn parse_apply(parser: &mut ArgumentParser<'_>) -> Result<CliCommand, SkillEvalError> {
    let mut run_id = None;
    let mut artifact = None;
    while parser.peek().is_some() {
        let flag = parser.peek().expect("checked above").to_owned();
        match flag.as_str() {
            "--run" => run_id = Some(parse_run_id(parser.value_once("--run")?)?),
            "--artifact" | "--skill" => {
                artifact = Some(ArtifactName(
                    parser.value_once("--artifact/--skill")?.to_owned(),
                ))
            }
            _ if parser.take_common()? => {}
            _ => break,
        }
    }
    Ok(CliCommand::Apply {
        run_id: run_id.ok_or_else(|| invalid("apply requires --run"))?,
        artifact: artifact.ok_or_else(|| invalid("apply requires --artifact"))?,
    })
}

fn parse_audit(parser: &mut ArgumentParser<'_>) -> Result<CliCommand, SkillEvalError> {
    let mut roots = Vec::new();
    let mut is_all_skills = false;
    let mut output_root = None;
    while parser.peek().is_some() {
        let flag = parser.peek().expect("checked above").to_owned();
        match flag.as_str() {
            "--skill" | "--artifact" => roots.push(PathBuf::from(parser.value()?)),
            "--all-skills" => {
                parser.take_once("--all-skills")?;
                is_all_skills = true;
            }
            "--out" => output_root = Some(PathBuf::from(parser.value_once("--out")?)),
            _ if parser.take_common()? => {}
            _ => break,
        }
    }
    if is_all_skills && !roots.is_empty() {
        return Err(invalid("--all-skills conflicts with explicit artifacts"));
    }
    if is_all_skills {
        roots = all_skill_roots()?;
    }
    if roots.is_empty() {
        return Err(invalid("audit-briefs requires at least one artifact"));
    }
    ensure_unique_paths(&roots)?;
    Ok(CliCommand::AuditBriefs {
        request: AuditBriefRequest {
            artifact_roots: roots,
            output_root: output_root.ok_or_else(|| invalid("audit-briefs requires --out"))?,
        },
    })
}

fn parse_judge(parser: &mut ArgumentParser<'_>) -> Result<CliCommand, SkillEvalError> {
    let mut prompt = None;
    let mut prompt_file = None;
    let mut timeout_seconds = DEFAULT_JUDGE_TIMEOUT;
    let mut candidate_provider = None;
    let mut candidate_model = None;
    let mut candidate_tier = None;
    let mut candidate_thinking = None;
    while parser.peek().is_some() {
        let flag = parser.peek().expect("checked above").to_owned();
        match flag.as_str() {
            "--prompt" => prompt = Some(parser.value_once("--prompt")?.to_owned()),
            "--prompt-file" => prompt_file = Some(parser.value_once("--prompt-file")?.to_owned()),
            "--timeout" => {
                timeout_seconds = parse_number(parser.value_once("--timeout")?, "timeout")?
            }
            "--candidate-provider" => {
                candidate_provider = Some(parser.value_once("--candidate-provider")?.to_owned())
            }
            "--candidate-model" => {
                candidate_model = Some(parser.value_once("--candidate-model")?.to_owned())
            }
            "--candidate-tier" => {
                candidate_tier = Some(parse_tier(parser.value_once("--candidate-tier")?)?)
            }
            "--candidate-thinking" => {
                candidate_thinking = Some(parser.value_once("--candidate-thinking")?.to_owned())
            }
            _ if parser.take_common()? => {}
            _ => break,
        }
    }
    if prompt.is_some() && prompt_file.is_some() {
        return Err(invalid("--prompt conflicts with --prompt-file"));
    }
    let prompt = match (prompt, prompt_file) {
        (Some(value), None) => value,
        (None, Some(path)) => read_prompt(&path)?,
        (None, None) => return Err(invalid("judge requires --prompt or --prompt-file")),
        _ => unreachable!(),
    };
    let candidate_values = [
        candidate_provider.is_some(),
        candidate_model.is_some(),
        candidate_tier.is_some(),
        candidate_thinking.is_some(),
    ];
    if candidate_values.iter().any(|value| *value) && !candidate_values.iter().all(|value| *value) {
        return Err(invalid(
            "candidate identity requires provider, model, tier, and thinking",
        ));
    }
    let candidate_model = match (
        candidate_provider,
        candidate_model,
        candidate_tier,
        candidate_thinking,
    ) {
        (Some(provider), Some(model), Some(tier), Some(thinking)) => Some(ModelIdentity {
            tier,
            provider,
            model,
            thinking,
        }),
        _ => None,
    };
    Ok(CliCommand::Judge {
        request: PromptJudgeRequest {
            prompt: nonempty(&prompt, "prompt")?.to_owned(),
            candidate_model,
            timeout_seconds,
        },
    })
}

pub(crate) fn execute_command(
    request: CliRequest,
    runtime: &mut dyn QualificationRuntime,
    output: &mut dyn Write,
) -> Result<(), SkillEvalError> {
    let format = request.output_format;
    match request.command {
        CliCommand::Qualify { request } => {
            let mut progress = RenderProgress { format, output };
            let report = start_qualification(request, runtime, &mut progress)?;
            write_run_id_file(&report.run_id)?;
            render_report(&report, format, progress.output)
        }
        CliCommand::Report { run_id } => {
            render_report(&build_report(&run_id, runtime)?, format, output)
        }
        CliCommand::Inspect { selector } => {
            render_trial(&inspect_trial(&selector, runtime)?, format, output)
        }
        CliCommand::Resume { run_id } => {
            let mut progress = RenderProgress { format, output };
            let report = resume_qualification(&run_id, runtime, &mut progress)?;
            render_report(&report, format, progress.output)
        }
        CliCommand::Decide {
            run_id,
            artifact,
            decision,
            assignments,
            reason,
        } => {
            let clock = FixedClock(runtime.now());
            record_decision(
                &run_id,
                &artifact,
                decision,
                assignments,
                reason,
                runtime,
                &clock,
            )?;
            render_report(&build_report(&run_id, runtime)?, format, output)
        }
        CliCommand::Apply { run_id, artifact } => {
            apply_command(&run_id, &artifact, runtime)?;
            render_report(&build_report(&run_id, runtime)?, format, output)
        }
        CliCommand::AuditBriefs { request } => {
            let briefs = prepare_audit_briefs(&request, runtime)?;
            match format {
                OutputFormat::JsonLines => {
                    for brief in &briefs {
                        write_json_line(brief, output)?;
                    }
                }
                OutputFormat::Text => writeln!(output, "audit briefs written: {}", briefs.len())
                    .map_err(output_error)?,
            }
            Ok(())
        }
        CliCommand::Judge { request } => {
            let result = judge_prompt(&request, runtime)?;
            match format {
                OutputFormat::Text => output
                    .write_all(result.response.as_bytes())
                    .and_then(|()| output.write_all(b"\n"))
                    .map_err(output_error),
                OutputFormat::JsonLines => write_json_line(&result, output),
            }
        }
        CliCommand::PoolQualify { .. }
        | CliCommand::PoolReport { .. }
        | CliCommand::PoolResume { .. } => Err(SkillEvalError::InvalidConfiguration(
            "model-pool commands are not implemented".to_owned(),
        )),
    }
}

fn apply_command(
    run_id: &RunId,
    artifact_name: &ArtifactName,
    runtime: &mut dyn QualificationRuntime,
) -> Result<(), SkillEvalError> {
    let report = build_report(run_id, runtime)?;
    let change = report.change.as_ref().ok_or_else(|| {
        SkillEvalError::InvalidArguments("apply requires a changed-artifact run".to_owned())
    })?;
    if change.artifact != *artifact_name {
        return Err(invalid("apply artifact differs from the changed artifact"));
    }
    let artifact = frozen_artifact(run_id, artifact_name, runtime)?;
    let gate = evaluate_publication_gate(change, &report)?;
    apply_tier_assignments(&gate, &artifact, runtime)?;
    runtime.append(
        run_id,
        &RunEvent::PublicationGateEvaluated {
            at: runtime.now(),
            gate,
        },
    )
}

fn frozen_artifact(
    run_id: &RunId,
    name: &ArtifactName,
    store: &dyn RunStore,
) -> Result<ArtifactDefinition, SkillEvalError> {
    let mut found = None;
    store.replay(run_id, &mut |event| {
        if let RunEvent::RunStarted { configuration, .. } = event {
            found = configuration
                .artifacts
                .into_iter()
                .find(|artifact| artifact.name == *name);
        }
        Ok(())
    })?;
    found
        .ok_or_else(|| SkillEvalError::NotFound(format!("artifact {:?} is not in the run", name.0)))
}

pub(crate) fn render_event(
    event: &RunEvent,
    format: OutputFormat,
    output: &mut dyn Write,
) -> Result<(), SkillEvalError> {
    if format == OutputFormat::JsonLines {
        return write_json_line(event, output);
    }
    // TODO(AGNT-0032.T88): Render minimal pool-child completion identity before full pool reporting.
    let line = match event {
        RunEvent::RunStarted { configuration, .. } => {
            format!("run {} started", configuration.run_id.0)
        }
        RunEvent::TrialStarted { key, .. } => format!(
            "{} {:?} {} trial {} started",
            key.artifact.0, key.tier, key.case.0, key.attempt
        ),
        RunEvent::CandidateExecuted { candidate, .. } => format!(
            "{} {:?} {} trial {} candidate complete",
            candidate.key.artifact.0,
            candidate.key.tier,
            candidate.key.case.0,
            candidate.key.attempt
        ),
        RunEvent::TrialCompleted { record, .. } => format!(
            "{} {:?} {} trial {} score {}",
            record.key.artifact.0,
            record.key.tier,
            record.key.case.0,
            record.key.attempt,
            record.verdict.score
        ),
        RunEvent::TierEvaluated {
            artifact, evidence, ..
        } => format!("{} {:?} {:?}", artifact.0, evidence.tier, evidence.status),
        RunEvent::BoundaryFound {
            artifact, boundary, ..
        } => boundary_line(&artifact.0, boundary),
        RunEvent::ReviewRequired {
            artifact, reason, ..
        } => format!("{} needs review: {reason}", artifact.0),
        RunEvent::RunPaused { reason, .. } => format!("run paused: {reason:?}"),
        RunEvent::RunResumed { .. } => "run resumed".to_owned(),
        RunEvent::DecisionRecorded { decision, .. } => {
            format!("{} decision: {:?}", decision.artifact.0, decision.decision)
        }
        RunEvent::PublicationGateEvaluated { gate, .. } => format!(
            "{} publication gate: {:?}",
            gate.change.artifact.0, gate.status
        ),
        RunEvent::DiscoveryCompleted { artifacts, .. } => {
            format!("discovered {} artifacts", artifacts.len())
        }
    };
    writeln!(output, "{line}").map_err(output_error)
}

pub(crate) fn render_report(
    report: &QualificationReport,
    format: OutputFormat,
    output: &mut dyn Write,
) -> Result<(), SkillEvalError> {
    if format == OutputFormat::JsonLines {
        return write_json_line(report, output);
    }
    writeln!(output, "run {}: {:?}", report.run_id.0, report.status).map_err(output_error)?;
    for artifact in &report.artifacts {
        if let Some(boundary) = &artifact.boundary {
            writeln!(output, "{}", boundary_line(&artifact.artifact.0, boundary))
                .map_err(output_error)?;
        } else {
            writeln!(output, "{}: {:?}", artifact.artifact.0, artifact.status)
                .map_err(output_error)?;
        }
        if let Some(gate) = &artifact.publication_gate {
            writeln!(output, "  publication: {:?}", gate.status).map_err(output_error)?;
        }
        if let Some(reason) = &artifact.review_reason {
            writeln!(output, "  review: {reason}").map_err(output_error)?;
        }
        for evidence in &artifact.tiers {
            writeln!(
                output,
                "  {:?}: {:?}, score {:.2} [{:.2}, {:.2}], {}/{} trials, candidate ${:.6}, judge ${:.6}",
                evidence.tier,
                evidence.status,
                evidence.score.estimate,
                evidence.score.lower,
                evidence.score.upper,
                evidence.completed_trials,
                evidence.expected_trials,
                dollars(evidence.candidate_usage.cost_millionths_of_dollar),
                dollars(evidence.judge_usage.cost_millionths_of_dollar),
            )
            .map_err(output_error)?;
        }
    }
    if let Some(pause) = &report.pause {
        writeln!(output, "pause: {pause:?}").map_err(output_error)?;
    }
    writeln!(
        output,
        "total: {} input tokens, {} output tokens, ${:.6}",
        report.total_usage.input_tokens,
        report.total_usage.output_tokens,
        dollars(report.total_usage.cost_millionths_of_dollar)
    )
    .map_err(output_error)
}

pub(crate) fn render_trial(
    trial: &TrialRecord,
    format: OutputFormat,
    output: &mut dyn Write,
) -> Result<(), SkillEvalError> {
    validate_render_path(&trial.artifact_path)?;
    validate_render_path(&trial.transcript_path)?;
    if format == OutputFormat::JsonLines {
        return write_json_line(trial, output);
    }
    writeln!(
        output,
        "{} {:?} {} trial {}",
        trial.key.artifact.0, trial.key.tier, trial.key.case.0, trial.key.attempt
    )
    .map_err(output_error)?;
    writeln!(
        output,
        "model: {}/{}",
        trial.model.provider, trial.model.model
    )
    .map_err(output_error)?;
    writeln!(
        output,
        "judge: {}/{}",
        trial.judge_model.provider, trial.judge_model.model
    )
    .map_err(output_error)?;
    writeln!(
        output,
        "verdict: score {}, catastrophic {}",
        trial.verdict.score, trial.verdict.is_catastrophic
    )
    .map_err(output_error)?;
    for check in &trial.verdict.checks {
        writeln!(output, "check {}: {:?}", check.name, check.status).map_err(output_error)?;
    }
    writeln!(output, "artifact: {}", trial.artifact_path.display()).map_err(output_error)?;
    writeln!(output, "transcript: {}", trial.transcript_path.display()).map_err(output_error)?;
    writeln!(
        output,
        "usage: candidate {} tokens, judge {} tokens",
        trial.candidate_usage.input_tokens + trial.candidate_usage.output_tokens,
        trial.judge_usage.input_tokens + trial.judge_usage.output_tokens
    )
    .map_err(output_error)
}

struct RenderProgress<'a> {
    format: OutputFormat,
    output: &'a mut dyn Write,
}

impl ProgressSink for RenderProgress<'_> {
    fn emit(&mut self, event: &RunEvent) -> Result<(), SkillEvalError> {
        render_event(event, self.format, self.output)
    }
}

struct FixedClock(Timestamp);

impl Clock for FixedClock {
    fn now(&self) -> Timestamp {
        self.0.clone()
    }
}

pub(crate) struct ConcreteRuntime {
    source: FileArtifactSource,
    models: ConfiguredModelResolver,
    runner: PiCandidateRunner,
    verifier: FileVerifier,
    judge: PiJudge,
    store: FileRunStore,
    writer: FileTierWriter,
    run_ids: PathRunIdSource,
    pi_version: String,
}

impl ConcreteRuntime {
    pub(crate) fn new(runs_root: &Path) -> Result<Self, SkillEvalError> {
        fs::create_dir_all(runs_root).map_err(|error| SkillEvalError::Io {
            path: runs_root.to_path_buf(),
            message: error.to_string(),
        })?;
        let catalog = command_output("pi", &["--list-models"])?;
        let pi_version = command_output("pi", &["--version"])?;
        let configuration_path = repository_root()?.join("config/model-tiers.json");
        Ok(Self {
            source: FileArtifactSource,
            models: ConfiguredModelResolver::load(&configuration_path, &catalog)?,
            runner: PiCandidateRunner::new(runs_root.to_path_buf()),
            verifier: FileVerifier::new(runs_root)?,
            judge: PiJudge::new(),
            store: FileRunStore::new(runs_root)?,
            writer: FileTierWriter,
            run_ids: PathRunIdSource::new(runs_root)?,
            pi_version: pi_version.trim().to_owned(),
        })
    }
}

impl ArtifactSource for ConcreteRuntime {
    fn load(&self, root: &Path) -> Result<ArtifactDefinition, SkillEvalError> {
        self.source.load(root)
    }
}

impl ModelResolver for ConcreteRuntime {
    fn candidates(&self, tier: Tier) -> Result<Vec<ModelIdentity>, SkillEvalError> {
        self.models.candidates(tier)
    }

    fn exact_candidate(&self, _requested: &ModelIdentity) -> Result<ModelIdentity, SkillEvalError> {
        Err(SkillEvalError::InvalidConfiguration(
            "exact model-pool candidate resolution is not implemented".to_owned(),
        ))
    }

    fn configured_judge_tier(&self) -> Result<Tier, SkillEvalError> {
        self.models.configured_judge_tier()
    }

    fn pool_judge(&self, candidate: &ModelIdentity) -> Result<ModelIdentity, SkillEvalError> {
        self.models.pool_judge(candidate)
    }

    fn judge(
        &self,
        judge_tier: Tier,
        candidate: Option<&ModelIdentity>,
    ) -> Result<ModelIdentity, SkillEvalError> {
        self.models.judge(judge_tier, candidate)
    }
}

impl HarnessResolver for ConcreteRuntime {
    fn identity(
        &self,
        artifact: &ArtifactDefinition,
        execution: &ExecutionDefinition,
    ) -> Result<HarnessIdentity, SkillEvalError> {
        if artifact.revision.trim().is_empty() {
            return Err(SkillEvalError::InvalidConfiguration(
                "harness identity requires an artifact revision".to_owned(),
            ));
        }
        let policy = serde_json::to_vec(execution).map_err(|error| {
            SkillEvalError::InvalidConfiguration(format!(
                "tool policy serialization failed: {error}"
            ))
        })?;
        Ok(HarnessIdentity {
            runner_version: RUNNER_VERSION.to_owned(),
            pi_version: self.pi_version.clone(),
            artifact_revision: artifact.revision.clone(),
            tool_policy_digest: stable_digest(&policy),
        })
    }
}

impl RunIdSource for ConcreteRuntime {
    fn next(&mut self) -> Result<RunId, SkillEvalError> {
        self.run_ids.next()
    }
}

impl CandidateRunner for ConcreteRuntime {
    fn execute(
        &mut self,
        run_id: &RunId,
        key: &crate::model::TrialKey,
        artifact: &ArtifactDefinition,
        case: &crate::model::CaseDefinition,
        model: &ModelIdentity,
        harness: &HarnessIdentity,
    ) -> Result<crate::model::CandidateArtifact, SkillEvalError> {
        self.runner
            .execute(run_id, key, artifact, case, model, harness)
    }
}

impl Verifier for ConcreteRuntime {
    fn verify(
        &mut self,
        case: &crate::model::CaseDefinition,
        candidate: &crate::model::CandidateArtifact,
    ) -> Result<Vec<crate::model::CheckResult>, SkillEvalError> {
        self.verifier.verify(case, candidate)
    }
}

impl Judge for ConcreteRuntime {
    fn grade(
        &mut self,
        model: &ModelIdentity,
        input: &crate::model::JudgeInput,
    ) -> Result<crate::model::JudgeResult, SkillEvalError> {
        self.judge.grade(model, input)
    }

    fn grade_prompt(
        &mut self,
        model: &ModelIdentity,
        request: &PromptJudgeRequest,
    ) -> Result<crate::model::PromptJudgeResult, SkillEvalError> {
        self.judge.grade_prompt(model, request)
    }
}

impl RunStore for ConcreteRuntime {
    fn append(&mut self, run_id: &RunId, event: &RunEvent) -> Result<(), SkillEvalError> {
        self.store.append(run_id, event)
    }

    fn replay(
        &self,
        run_id: &RunId,
        visitor: &mut dyn FnMut(RunEvent) -> Result<(), SkillEvalError>,
    ) -> Result<(), SkillEvalError> {
        self.store.replay(run_id, visitor)
    }

    fn find_trial(&self, selector: &TrialSelector) -> Result<TrialRecord, SkillEvalError> {
        self.store.find_trial(selector)
    }
}

impl Clock for ConcreteRuntime {
    fn now(&self) -> Timestamp {
        let value = Command::new("date")
            .arg("+%Y-%m-%dT%H:%M:%S%z")
            .output()
            .ok()
            .filter(|output| output.status.success())
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .map(|value| value.trim().to_owned())
            .unwrap_or_else(|| "1970-01-01T00:00:00+0000".to_owned());
        Timestamp(value)
    }
}

impl TierWriter for ConcreteRuntime {
    fn write(
        &mut self,
        artifact: &ArtifactDefinition,
        assignments: &[TierAssignment],
    ) -> Result<(), SkillEvalError> {
        self.writer.write(artifact, assignments)
    }
}

impl QualificationRuntime for ConcreteRuntime {}

struct PathRunIdSource {
    reservation_root: PathBuf,
}

impl PathRunIdSource {
    fn new(runs_root: &Path) -> Result<Self, SkillEvalError> {
        let reservation_root = runs_root.join(".run-ids");
        fs::create_dir_all(&reservation_root).map_err(|error| SkillEvalError::Io {
            path: reservation_root.clone(),
            message: error.to_string(),
        })?;
        Ok(Self { reservation_root })
    }
}

impl RunIdSource for PathRunIdSource {
    fn next(&mut self) -> Result<RunId, SkillEvalError> {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        for _ in 0..128 {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|error| {
                    SkillEvalError::InvalidConfiguration(format!(
                        "system clock is invalid: {error}"
                    ))
                })?
                .as_nanos();
            let sequence = COUNTER.fetch_add(1, Ordering::Relaxed);
            let value = format!(
                "run-{nanos:032x}-{:08x}-{sequence:016x}",
                std::process::id()
            );
            let reservation = self.reservation_root.join(&value);
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&reservation)
            {
                Ok(_) => return Ok(RunId(value)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(SkillEvalError::Io {
                        path: reservation,
                        message: error.to_string(),
                    });
                }
            }
        }
        Err(SkillEvalError::InvalidConfiguration(
            "could not reserve a unique run identifier".to_owned(),
        ))
    }
}

pub(crate) fn run_main() -> Result<(), SkillEvalError> {
    let arguments = env::args_os().skip(1).collect::<Vec<_>>();
    if arguments.len() == 1 && matches!(arguments[0].to_str(), Some("--help" | "-h")) {
        print_help(&mut std::io::stdout()).map_err(output_error)?;
        return Ok(());
    }
    let request = parse_arguments(&arguments)?;
    let mut runtime = ConcreteRuntime::new(&request.runs_root)?;
    execute_command(request, &mut runtime, &mut std::io::stdout())
}

fn print_help(output: &mut dyn Write) -> std::io::Result<()> {
    writeln!(output, "skill-eval <command> [options]")?;
    writeln!(output, "commands:")?;
    for command in [
        "qualify",
        "report",
        "inspect",
        "resume",
        "decide",
        "apply",
        "audit-briefs",
        "judge",
    ] {
        writeln!(output, "  {command}")?;
    }
    writeln!(
        output,
        "common options: --runs-root PATH; --format text; --format jsonl"
    )?;
    writeln!(
        output,
        "start options: --skill PATH|--artifact PATH|--all-skills [--dry-run] [--change-artifact PATH --incumbent-revision REV --candidate-revision REV --own-eval PATH] [--start-tier TIER] [--reference-tier TIER] [--run-id-file PATH] [--trials N]"
    )?;
    writeln!(
        output,
        "owner options: --run ID --artifact NAME (--accept --assign DEST=TIER...|--reject --reason TEXT)"
    )?;
    writeln!(
        output,
        "prompt options: --prompt TEXT|--prompt-file PATH [--timeout SECONDS]"
    )
}

struct ArgumentParser<'a> {
    values: &'a [String],
    index: usize,
    singleton_flags: BTreeSet<&'static str>,
    runs_root: PathBuf,
    output_format: OutputFormat,
}

impl<'a> ArgumentParser<'a> {
    fn new(values: &'a [String]) -> Self {
        Self {
            values,
            index: 0,
            singleton_flags: BTreeSet::new(),
            runs_root: PathBuf::from(DEFAULT_RUNS_ROOT),
            output_format: OutputFormat::Text,
        }
    }

    fn peek(&self) -> Option<&str> {
        self.values.get(self.index).map(String::as_str)
    }

    fn take(&mut self) -> Option<&str> {
        let index = self.index;
        self.index += 1;
        self.values.get(index).map(String::as_str)
    }

    fn value(&mut self) -> Result<&str, SkillEvalError> {
        let flag = self
            .take()
            .ok_or_else(|| invalid("missing flag"))?
            .to_owned();
        let value = self
            .take()
            .ok_or_else(|| invalid(format!("{flag} requires a value")))?;
        if value.starts_with("--") {
            return Err(invalid(format!("{flag} requires a value")));
        }
        Ok(value)
    }

    fn value_once(&mut self, flag: &'static str) -> Result<&str, SkillEvalError> {
        self.mark_singleton(flag)?;
        self.value()
    }

    fn take_once(&mut self, flag: &'static str) -> Result<(), SkillEvalError> {
        self.mark_singleton(flag)?;
        self.take();
        Ok(())
    }

    fn mark_singleton(&mut self, flag: &'static str) -> Result<(), SkillEvalError> {
        if !self.singleton_flags.insert(flag) {
            return Err(invalid(format!("{flag} may be specified only once")));
        }
        Ok(())
    }

    fn take_common(&mut self) -> Result<bool, SkillEvalError> {
        match self.peek() {
            Some("--runs-root") => {
                self.runs_root = PathBuf::from(self.value_once("--runs-root")?);
                validate_output_path(&self.runs_root, "runs root")?;
                Ok(true)
            }
            Some("--format") => {
                self.output_format = match self.value_once("--format")? {
                    "text" => OutputFormat::Text,
                    "jsonl" => OutputFormat::JsonLines,
                    value => return Err(invalid(format!("unknown output format {value:?}"))),
                };
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn finish(&self) -> Result<(), SkillEvalError> {
        match self.peek() {
            Some(value) => Err(invalid(format!("unknown or misplaced flag {value:?}"))),
            None => Ok(()),
        }
    }
}

fn parse_assignment(value: &str) -> Result<TierAssignment, SkillEvalError> {
    let (destination, tier) = value
        .split_once('=')
        .ok_or_else(|| invalid("assignment must be <destination>=<tier>"))?;
    let destination = match destination {
        "skill_minimum" | "minimum" => TierDestination::SkillMinimum,
        "skill_target" | "target" => TierDestination::SkillTarget,
        "agent" => TierDestination::Agent,
        "workflow_orchestrator" | "orchestrator" => TierDestination::WorkflowOrchestrator,
        value if value.starts_with("workflow_node:") => TierDestination::WorkflowNode {
            node: nonempty(&value["workflow_node:".len()..], "workflow node")?.to_owned(),
        },
        _ => return Err(invalid(format!("unknown tier destination {destination:?}"))),
    };
    Ok(TierAssignment {
        destination,
        tier: parse_tier(tier)?,
    })
}

fn parse_tier(value: &str) -> Result<Tier, SkillEvalError> {
    match value.to_ascii_lowercase().as_str() {
        "t1" => Ok(Tier::T1),
        "t2" => Ok(Tier::T2),
        "t3" => Ok(Tier::T3),
        "t4" => Ok(Tier::T4),
        "t5" => Ok(Tier::T5),
        _ => Err(invalid(format!("unknown tier {value:?}"))),
    }
}

fn tier_search_order(
    start: Tier,
    reference: Tier,
    judge: Tier,
) -> Result<Vec<Tier>, SkillEvalError> {
    if judge <= reference {
        return Err(invalid("judge tier must be above the reference tier"));
    }
    if start >= judge {
        return Err(invalid("start tier must be below the judge tier"));
    }
    let all = [Tier::T1, Tier::T2, Tier::T3, Tier::T4];
    if start == reference {
        return Err(invalid("start tier must differ from the reference tier"));
    }
    let mut tiers = vec![start];
    tiers.extend(
        all.iter()
            .rev()
            .copied()
            .filter(|tier| *tier < start && *tier != reference),
    );
    tiers.extend(
        all.iter()
            .copied()
            .filter(|tier| *tier > start && *tier < judge && *tier != reference),
    );
    Ok(tiers)
}

fn parse_run_id(value: &str) -> Result<RunId, SkillEvalError> {
    let run_id = RunId(nonempty(value, "run identifier")?.to_owned());
    let path = Path::new(value);
    if path.is_absolute()
        || path.components().count() != 1
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        || value.contains(['/', '\\'])
        || value.chars().any(char::is_control)
    {
        return Err(invalid("run identifier must be one safe path component"));
    }
    Ok(run_id)
}

fn all_skill_roots() -> Result<Vec<PathBuf>, SkillEvalError> {
    let root = repository_root()?.join("skills");
    let mut roots = fs::read_dir(&root)
        .map_err(|error| SkillEvalError::Io {
            path: root.clone(),
            message: error.to_string(),
        })?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            entry
                .file_type()
                .ok()
                .filter(|kind| kind.is_dir())
                .map(|_| entry.path())
        })
        .filter(|path| path.join("SKILL.md").is_file())
        .collect::<Vec<_>>();
    roots.sort();
    if roots.is_empty() {
        return Err(invalid("--all-skills found no skills"));
    }
    Ok(roots)
}

fn repository_root() -> Result<PathBuf, SkillEvalError> {
    let mut current = env::current_dir().map_err(|error| SkillEvalError::Io {
        path: PathBuf::from("."),
        message: error.to_string(),
    })?;
    loop {
        if current.join("config/model-tiers.json").is_file() && current.join("skills").is_dir() {
            return Ok(current);
        }
        if !current.pop() {
            return Err(SkillEvalError::InvalidConfiguration(
                "repository root with config/model-tiers.json was not found".to_owned(),
            ));
        }
    }
}

fn command_output(program: &str, arguments: &[&str]) -> Result<String, SkillEvalError> {
    let output = Command::new(program)
        .args(arguments)
        .output()
        .map_err(|error| SkillEvalError::Process {
            program: program.to_owned(),
            exit_code: None,
            standard_error: error.to_string(),
        })?;
    if !output.status.success() {
        return Err(SkillEvalError::Process {
            program: program.to_owned(),
            exit_code: output.status.code(),
            standard_error: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }
    String::from_utf8(output.stdout).map_err(|error| {
        SkillEvalError::InvalidConfiguration(format!("{program} output is not UTF-8: {error}"))
    })
}

fn read_prompt(path: &str) -> Result<String, SkillEvalError> {
    if path == "-" {
        let mut value = String::new();
        std::io::stdin()
            .read_to_string(&mut value)
            .map_err(|error| SkillEvalError::Io {
                path: PathBuf::from("<stdin>"),
                message: error.to_string(),
            })?;
        return Ok(value);
    }
    fs::read_to_string(path).map_err(|error| SkillEvalError::Io {
        path: PathBuf::from(path),
        message: error.to_string(),
    })
}

fn stable_digest(bytes: &[u8]) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("fnv1a64:{hash:016x}")
}

fn write_run_id_file(run_id: &RunId) -> Result<(), SkillEvalError> {
    let path = RUN_ID_FILE.with(|slot| slot.borrow_mut().take());
    let Some(path) = path else {
        return Ok(());
    };
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|error| SkillEvalError::Io {
            path: parent.to_path_buf(),
            message: error.to_string(),
        })?;
    }
    fs::write(&path, format!("{}\n", run_id.0)).map_err(|error| SkillEvalError::Io {
        path,
        message: error.to_string(),
    })
}

fn write_json_line<T: Serialize + ?Sized>(
    value: &T,
    output: &mut dyn Write,
) -> Result<(), SkillEvalError> {
    serde_json::to_writer(&mut *output, value).map_err(|error| {
        SkillEvalError::InvalidConfiguration(format!("output serialization failed: {error}"))
    })?;
    output.write_all(b"\n").map_err(output_error)
}

fn boundary_line(name: &str, boundary: &crate::model::QualificationBoundary) -> String {
    match &boundary.failing {
        Some(failing) => format!(
            "{}: {:?} failed -> {:?} accepted",
            name, failing.tier, boundary.accepted.tier
        ),
        None => format!("{}: {:?} accepted", name, boundary.accepted.tier),
    }
}

fn dollars(millionths: u64) -> f64 {
    millionths as f64 / 1_000_000.0
}

fn validate_render_path(path: &Path) -> Result<(), SkillEvalError> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
        || path.to_string_lossy().chars().any(char::is_control)
    {
        return Err(SkillEvalError::InvalidConfiguration(
            "trial output contains an unsafe path".to_owned(),
        ));
    }
    Ok(())
}

fn validate_output_path(path: &Path, label: &str) -> Result<(), SkillEvalError> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(invalid(format!(
            "{label} must not be empty or contain a parent segment"
        )));
    }
    Ok(())
}

fn ensure_unique_paths(paths: &[PathBuf]) -> Result<(), SkillEvalError> {
    let mut seen = BTreeSet::new();
    if paths.iter().any(|path| !seen.insert(path.clone())) {
        return Err(invalid("artifact paths must be unique"));
    }
    Ok(())
}

fn ensure_unique_assignments(assignments: &[TierAssignment]) -> Result<(), SkillEvalError> {
    let mut seen = BTreeSet::new();
    if assignments
        .iter()
        .any(|assignment| !seen.insert(assignment.destination.clone()))
    {
        return Err(invalid("tier assignment destinations must be unique"));
    }
    Ok(())
}

fn set_decision(current: &mut Option<Decision>, value: Decision) -> Result<(), SkillEvalError> {
    if current.replace(value).is_some() {
        return Err(invalid("choose exactly one of --accept or --reject"));
    }
    Ok(())
}

fn parse_number<T>(value: &str, label: &str) -> Result<T, SkillEvalError>
where
    T: std::str::FromStr,
{
    value
        .parse()
        .map_err(|_| invalid(format!("{label} is not a valid number")))
}

fn parse_float(value: &str, label: &str) -> Result<f64, SkillEvalError> {
    let number: f64 = parse_number(value, label)?;
    if !number.is_finite() {
        return Err(invalid(format!("{label} must be finite")));
    }
    Ok(number)
}

fn nonempty<'a>(value: &'a str, label: &str) -> Result<&'a str, SkillEvalError> {
    if value.trim().is_empty() {
        Err(invalid(format!("{label} must not be empty")))
    } else {
        Ok(value)
    }
}

fn invalid(message: impl Into<String>) -> SkillEvalError {
    SkillEvalError::InvalidArguments(message.into())
}

fn output_error(error: std::io::Error) -> SkillEvalError {
    SkillEvalError::Io {
        path: PathBuf::from("<stdout>"),
        message: error.to_string(),
    }
}

#[cfg(test)]
include!("../tests/cli.rs");

#[cfg(test)]
cli_tests!();
