use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};

use crate::frontier_store::FileFrontierStore;
use crate::judge::PiJudge;
use crate::model::{
    ArtifactChange, ArtifactDefinition, ArtifactName, AuditBriefRequest, CandidateEnvironmentEntry,
    CaseDefinition, CaseId, CliCommand, CliRequest, Decision, ExecutionDefinition,
    FRONTIER_WORKER_LIMIT, FrontierApplyReport, FrontierBaseline, FrontierBaselineLedger,
    FrontierCaseGroup, FrontierDecisionRequest, FrontierInspection, FrontierPlan,
    FrontierPreviewReport, FrontierReport, FrontierRunId, FrontierRunState, FrontierSuite,
    FrontierSuiteConstructionPlan, FrontierSuiteInventory, FrontierSuiteProposal,
    FrontierSuitePublication, FrontierSuiteReviewSet, FrontierTrialJob, FrontierTrialOutcome,
    FrontierTrialSelector, HarnessIdentity, JudgeInput, ModelIdentity, OutputFormat,
    OwnEvalEvidence, PoolChildStatus, PoolEntrant, PoolEntrantEvidence, PoolQualifyRequest,
    PoolRunId, PoolRunState, PoolRunStatus, PromptJudgeRequest, QualificationPolicy,
    QualificationPurpose, QualificationReport, QualifyRequest, RunEvent, RunId, SkillEvalError,
    T1ScreenCampaignCapExtensionRequest, T1ScreenCampaignCreateRequest, T1ScreenCampaignId,
    T1ScreenCampaignRunRetirementRequest, T1ScreenCandidateEnvironment, T1ScreenCandidatePrice,
    T1ScreenCapExtensionRequest, T1ScreenExclusionReason, T1ScreenFormat, T1ScreenModelState,
    T1ScreenPolicy, T1ScreenPreviewReport, T1ScreenReport, T1ScreenRouteFailureRequest,
    T1ScreenRunConfiguration, T1ScreenRunId, T1ScreenRunState, T1ScreenRunStatus,
    T1ScreenStartRequest, Tier, TierAssignment, TierDestination, Timestamp, TrialRecord,
    TrialSelector, TrialUsage,
};
use crate::model_capabilities;
use crate::models::{ConfiguredModelResolver, validate_rpc_models_data};
use crate::pi_runner::PiCandidateRunner;
use crate::pool_source::FilePoolPlanSource;
use crate::pool_store::FilePoolStore;
use crate::ports::{
    ArtifactSource, CandidateRunner, Clock, FrontierProgressSink, FrontierRuntime,
    FrontierSuiteRuntime, HarnessResolver, Judge, ModelResolver, PoolPlanSource, PoolProgressSink,
    PoolRunIdSource, PoolRuntime, PoolStore, ProgressSink, QualificationRuntime, RunIdSource,
    RunStore, T1ScreenProgressSink, T1ScreenRuntime, T1ScreenStore, TierWriter, Verifier,
};
use crate::service::{
    FrontierApplyRuntime, FrontierPreviewRuntime, FrontierTrialRuntime, active_frontier_routes,
    apply_frontier_baseline, apply_frontier_suite, apply_tier_assignments, build_pool_report,
    build_report, build_t1_screen_report, check_frontier_suite, evaluate_publication_gate,
    extend_t1_screen_cap, fail_t1_screen_route, inspect_frontier, inspect_trial,
    inventory_frontier_suite, judge_prompt, pending_t1_screen_state, prepare_audit_briefs,
    preview_frontier, propose_frontier_suite, record_decision, record_frontier_decision,
    resume_frontier, resume_pool_qualification, resume_qualification, resume_t1_screening,
    start_frontier, start_pool_qualification, start_pool_replacement_qualification,
    start_qualification, start_t1_screening,
};
use crate::source::FileArtifactSource;
use crate::statistics::select_thinking_level;
use crate::store::FileRunStore;
use crate::t1_screen_campaign_store::{
    FileT1ScreenCampaignStore, T1_SCREEN_CAMPAIGN_APPROVED_TOTAL,
};
use crate::t1_screen_store::{
    FileT1ScreenStore, candidate_environment_manifest_digest, preallocate_t1_screen_children,
    t1_screen_classification_digest,
};
use crate::tier_writer::FileTierWriter;
use crate::verifier::FileVerifier;

const DEFAULT_RUNS_ROOT: &str = ".map/skill-eval/runs";
const DEFAULT_REPEATS: u16 = 3;
const DEFAULT_SCORE: u8 = 8;
const DEFAULT_MARGIN: f64 = 1.0;
const DEFAULT_CONFIDENCE: f64 = 0.95;
const DEFAULT_JUDGE_TIMEOUT: u32 = 120;
const RUNNER_VERSION: &str = env!("CARGO_PKG_VERSION");
const MODELS_RPC_ID: &str = "skill-eval-models";
const MODELS_RPC_REQUEST: &[u8] =
    b"{\"id\":\"skill-eval-models\",\"type\":\"get_available_models\"}\n";
const MODELS_RPC_ARGUMENTS: [&str; 5] = [
    "--mode",
    "rpc",
    "--no-session",
    "--no-context-files",
    "--no-extensions",
];
static RUN_ID_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static ROUTING_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

thread_local! {
    static RUN_ID_FILE: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

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
        "model-capabilities" => parse_model_capabilities(&mut parser)?,
        "t1-screen-preview" => parse_t1_screen_preview(&mut parser)?,
        "t1-screen-campaign-create" => parse_t1_screen_campaign_create(&mut parser)?,
        "t1-screen-campaign-extend-cap" => parse_t1_screen_campaign_extend_cap(&mut parser)?,
        "t1-screen-campaign-retire-run" => parse_t1_screen_campaign_retire_run(&mut parser)?,
        "t1-screen-start" => parse_t1_screen_start(&mut parser)?,
        "t1-screen-resume" => parse_t1_screen_run(&mut parser, true)?,
        "t1-screen-extend-cap" => parse_t1_screen_extend_cap(&mut parser)?,
        "t1-screen-fail-route" => parse_t1_screen_fail_route(&mut parser)?,
        "t1-screen-report" => parse_t1_screen_run(&mut parser, false)?,
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
        "pool-qualify" => parse_pool_qualify(&mut parser)?,
        "pool-report" => CliCommand::PoolReport {
            run_id: parse_pool_run_only(&mut parser)?,
        },
        "pool-resume" => CliCommand::PoolResume {
            run_id: parse_pool_run_only(&mut parser)?,
        },
        "pool-replacement" => parse_pool_replacement(&mut parser)?,
        "frontier-suite-inventory" => parse_frontier_suite_inventory(&mut parser)?,
        "frontier-suite-propose" => parse_frontier_suite_propose(&mut parser)?,
        "frontier-suite-check" => parse_frontier_suite_check(&mut parser)?,
        "frontier-suite-apply" => parse_frontier_suite_apply(&mut parser)?,
        "frontier-preview" => parse_frontier_preview(&mut parser)?,
        "frontier-start" => parse_frontier_start(&mut parser)?,
        "frontier-resume" => parse_frontier_resume(&mut parser)?,
        "frontier-report" => parse_frontier_report(&mut parser)?,
        "frontier-inspect" => parse_frontier_inspect(&mut parser)?,
        "frontier-decide" => parse_frontier_decide(&mut parser)?,
        "frontier-apply" => parse_frontier_apply(&mut parser)?,
        _ => return Err(invalid(format!("unknown command {command:?}"))),
    };
    parser.finish()?;
    Ok(CliRequest {
        runs_root: parser.runs_root,
        output_format: parser.output_format,
        command: request,
    })
}

fn parse_model_capabilities(parser: &mut ArgumentParser<'_>) -> Result<CliCommand, SkillEvalError> {
    let mut output = None;
    while parser.peek().is_some() {
        match parser.peek().expect("checked above") {
            "--output" => {
                let path = PathBuf::from(parser.value_once("--output")?);
                validate_repository_path(&path, "model capability output")?;
                output = Some(path);
            }
            _ => break,
        }
    }
    Ok(CliCommand::ModelCapabilities {
        output: output.ok_or_else(|| invalid("model-capabilities requires --output"))?,
    })
}

fn parse_t1_screen_preview(parser: &mut ArgumentParser<'_>) -> Result<CliCommand, SkillEvalError> {
    let mut capabilities = None;
    let mut format = T1ScreenFormat::Text;
    while parser.peek().is_some() {
        match parser.peek().expect("checked above") {
            "--capabilities" => {
                let path = PathBuf::from(parser.value_once("--capabilities")?);
                validate_repository_path(&path, "T1 capability snapshot")?;
                capabilities = Some(path);
            }
            "--format" => {
                format = match parser.value_once("--format")? {
                    "text" => T1ScreenFormat::Text,
                    "json" => T1ScreenFormat::Json,
                    value => return Err(invalid(format!("unknown T1 preview format {value:?}"))),
                };
            }
            _ => break,
        }
    }
    Ok(CliCommand::T1ScreenPreview {
        capabilities: capabilities
            .ok_or_else(|| invalid("t1-screen-preview requires --capabilities"))?,
        format,
    })
}

fn parse_t1_screen_campaign_create(
    parser: &mut ArgumentParser<'_>,
) -> Result<CliCommand, SkillEvalError> {
    let mut campaign_id = None;
    let mut judge_cap = None;
    let mut owner_reason = None;
    let mut run_ids = Vec::new();
    let mut format = T1ScreenFormat::Text;
    while parser.peek().is_some() {
        let flag = parser.peek().expect("checked above").to_owned();
        match flag.as_str() {
            "--campaign" => {
                campaign_id = Some(parse_t1_screen_campaign_id(
                    parser.value_once("--campaign")?,
                )?);
            }
            "--judge-cap-millionths" => {
                judge_cap = Some(parse_positive_number(
                    parser.value_once("--judge-cap-millionths")?,
                    "campaign judge cap millionths",
                )?);
            }
            "--reason" => {
                owner_reason = Some(
                    nonempty(parser.value_once("--reason")?, "campaign owner reason")?.to_owned(),
                );
            }
            "--run" => run_ids.push(parse_t1_screen_run_id(parser.value()?)?),
            "--format" => format = parse_t1_screen_format(parser.value_once("--format")?)?,
            _ if parser.take_common()? => {}
            _ => break,
        }
    }
    if run_ids.is_empty() {
        return Err(invalid(
            "t1-screen-campaign-create requires at least one --run",
        ));
    }
    let judge_cap = judge_cap
        .ok_or_else(|| invalid("t1-screen-campaign-create requires --judge-cap-millionths"))?;
    if judge_cap != T1_SCREEN_CAMPAIGN_APPROVED_TOTAL {
        return Err(invalid(format!(
            "t1-screen-campaign-create judge cap must be exactly {T1_SCREEN_CAMPAIGN_APPROVED_TOTAL} millionths"
        )));
    }
    Ok(CliCommand::T1ScreenCampaignCreate {
        request: T1ScreenCampaignCreateRequest {
            campaign_id: campaign_id
                .ok_or_else(|| invalid("t1-screen-campaign-create requires --campaign"))?,
            judge_cap_millionths_of_dollar: judge_cap,
            owner_reason: owner_reason
                .ok_or_else(|| invalid("t1-screen-campaign-create requires --reason"))?,
            run_ids,
        },
        format,
    })
}

fn parse_t1_screen_campaign_extend_cap(
    parser: &mut ArgumentParser<'_>,
) -> Result<CliCommand, SkillEvalError> {
    let mut campaign_id = None;
    let mut approved_total = None;
    let mut owner_reason = None;
    let mut format = T1ScreenFormat::Text;
    while parser.peek().is_some() {
        let flag = parser.peek().expect("checked above").to_owned();
        match flag.as_str() {
            "--campaign" => {
                campaign_id = Some(parse_t1_screen_campaign_id(
                    parser.value_once("--campaign")?,
                )?);
            }
            "--judge-cap-millionths" => {
                approved_total = Some(parse_positive_number(
                    parser.value_once("--judge-cap-millionths")?,
                    "campaign judge cap millionths",
                )?);
            }
            "--reason" => {
                owner_reason = Some(
                    nonempty(parser.value_once("--reason")?, "campaign owner reason")?.to_owned(),
                );
            }
            "--format" => format = parse_t1_screen_format(parser.value_once("--format")?)?,
            _ if parser.take_common()? => {}
            _ => break,
        }
    }
    Ok(CliCommand::T1ScreenCampaignExtendCap {
        request: T1ScreenCampaignCapExtensionRequest {
            campaign_id: campaign_id
                .ok_or_else(|| invalid("t1-screen-campaign-extend-cap requires --campaign"))?,
            new_approved_total_millionths_of_dollar: approved_total.ok_or_else(|| {
                invalid("t1-screen-campaign-extend-cap requires --judge-cap-millionths")
            })?,
            owner_reason: owner_reason
                .ok_or_else(|| invalid("t1-screen-campaign-extend-cap requires --reason"))?,
        },
        format,
    })
}

fn parse_t1_screen_campaign_retire_run(
    parser: &mut ArgumentParser<'_>,
) -> Result<CliCommand, SkillEvalError> {
    let mut campaign_id = None;
    let mut run_id = None;
    let mut owner_reason = None;
    let mut format = T1ScreenFormat::Text;
    while parser.peek().is_some() {
        let flag = parser.peek().expect("checked above").to_owned();
        match flag.as_str() {
            "--campaign" => {
                campaign_id = Some(parse_t1_screen_campaign_id(
                    parser.value_once("--campaign")?,
                )?);
            }
            "--run" => {
                run_id = Some(parse_t1_screen_run_id(parser.value_once("--run")?)?);
            }
            "--reason" => {
                owner_reason = Some(
                    nonempty(parser.value_once("--reason")?, "retirement owner reason")?.to_owned(),
                );
            }
            "--format" => format = parse_t1_screen_format(parser.value_once("--format")?)?,
            _ => break,
        }
    }
    Ok(CliCommand::T1ScreenCampaignRetireRun {
        request: T1ScreenCampaignRunRetirementRequest {
            campaign_id: campaign_id
                .ok_or_else(|| invalid("t1-screen-campaign-retire-run requires --campaign"))?,
            run_id: run_id
                .ok_or_else(|| invalid("t1-screen-campaign-retire-run requires --run"))?,
            owner_reason: owner_reason
                .ok_or_else(|| invalid("t1-screen-campaign-retire-run requires --reason"))?,
        },
        format,
    })
}

fn parse_t1_screen_start(parser: &mut ArgumentParser<'_>) -> Result<CliCommand, SkillEvalError> {
    let mut campaign_id = None;
    let mut capabilities = None;
    let mut exam = None;
    let mut owner_cap = None;
    let mut provider_cap = None;
    let mut format = T1ScreenFormat::Text;
    while parser.peek().is_some() {
        let flag = parser.peek().expect("checked above").to_owned();
        match flag.as_str() {
            "--campaign" => {
                campaign_id = Some(parse_t1_screen_campaign_id(
                    parser.value_once("--campaign")?,
                )?);
            }
            "--capabilities" => {
                let path = PathBuf::from(parser.value_once("--capabilities")?);
                validate_repository_path(&path, "T1 capability snapshot")?;
                capabilities = Some(path);
            }
            "--exam" => {
                let path = PathBuf::from(parser.value_once("--exam")?);
                validate_repository_path(&path, "T1 exam root")?;
                exam = Some(path);
            }
            "--judge-cap-millionths" => {
                owner_cap = Some(parse_positive_number(
                    parser.value_once("--judge-cap-millionths")?,
                    "judge cap millionths",
                )?);
            }
            "--provider-cap-millionths" => {
                provider_cap = Some(parse_positive_number(
                    parser.value_once("--provider-cap-millionths")?,
                    "provider cap millionths",
                )?);
            }
            "--run-id-file" => {
                let path = PathBuf::from(parser.value_once("--run-id-file")?);
                validate_output_path(&path, "run-id file")?;
                RUN_ID_FILE.with(|slot| *slot.borrow_mut() = Some(path));
            }
            "--format" => format = parse_t1_screen_format(parser.value_once("--format")?)?,
            _ if parser.take_common()? => {}
            _ => break,
        }
    }
    let owner_cap =
        owner_cap.ok_or_else(|| invalid("t1-screen-start requires --judge-cap-millionths"))?;
    let provider_cap = provider_cap
        .ok_or_else(|| invalid("t1-screen-start requires --provider-cap-millionths"))?;
    if provider_cap > owner_cap {
        return Err(invalid(
            "provider cap millionths must not exceed judge cap millionths",
        ));
    }
    Ok(CliCommand::T1ScreenStart {
        request: T1ScreenStartRequest {
            campaign_id: campaign_id
                .ok_or_else(|| invalid("t1-screen-start requires --campaign"))?,
            capabilities: capabilities
                .ok_or_else(|| invalid("t1-screen-start requires --capabilities"))?,
            exam: exam.ok_or_else(|| invalid("t1-screen-start requires --exam"))?,
            owner_approved_judge_cap_millionths_of_dollar: owner_cap,
            provider_enforced_judge_cap_millionths_of_dollar: provider_cap,
        },
        format,
    })
}

fn parse_t1_screen_run(
    parser: &mut ArgumentParser<'_>,
    is_resume: bool,
) -> Result<CliCommand, SkillEvalError> {
    let mut run_id = None;
    let mut format = T1ScreenFormat::Text;
    while parser.peek().is_some() {
        let flag = parser.peek().expect("checked above").to_owned();
        match flag.as_str() {
            "--run" => {
                run_id = Some(parse_t1_screen_run_id(parser.value_once("--run")?)?);
            }
            "--format" => format = parse_t1_screen_format(parser.value_once("--format")?)?,
            _ if parser.take_common()? => {}
            _ => break,
        }
    }
    let run_id = run_id.ok_or_else(|| {
        invalid(if is_resume {
            "t1-screen-resume requires --run"
        } else {
            "t1-screen-report requires --run"
        })
    })?;
    Ok(if is_resume {
        CliCommand::T1ScreenResume { run_id, format }
    } else {
        CliCommand::T1ScreenReport { run_id, format }
    })
}

fn parse_t1_screen_extend_cap(
    parser: &mut ArgumentParser<'_>,
) -> Result<CliCommand, SkillEvalError> {
    let mut run_id = None;
    let mut owner_cap = None;
    let mut provider_cap = None;
    let mut owner_reason = None;
    let mut format = T1ScreenFormat::Text;
    while parser.peek().is_some() {
        let flag = parser.peek().expect("checked above").to_owned();
        match flag.as_str() {
            "--run" => run_id = Some(parse_t1_screen_run_id(parser.value_once("--run")?)?),
            "--judge-cap-millionths" => {
                owner_cap = Some(parse_positive_number(
                    parser.value_once("--judge-cap-millionths")?,
                    "judge cap millionths",
                )?);
            }
            "--provider-cap-millionths" => {
                provider_cap = Some(parse_positive_number(
                    parser.value_once("--provider-cap-millionths")?,
                    "provider cap millionths",
                )?);
            }
            "--reason" => {
                owner_reason =
                    Some(nonempty(parser.value_once("--reason")?, "owner reason")?.to_owned());
            }
            "--format" => format = parse_t1_screen_format(parser.value_once("--format")?)?,
            _ if parser.take_common()? => {}
            _ => break,
        }
    }
    let run_id = run_id.ok_or_else(|| invalid("t1-screen-extend-cap requires --run"))?;
    let owner_cap =
        owner_cap.ok_or_else(|| invalid("t1-screen-extend-cap requires --judge-cap-millionths"))?;
    let provider_cap = provider_cap
        .ok_or_else(|| invalid("t1-screen-extend-cap requires --provider-cap-millionths"))?;
    if provider_cap > owner_cap {
        return Err(invalid(
            "provider cap millionths must not exceed judge cap millionths",
        ));
    }
    Ok(CliCommand::T1ScreenExtendCap {
        request: T1ScreenCapExtensionRequest {
            run_id,
            new_owner_cap_millionths_of_dollar: owner_cap,
            new_provider_cap_millionths_of_dollar: provider_cap,
            owner_reason: owner_reason
                .ok_or_else(|| invalid("t1-screen-extend-cap requires --reason"))?,
        },
        format,
    })
}

fn parse_t1_screen_fail_route(
    parser: &mut ArgumentParser<'_>,
) -> Result<CliCommand, SkillEvalError> {
    let mut run_id = None;
    let mut child_run_id = None;
    let mut owner_reason = None;
    let mut format = T1ScreenFormat::Text;
    while parser.peek().is_some() {
        let flag = parser.peek().expect("checked above").to_owned();
        match flag.as_str() {
            "--run" => run_id = Some(parse_t1_screen_run_id(parser.value_once("--run")?)?),
            "--child" => {
                child_run_id = Some(parse_t1_screen_child_run_id(parser.value_once("--child")?)?)
            }
            "--reason" => {
                owner_reason = Some(
                    nonempty(parser.value_once("--reason")?, "route failure owner reason")?
                        .to_owned(),
                );
            }
            "--format" => format = parse_t1_screen_format(parser.value_once("--format")?)?,
            _ if parser.take_common()? => {}
            _ => break,
        }
    }
    Ok(CliCommand::T1ScreenFailRoute {
        request: T1ScreenRouteFailureRequest {
            run_id: run_id.ok_or_else(|| invalid("t1-screen-fail-route requires --run"))?,
            child_run_id: child_run_id
                .ok_or_else(|| invalid("t1-screen-fail-route requires --child"))?,
            owner_reason: owner_reason
                .ok_or_else(|| invalid("t1-screen-fail-route requires --reason"))?,
        },
        format,
    })
}

fn parse_t1_screen_format(value: &str) -> Result<T1ScreenFormat, SkillEvalError> {
    match value {
        "text" => Ok(T1ScreenFormat::Text),
        "json" => Ok(T1ScreenFormat::Json),
        value => Err(invalid(format!("unknown T1 screen format {value:?}"))),
    }
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

fn parse_pool_qualify(parser: &mut ArgumentParser<'_>) -> Result<CliCommand, SkillEvalError> {
    let mut plan_path = None;
    let mut artifact_roots = Vec::new();
    let mut selected_tiers = Vec::new();
    let mut is_dry_run = false;

    while parser.peek().is_some() {
        let flag = parser.peek().expect("checked above").to_owned();
        match flag.as_str() {
            "--plan" => {
                let path = PathBuf::from(parser.value_once("--plan")?);
                validate_repository_path(&path, "pool plan")?;
                plan_path = Some(path);
            }
            "--artifact" => {
                let path = PathBuf::from(parser.value()?);
                validate_repository_path(&path, "pool artifact")?;
                artifact_roots.push(path);
            }
            "--tiers" => selected_tiers.push(parse_tier(parser.value()?)?),
            "--dry-run" => {
                parser.take_once("--dry-run")?;
                is_dry_run = true;
            }
            "--run-id-file" => {
                let path = PathBuf::from(parser.value_once("--run-id-file")?);
                validate_output_path(&path, "run-id file")?;
                RUN_ID_FILE.with(|slot| *slot.borrow_mut() = Some(path));
            }
            _ if parser.take_common()? => {}
            _ => break,
        }
    }

    if artifact_roots.is_empty() {
        return Err(invalid("pool-qualify requires at least one --artifact"));
    }
    ensure_unique_paths(&artifact_roots)?;
    if selected_tiers.is_empty() {
        selected_tiers = vec![Tier::T1, Tier::T2, Tier::T3, Tier::T4, Tier::T5];
    }
    ensure_unique_tiers(&selected_tiers)?;

    Ok(CliCommand::PoolQualify {
        request: PoolQualifyRequest {
            plan_path: plan_path.ok_or_else(|| invalid("pool-qualify requires --plan"))?,
            artifact_roots,
            selected_tiers,
            is_dry_run,
        },
    })
}

fn parse_pool_replacement(parser: &mut ArgumentParser<'_>) -> Result<CliCommand, SkillEvalError> {
    let mut run_id = None;
    let mut entrant_index = None;
    while parser.peek().is_some() {
        let flag = parser.peek().expect("checked above").to_owned();
        match flag.as_str() {
            "--run" => run_id = Some(parse_pool_run_id(parser.value_once("--run")?)?),
            "--entrant-index" => {
                entrant_index = Some(parse_number(
                    parser.value_once("--entrant-index")?,
                    "entrant index",
                )?)
            }
            "--run-id-file" => {
                let path = PathBuf::from(parser.value_once("--run-id-file")?);
                validate_output_path(&path, "run-id file")?;
                RUN_ID_FILE.with(|slot| *slot.borrow_mut() = Some(path));
            }
            _ if parser.take_common()? => {}
            _ => break,
        }
    }
    Ok(CliCommand::PoolReplacement {
        run_id: run_id.ok_or_else(|| invalid("pool-replacement requires --run"))?,
        entrant_index: entrant_index
            .ok_or_else(|| invalid("pool-replacement requires --entrant-index"))?,
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

fn parse_pool_run_only(parser: &mut ArgumentParser<'_>) -> Result<PoolRunId, SkillEvalError> {
    let mut run_id = None;
    while parser.peek().is_some() {
        let flag = parser.peek().expect("checked above").to_owned();
        match flag.as_str() {
            "--run" => run_id = Some(parse_pool_run_id(parser.value_once("--run")?)?),
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
    let mut route_index = None;
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
            "--route-index" => {
                route_index = Some(parse_number(
                    parser.value_once("--route-index")?,
                    "route index",
                )?)
            }
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
            route_index: route_index.ok_or_else(|| invalid("inspect requires --route-index"))?,
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
    runtime: &mut dyn PoolRuntime,
    output: &mut dyn Write,
) -> Result<(), SkillEvalError> {
    let format = request.output_format;
    match request.command {
        CliCommand::ModelCapabilities { output: path } => run_model_capabilities(&path, output),
        CliCommand::T1ScreenPreview {
            capabilities,
            format,
        } => run_t1_screen_preview(&capabilities, format, output),
        CliCommand::T1ScreenCampaignCreate { .. }
        | CliCommand::T1ScreenCampaignExtendCap { .. }
        | CliCommand::T1ScreenCampaignRetireRun { .. }
        | CliCommand::T1ScreenStart { .. }
        | CliCommand::T1ScreenResume { .. }
        | CliCommand::T1ScreenExtendCap { .. }
        | CliCommand::T1ScreenFailRoute { .. }
        | CliCommand::T1ScreenReport { .. } => Err(invalid(
            "T1 screening commands require the dedicated T1 runtime",
        )),
        CliCommand::FrontierSuiteInventory { .. }
        | CliCommand::FrontierSuitePropose { .. }
        | CliCommand::FrontierSuiteCheck { .. }
        | CliCommand::FrontierSuiteApply { .. } => Err(invalid(
            "complete-bank suite commands require the dedicated suite runtime",
        )),
        CliCommand::FrontierPreview { .. }
        | CliCommand::FrontierStart { .. }
        | CliCommand::FrontierResume { .. }
        | CliCommand::FrontierReport { .. }
        | CliCommand::FrontierInspect { .. }
        | CliCommand::FrontierDecide { .. }
        | CliCommand::FrontierApply { .. } => Err(invalid(
            "frontier commands require the dedicated frontier runtime",
        )),
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
        CliCommand::PoolQualify { request } => {
            let mut progress = RenderPoolProgress {
                format,
                output,
                is_run_id_written: false,
            };
            let state = start_pool_qualification(request, runtime, &mut progress)?;
            render_pool_report(&state, format, progress.output)
        }
        CliCommand::PoolReport { run_id } => {
            let state = build_pool_report(&run_id, runtime)?;
            render_pool_report(&state, format, output)
        }
        CliCommand::PoolResume { run_id } => {
            let mut progress = RenderPoolProgress {
                format,
                output,
                is_run_id_written: false,
            };
            let state = resume_pool_qualification(&run_id, runtime, &mut progress)?;
            render_pool_report(&state, format, progress.output)
        }
        CliCommand::PoolReplacement {
            run_id,
            entrant_index,
        } => {
            let mut progress = RenderProgress { format, output };
            let report = start_pool_replacement_qualification(
                &run_id,
                entrant_index,
                runtime,
                &mut progress,
            )?;
            write_run_id_file(&report.run_id)?;
            render_report(&report, format, progress.output)
        }
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
        RunEvent::PoolChildCompleted { artifact, tier, .. } => {
            format!("{} {:?} pool child complete", artifact.0, tier)
        }
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

const RESULT_MATRIX_LEVELS: [&str; 7] = ["off", "minimal", "low", "medium", "high", "xhigh", "max"];
type ResultMatrixCells = [Option<bool>; RESULT_MATRIX_LEVELS.len()];

fn render_pool_tier_matrix(
    entrants: &[PoolEntrant],
    pool: Option<&crate::model::RankedPool>,
    output: &mut dyn Write,
) -> Result<(), SkillEvalError> {
    let rows = pool_tier_matrix_rows(entrants, pool)?;
    render_result_matrix(&rows, output)
}

fn pool_tier_matrix_rows(
    entrants: &[PoolEntrant],
    pool: Option<&crate::model::RankedPool>,
) -> Result<Vec<(String, ResultMatrixCells)>, SkillEvalError> {
    let tier = entrants
        .first()
        .map(|entrant| entrant.model.tier)
        .ok_or_else(|| malformed_pool_report("matrix entrants are empty"))?;
    let mut configured = BTreeMap::new();
    for (entrant_index, entrant) in entrants.iter().enumerate() {
        if entrant.model.provider.trim().is_empty()
            || entrant.model.model.trim().is_empty()
            || entrant.thinking_levels.is_empty()
        {
            return Err(malformed_pool_report("matrix entrant is empty"));
        }
        if entrant.model.tier != tier {
            return Err(malformed_pool_report("matrix entrants span tiers"));
        }
        let base = (
            entrant.model.provider.as_str(),
            entrant.model.model.as_str(),
        );
        if configured.insert(base, entrant_index).is_some() {
            return Err(malformed_pool_report(
                "matrix entrants contain a duplicate base model",
            ));
        }
        let mut levels = BTreeSet::new();
        for level in &entrant.thinking_levels {
            thinking_level_index(level)?;
            if !levels.insert(level) {
                return Err(malformed_pool_report(
                    "matrix entrant contains a duplicate thinking level",
                ));
            }
        }
        thinking_level_index(&entrant.model.thinking)?;
        if !levels.contains(&entrant.model.thinking) {
            return Err(malformed_pool_report(
                "matrix entrant start thinking is not configured",
            ));
        }
    }
    if pool.is_some_and(|pool| pool.tier != tier) {
        return Err(malformed_pool_report("matrix pool tier differs"));
    }

    let mut calibration = vec![[None; RESULT_MATRIX_LEVELS.len()]; entrants.len()];
    let mut qualification = calibration.clone();
    let mut seen = BTreeSet::new();
    if let Some(pool) = pool {
        collect_pool_matrix_evidence(
            entrants,
            &configured,
            &pool.calibration,
            crate::model::PoolStage::Calibration,
            &mut calibration,
            &mut seen,
        )?;
        collect_pool_matrix_evidence(
            entrants,
            &configured,
            &pool.qualification,
            crate::model::PoolStage::Qualification,
            &mut qualification,
            &mut seen,
        )?;
    }

    Ok(entrants
        .iter()
        .enumerate()
        .map(|(entrant_index, entrant)| {
            let cells = std::array::from_fn(|level_index| {
                qualification[entrant_index][level_index]
                    .or(calibration[entrant_index][level_index])
            });
            (
                format!("{}/{}", entrant.model.provider, entrant.model.model),
                cells,
            )
        })
        .collect())
}

fn collect_pool_matrix_evidence<'a>(
    entrants: &[PoolEntrant],
    configured: &BTreeMap<(&'a str, &'a str), usize>,
    evidence: &[PoolEntrantEvidence],
    stage: crate::model::PoolStage,
    cells: &mut [ResultMatrixCells],
    seen: &mut BTreeSet<(crate::model::PoolStage, usize, usize)>,
) -> Result<(), SkillEvalError> {
    for item in evidence {
        if item.stage != stage {
            return Err(malformed_pool_report(
                "matrix evidence appears in the wrong stage",
            ));
        }
        let base = (
            item.requested_model.provider.as_str(),
            item.requested_model.model.as_str(),
        );
        let entrant_index = configured.get(&base).copied().ok_or_else(|| {
            malformed_pool_report("matrix evidence belongs to a foreign base model")
        })?;
        let entrant = &entrants[entrant_index];
        if item.requested_model.tier != entrant.model.tier
            || item.effective_model != item.requested_model
        {
            return Err(malformed_pool_report(
                "matrix evidence identity differs from its configured entrant",
            ));
        }
        let level_index = thinking_level_index(&item.requested_model.thinking)?;
        if !entrant
            .thinking_levels
            .contains(&item.requested_model.thinking)
        {
            return Err(malformed_pool_report(
                "matrix evidence uses an unconfigured thinking level",
            ));
        }
        if !seen.insert((stage, entrant_index, level_index)) {
            let message = if cells[entrant_index][level_index]
                .is_some_and(|is_passing| is_passing != item.is_passing)
            {
                "matrix evidence conflicts within one stage"
            } else {
                "matrix evidence contains a duplicate configuration"
            };
            return Err(malformed_pool_report(message));
        }
        if item.expected_trials > 0 && item.completed_trials == item.expected_trials {
            cells[entrant_index][level_index] = Some(item.is_passing);
        }
    }
    Ok(())
}

fn thinking_level_index(level: &str) -> Result<usize, SkillEvalError> {
    RESULT_MATRIX_LEVELS
        .iter()
        .position(|candidate| *candidate == level)
        .ok_or_else(|| malformed_pool_report("matrix uses an unknown thinking level"))
}

fn render_result_matrix(
    rows: &[(String, ResultMatrixCells)],
    output: &mut dyn Write,
) -> Result<(), SkillEvalError> {
    writeln!(
        output,
        "| Model | off | minimal | low | medium | high | xhigh | max |"
    )
    .map_err(output_error)?;
    writeln!(output, "| --- | --- | --- | --- | --- | --- | --- | --- |").map_err(output_error)?;
    for (model, cells) in rows {
        write!(output, "| {model} |").map_err(output_error)?;
        for cell in cells {
            write!(
                output,
                " {} |",
                cell.map_or("", |is_passing| if is_passing { "P" } else { "F" })
            )
            .map_err(output_error)?;
        }
        writeln!(output).map_err(output_error)?;
    }
    writeln!(output).map_err(output_error)
}

pub(crate) fn render_pool_report(
    state: &PoolRunState,
    format: OutputFormat,
    output: &mut dyn Write,
) -> Result<(), SkillEvalError> {
    validate_pool_report_state(state)?;
    if format == OutputFormat::JsonLines {
        return write_json_line(state, output);
    }

    for tier in &state.selected_tiers {
        let entrants = selected_tier_entrants(state, *tier)?;
        let pool = state.pools.iter().find(|pool| pool.tier == *tier);
        render_pool_tier_matrix(entrants, pool, output)?;
    }

    writeln!(
        output,
        "pool {}: {:?}",
        state.configuration.run_id.0, state.status
    )
    .map_err(output_error)?;
    writeln!(
        output,
        "selected tiers: {}",
        state
            .selected_tiers
            .iter()
            .map(|tier| tier_label(*tier))
            .collect::<Vec<_>>()
            .join(", ")
    )
    .map_err(output_error)?;
    writeln!(output, "created: {}", state.configuration.created_at.0).map_err(output_error)?;
    writeln!(
        output,
        "floors: score >= {}, calibration reliability >= {:.2}%, qualification reliability >= {:.2}%",
        state.configuration.policy.minimum_score,
        f64::from(
            state
                .configuration
                .policy
                .calibration_minimum_reliability_basis_points
        ) / 100.0,
        f64::from(
            state
                .configuration
                .policy
                .qualification_minimum_reliability_basis_points
        ) / 100.0
    )
    .map_err(output_error)?;
    writeln!(
        output,
        "candidate spending: ${:.6} spent / ${:.6} limit; provider limit enforced: {}",
        dollars(state.spent_millionths_of_dollar),
        dollars(
            state
                .configuration
                .policy
                .spending_limit_millionths_of_dollar
        ),
        state.configuration.policy.is_provider_limit_enforced
    )
    .map_err(output_error)?;
    writeln!(
        output,
        "control excluded: {}",
        model_label(&state.configuration.control)
    )
    .map_err(output_error)?;

    for artifact in &state.configuration.artifacts {
        writeln!(
            output,
            "artifact: {} revision {}",
            artifact.name.0, artifact.revision
        )
        .map_err(output_error)?;
    }
    for tier in &state.selected_tiers {
        let entrants = selected_tier_entrants(state, *tier)?;
        for (index, entrant) in entrants.iter().enumerate() {
            writeln!(
                output,
                "{} entrant {}: exact candidate host {} catalog {}; ordered thinking levels [{}]; starting thinking {}",
                tier_label(*tier),
                index + 1,
                model_label(&entrant.model),
                entrant.catalog_observed_at.0,
                entrant.thinking_levels.join(", "),
                entrant.model.thinking,
            )
            .map_err(output_error)?;
        }
    }
    for child in &state.child_runs {
        let candidate =
            pool_child_model(state, child.tier, child.entrant_index, child.thinking_index)?;
        writeln!(
            output,
            "child {}: {} entrant {} thinking index {} {:?} {:?}; exact requested {}",
            child.run_id.0,
            tier_label(child.tier),
            child.entrant_index + 1,
            child.thinking_index,
            child.stage,
            child.status,
            model_label(&candidate)
        )
        .map_err(output_error)?;
    }
    for tier in &state.selected_tiers {
        render_tier_pool(state, *tier, output)?;
    }

    if let Some(pause) = &state.pause {
        writeln!(output, "pause reason: {pause:?}").map_err(output_error)?;
        writeln!(
            output,
            "resume: skill-eval pool-resume --run {}",
            state.configuration.run_id.0
        )
        .map_err(output_error)?;
    }
    let is_result_ready = state.status == PoolRunStatus::AwaitingDecision
        && state.selected_tiers.iter().all(|tier| {
            state
                .pools
                .iter()
                .any(|pool| pool.tier == *tier && pool.is_complete)
        });
    writeln!(
        output,
        "owner state: {}",
        if is_result_ready {
            "result-ready"
        } else {
            "not-result-ready"
        }
    )
    .map_err(output_error)
}

fn selected_tier_entrants(
    state: &PoolRunState,
    tier: Tier,
) -> Result<&[PoolEntrant], SkillEvalError> {
    let entrants = state
        .configuration
        .entrants
        .get(&tier)
        .ok_or_else(|| malformed_pool_report("selected tier has no configured entrants"))?;
    if entrants.len() < 3 {
        return Err(malformed_pool_report(
            "selected tier must contain at least three entrants",
        ));
    }
    Ok(entrants)
}

fn validate_pool_report_state(state: &PoolRunState) -> Result<(), SkillEvalError> {
    if state.selected_tiers.iter().collect::<BTreeSet<_>>().len() != state.selected_tiers.len() {
        return Err(malformed_pool_report("selected tiers contain a duplicate"));
    }
    for child in &state.child_runs {
        pool_child_model(state, child.tier, child.entrant_index, child.thinking_index)?;
    }
    for pool in &state.pools {
        if !state.selected_tiers.contains(&pool.tier)
            || state
                .pools
                .iter()
                .filter(|candidate| candidate.tier == pool.tier)
                .count()
                != 1
        {
            return Err(malformed_pool_report(
                "pool report contains an unselected or duplicate tier pool",
            ));
        }
    }
    for tier in &state.selected_tiers {
        let entrants = selected_tier_entrants(state, *tier)?;
        let pool = state.pools.iter().find(|pool| pool.tier == *tier);
        pool_tier_matrix_rows(entrants, pool)?;
        let calibration = pool.map_or(&[][..], |pool| pool.calibration.as_slice());
        for evidence in calibration {
            if entrants
                .iter()
                .filter(|entrant| is_same_base_model(&entrant.model, &evidence.requested_model))
                .count()
                != 1
            {
                return Err(malformed_pool_report(
                    "calibration evidence does not belong to one configured base model",
                ));
            }
        }
        for entrant in entrants {
            let evidence = calibration
                .iter()
                .filter(|item| is_same_base_model(&item.requested_model, &entrant.model))
                .cloned()
                .collect::<Vec<_>>();
            let decision = select_thinking_level(entrant, &evidence)?;
            validate_persisted_thinking_selection(pool, entrant, &decision.selected)?;
        }
        if let Some(pool) = pool {
            for selection in &pool.thinking_selections {
                if entrants
                    .iter()
                    .filter(|entrant| is_same_base_model(&entrant.model, selection))
                    .count()
                    != 1
                {
                    return Err(malformed_pool_report(
                        "thinking selection does not belong to one configured base model",
                    ));
                }
            }
        }
    }
    Ok(())
}

fn render_tier_pool(
    state: &PoolRunState,
    tier: Tier,
    output: &mut dyn Write,
) -> Result<(), SkillEvalError> {
    let entrants = selected_tier_entrants(state, tier)?;
    let pool = state.pools.iter().find(|pool| pool.tier == tier);
    let calibration = pool.map_or(&[][..], |pool| pool.calibration.as_slice());
    let qualification = pool.map_or(&[][..], |pool| pool.qualification.as_slice());
    let is_complete = pool.is_some_and(|pool| pool.is_complete);

    for evidence in calibration {
        if entrants
            .iter()
            .filter(|entrant| is_same_base_model(&entrant.model, &evidence.requested_model))
            .count()
            != 1
        {
            return Err(malformed_pool_report(
                "calibration evidence does not belong to one configured base model",
            ));
        }
    }

    writeln!(
        output,
        "{} calibration: {}/{} passing; full qualification: {}/{} passing; complete: {}",
        tier_label(tier),
        calibration.iter().filter(|item| item.is_passing).count(),
        calibration.len(),
        qualification.iter().filter(|item| item.is_passing).count(),
        qualification.len(),
        is_complete
    )
    .map_err(output_error)?;

    for (entrant_index, entrant) in entrants.iter().enumerate() {
        let evidence = calibration
            .iter()
            .filter(|item| is_same_base_model(&item.requested_model, &entrant.model))
            .cloned()
            .collect::<Vec<_>>();
        let decision = select_thinking_level(entrant, &evidence)?;
        validate_persisted_thinking_selection(pool, entrant, &decision.selected)?;
        render_thinking_progress(
            state,
            tier,
            entrant_index,
            entrant,
            &evidence,
            &decision,
            output,
        )?;
    }

    for evidence in qualification {
        render_pool_evidence("full qualification", evidence, output)?;
    }
    let thinking_selections = pool.map_or(&[][..], |pool| pool.thinking_selections.as_slice());
    let promoted = pool.map_or(&[][..], |pool| pool.promoted.as_slice());
    let ranked = pool.map_or(&[][..], |pool| pool.ranked.as_slice());
    let retained_lower_routes = pool.map_or(&[][..], |pool| pool.retained_lower_routes.as_slice());
    writeln!(
        output,
        "{} thinking selections: {}",
        tier_label(tier),
        model_list(thinking_selections)
    )
    .map_err(output_error)?;
    writeln!(
        output,
        "{} promoted routes: {}",
        tier_label(tier),
        model_list(promoted)
    )
    .map_err(output_error)?;
    writeln!(
        output,
        "{} ranked order: {}",
        tier_label(tier),
        model_list(ranked)
    )
    .map_err(output_error)?;
    if retained_lower_routes.is_empty() {
        return Ok(());
    }
    writeln!(
        output,
        "{} retained lower routes: {}",
        tier_label(tier),
        model_list(retained_lower_routes)
    )
    .map_err(output_error)
}

fn validate_persisted_thinking_selection(
    pool: Option<&crate::model::RankedPool>,
    entrant: &PoolEntrant,
    derived: &Option<ModelIdentity>,
) -> Result<(), SkillEvalError> {
    let mut selections = pool
        .into_iter()
        .flat_map(|pool| &pool.thinking_selections)
        .filter(|selection| is_same_base_model(selection, &entrant.model));
    let persisted = selections.next();
    if selections.next().is_some() {
        return Err(malformed_pool_report(
            "thinking selections contain a duplicate base model",
        ));
    }
    if persisted.is_some_and(|selection| Some(selection) != derived.as_ref()) {
        return Err(malformed_pool_report(
            "persisted thinking selection differs from calibration evidence",
        ));
    }
    Ok(())
}

fn render_thinking_progress(
    state: &PoolRunState,
    tier: Tier,
    entrant_index: usize,
    entrant: &PoolEntrant,
    evidence: &[PoolEntrantEvidence],
    decision: &crate::model::ThinkingDecision,
    output: &mut dyn Write,
) -> Result<(), SkillEvalError> {
    writeln!(
        output,
        "  {} entrant {} thinking: exact base {}/{}; ordered levels [{}]; start {}",
        tier_label(tier),
        entrant_index + 1,
        entrant.model.provider,
        entrant.model.model,
        entrant.thinking_levels.join(", "),
        entrant.model.thinking
    )
    .map_err(output_error)?;
    if evidence.is_empty() {
        writeln!(output, "    calibration attempts: none").map_err(output_error)?;
    }
    for (attempt_index, item) in evidence.iter().enumerate() {
        let thinking_index = entrant
            .thinking_levels
            .iter()
            .position(|level| level == &item.requested_model.thinking)
            .ok_or_else(|| malformed_pool_report("calibration evidence uses an unknown level"))?;
        render_pool_evidence(
            &format!(
                "thinking attempt {} index {} {}",
                attempt_index + 1,
                thinking_index,
                item.requested_model.thinking
            ),
            item,
            output,
        )?;
    }
    if decision.is_complete {
        match &decision.selected {
            Some(selected) => writeln!(
                output,
                "    thinking result: selected lowest passing exact identity {}",
                model_label(selected)
            )
            .map_err(output_error)?,
            None => writeln!(output, "    thinking result: complete, all levels failed")
                .map_err(output_error)?,
        }
    } else {
        let next_index = decision.next_thinking_index.ok_or_else(|| {
            malformed_pool_report("incomplete thinking decision has no next probe")
        })?;
        let next = pool_thinking_model(entrant, next_index)?;
        writeln!(
            output,
            "    next thinking probe: index {} exact requested {}",
            next_index,
            model_label(&next)
        )
        .map_err(output_error)?;
    }
    let skipped = state
        .child_runs
        .iter()
        .filter(|child| {
            child.tier == tier
                && usize::from(child.entrant_index) == entrant_index
                && child.stage == crate::model::PoolStage::Calibration
                && child.status == PoolChildStatus::Skipped
        })
        .map(|child| {
            entrant
                .thinking_levels
                .get(usize::from(child.thinking_index))
                .cloned()
                .ok_or_else(|| malformed_pool_report("skipped child has an unknown thinking index"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    writeln!(
        output,
        "    skipped calibration levels: {}",
        if skipped.is_empty() {
            "none".to_owned()
        } else {
            skipped.join(", ")
        }
    )
    .map_err(output_error)
}

fn pool_child_model(
    state: &PoolRunState,
    tier: Tier,
    entrant_index: u8,
    thinking_index: u8,
) -> Result<ModelIdentity, SkillEvalError> {
    let entrant = selected_tier_entrants(state, tier)?
        .get(usize::from(entrant_index))
        .ok_or_else(|| malformed_pool_report("child has an unknown entrant index"))?;
    pool_thinking_model(entrant, thinking_index)
}

fn pool_thinking_model(
    entrant: &PoolEntrant,
    thinking_index: u8,
) -> Result<ModelIdentity, SkillEvalError> {
    let thinking = entrant
        .thinking_levels
        .get(usize::from(thinking_index))
        .ok_or_else(|| malformed_pool_report("child has an unknown thinking index"))?;
    let mut model = entrant.model.clone();
    model.thinking.clone_from(thinking);
    Ok(model)
}

fn is_same_base_model(left: &ModelIdentity, right: &ModelIdentity) -> bool {
    left.tier == right.tier && left.provider == right.provider && left.model == right.model
}

fn render_pool_evidence(
    label: &str,
    evidence: &PoolEntrantEvidence,
    output: &mut dyn Write,
) -> Result<(), SkillEvalError> {
    let failure_rate = if evidence.completed_trials == 0 {
        0.0
    } else {
        f64::from(evidence.failed_trials) * 100.0 / f64::from(evidence.completed_trials)
    };
    writeln!(
        output,
        "    {label}: exact candidate {}; judge host {}; result {}; score {:.2} [{:.2}, {:.2}]; trials {}/{}; failures {} ({failure_rate:.2}%); catastrophic {}; candidate usage {} input, {} output, ${:.6}, {} ms; judge usage {} input, {} output, ${:.6}, {} ms",
        model_label(&evidence.effective_model),
        model_label(&evidence.judge_model),
        if evidence.is_passing { "pass" } else { "fail" },
        evidence.score.estimate,
        evidence.score.lower,
        evidence.score.upper,
        evidence.completed_trials,
        evidence.expected_trials,
        evidence.failed_trials,
        evidence.catastrophic_trials,
        evidence.candidate_usage.input_tokens,
        evidence.candidate_usage.output_tokens,
        dollars(evidence.candidate_usage.cost_millionths_of_dollar),
        evidence.candidate_usage.elapsed_milliseconds,
        evidence.judge_usage.input_tokens,
        evidence.judge_usage.output_tokens,
        dollars(evidence.judge_usage.cost_millionths_of_dollar),
        evidence.judge_usage.elapsed_milliseconds,
    )
    .map_err(output_error)
}

fn malformed_pool_report(message: impl Into<String>) -> SkillEvalError {
    SkillEvalError::InvalidConfiguration(format!("malformed pool report: {}", message.into()))
}

fn model_label(model: &ModelIdentity) -> String {
    format!(
        "{}/{} ({}; {})",
        model.provider,
        model.model,
        tier_label(model.tier),
        model.thinking
    )
}

fn model_list(models: &[ModelIdentity]) -> String {
    if models.is_empty() {
        return "none".to_owned();
    }
    models
        .iter()
        .enumerate()
        .map(|(index, model)| format!("{}. {}", index + 1, model_label(model)))
        .collect::<Vec<_>>()
        .join("; ")
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

struct RenderFrontierProgress<'a> {
    format: OutputFormat,
    output: &'a mut dyn Write,
}

impl FrontierProgressSink for RenderFrontierProgress<'_> {
    fn emit_frontier(&mut self, state: &FrontierRunState) -> Result<(), SkillEvalError> {
        match self.format {
            OutputFormat::JsonLines => write_json_line(state, self.output),
            OutputFormat::Text => writeln!(
                self.output,
                "frontier {}: {:?}, spent {} millionths",
                state.configuration.run_id.0, state.status, state.spent_millionths_of_dollar,
            )
            .map_err(output_error),
        }
    }
}

struct RenderPoolProgress<'a> {
    format: OutputFormat,
    output: &'a mut dyn Write,
    is_run_id_written: bool,
}

impl PoolProgressSink for RenderPoolProgress<'_> {
    fn emit_pool(&mut self, state: &PoolRunState) -> Result<(), SkillEvalError> {
        if !self.is_run_id_written {
            write_run_id_value(&state.configuration.run_id.0)?;
            self.is_run_id_written = true;
        }
        render_pool_report(state, self.format, self.output)
    }
}

struct FixedClock(Timestamp);

impl Clock for FixedClock {
    fn now(&self) -> Timestamp {
        self.0.clone()
    }
}

const CANDIDATE_EXTENSION_IDENTITY_SIZE_LIMIT: u64 = 64 * 1024 * 1024;
const MISSING_NODE_MODULES_LOCK_MARKER: &[u8] = b"missing";

fn candidate_environment_manifest() -> Result<Vec<CandidateEnvironmentEntry>, SkillEvalError> {
    let home = env::var_os("HOME").ok_or_else(|| {
        SkillEvalError::InvalidConfiguration(
            "HOME is required to identify the candidate Pi environment".to_owned(),
        )
    })?;
    candidate_environment_manifest_at(&PathBuf::from(home).join(".pi/agent"))
}

fn candidate_environment_manifest_at(
    agent_root: &Path,
) -> Result<Vec<CandidateEnvironmentEntry>, SkillEvalError> {
    let mut manifest = Vec::new();
    for name in ["settings.json", "models.json"] {
        let path = agent_root.join(name);
        match fs::metadata(&path) {
            Ok(metadata) if metadata.is_file() => {
                let bytes = read_candidate_file(&path)?;
                manifest.push(CandidateEnvironmentEntry {
                    key: format!("pi-agent/{name}"),
                    sha256: sha256_digest(&bytes),
                });
            }
            Ok(_) => {
                return Err(SkillEvalError::InvalidConfiguration(format!(
                    "candidate Pi input {} is not a regular file",
                    path.display()
                )));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(io_path_error(path, error)),
        }
    }

    let extensions = agent_root.join("extensions");
    let mut entries = match fs::read_dir(&extensions) {
        Ok(entries) => entries
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| io_path_error(extensions.clone(), error))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(io_path_error(extensions, error)),
    };
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        let name = utf8_path_segment(&entry.file_name(), "candidate extension name")?;
        let key_root = format!("extensions/{name}");
        let path = entry.path();
        let canonical =
            fs::canonicalize(&path).map_err(|error| io_path_error(path.clone(), error))?;
        manifest.push(CandidateEnvironmentEntry {
            key: key_root.clone(),
            sha256: sha256_digest(name.as_bytes()),
        });
        manifest.push(CandidateEnvironmentEntry {
            key: format!("{key_root}/canonical-path"),
            sha256: sha256_digest(canonical.as_os_str().as_encoded_bytes()),
        });
        let metadata =
            fs::metadata(&canonical).map_err(|error| io_path_error(canonical.clone(), error))?;
        if metadata.is_file() {
            let mut total_bytes = 0;
            let bytes = read_bounded_extension_file(&canonical, &canonical, &mut total_bytes)?;
            manifest.push(CandidateEnvironmentEntry {
                key: format!("{key_root}/content"),
                sha256: sha256_digest(&bytes),
            });
        } else if metadata.is_dir() {
            append_extension_directory_manifest(&canonical, &key_root, &mut manifest)?;
        } else {
            return Err(SkillEvalError::InvalidConfiguration(format!(
                "candidate extension {} has an unsupported type",
                path.display()
            )));
        }
    }

    manifest.sort_by(|left, right| left.key.cmp(&right.key));
    if manifest
        .windows(2)
        .any(|entries| entries[0].key == entries[1].key)
    {
        return Err(SkillEvalError::InvalidConfiguration(
            "candidate environment manifest contains a duplicate key".to_owned(),
        ));
    }
    Ok(manifest)
}

fn append_extension_directory_manifest(
    root: &Path,
    key_root: &str,
    manifest: &mut Vec<CandidateEnvironmentEntry>,
) -> Result<(), SkillEvalError> {
    let mut pending = vec![root.to_path_buf()];
    let mut visited = BTreeSet::from([root.to_path_buf()]);
    let mut total_bytes = 0;
    while let Some(directory) = pending.pop() {
        let mut entries = fs::read_dir(&directory)
            .map_err(|error| io_path_error(directory.clone(), error))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| io_path_error(directory.clone(), error))?;
        entries.sort_by_key(fs::DirEntry::file_name);
        for entry in entries {
            let path = entry.path();
            let name = entry.file_name();
            if name == ".git" || name == "node_modules" {
                continue;
            }
            let metadata =
                fs::metadata(&path).map_err(|error| io_path_error(path.clone(), error))?;
            if metadata.is_dir() {
                let canonical =
                    fs::canonicalize(&path).map_err(|error| io_path_error(path.clone(), error))?;
                if !visited.insert(canonical) {
                    return Err(SkillEvalError::InvalidConfiguration(format!(
                        "candidate extension directory {} contains a directory cycle",
                        root.display()
                    )));
                }
                pending.push(path);
                continue;
            }
            if !metadata.is_file() {
                return Err(SkillEvalError::InvalidConfiguration(format!(
                    "candidate extension entry {} has an unsupported type",
                    path.display()
                )));
            }
            let bytes = read_bounded_extension_file(&path, root, &mut total_bytes)?;
            let relative = searchable_relative_path(root, &path)?;
            manifest.push(CandidateEnvironmentEntry {
                key: format!("{key_root}/files/{relative}"),
                sha256: sha256_digest(&bytes),
            });
        }
    }
    append_node_modules_lock_marker(root, key_root, manifest, &mut total_bytes)
}

fn append_node_modules_lock_marker(
    root: &Path,
    key_root: &str,
    manifest: &mut Vec<CandidateEnvironmentEntry>,
    total_bytes: &mut u64,
) -> Result<(), SkillEvalError> {
    let is_runtime_dependencies_present = runtime_dependencies_indicated(root)?;
    let node_modules = root.join("node_modules");
    match fs::symlink_metadata(&node_modules) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => {
            return Err(SkillEvalError::InvalidConfiguration(format!(
                "candidate extension entry {} has an unsupported type",
                node_modules.display()
            )));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if is_runtime_dependencies_present {
                manifest.push(missing_node_modules_lock_entry(key_root));
            }
            return Ok(());
        }
        Err(error) => return Err(io_path_error(node_modules, error)),
    }

    let lock = node_modules.join(".package-lock.json");
    match fs::symlink_metadata(&lock) {
        Ok(metadata) if metadata.is_file() => {
            let bytes = read_bounded_extension_file(&lock, root, total_bytes)?;
            manifest.push(CandidateEnvironmentEntry {
                key: node_modules_lock_key(key_root),
                sha256: sha256_digest(&bytes),
            });
        }
        Ok(_) => {
            return Err(SkillEvalError::InvalidConfiguration(format!(
                "candidate extension entry {} has an unsupported type",
                lock.display()
            )));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if is_runtime_dependencies_present {
                manifest.push(missing_node_modules_lock_entry(key_root));
            }
        }
        Err(error) => return Err(io_path_error(lock, error)),
    }
    Ok(())
}

fn missing_node_modules_lock_entry(key_root: &str) -> CandidateEnvironmentEntry {
    CandidateEnvironmentEntry {
        key: node_modules_lock_key(key_root),
        sha256: sha256_digest(MISSING_NODE_MODULES_LOCK_MARKER),
    }
}

fn node_modules_lock_key(key_root: &str) -> String {
    format!("{key_root}/files/node_modules/.package-lock.json")
}

fn runtime_dependencies_indicated(root: &Path) -> Result<bool, SkillEvalError> {
    let package = dependency_object_is_nonempty(&root.join("package.json"), &["dependencies"])?;
    let lock = root.join("package-lock.json");
    let lock_root = dependency_object_is_nonempty(&lock, &["packages", "", "dependencies"])?;
    let lock_v1 = dependency_object_is_nonempty(&lock, &["dependencies"])?;
    Ok(package || lock_root || lock_v1)
}

fn dependency_object_is_nonempty(path: &Path, fields: &[&str]) -> Result<bool, SkillEvalError> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(io_path_error(path.to_path_buf(), error)),
    };
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|error| {
        SkillEvalError::InvalidConfiguration(format!(
            "candidate extension dependency file {} is malformed: {error}",
            path.display()
        ))
    })?;
    let value = fields
        .iter()
        .try_fold(&value, |value, field| value.get(*field).ok_or(()));
    Ok(value
        .ok()
        .and_then(serde_json::Value::as_object)
        .is_some_and(|dependencies| !dependencies.is_empty()))
}

fn read_candidate_file(path: &Path) -> Result<Vec<u8>, SkillEvalError> {
    fs::read(path).map_err(|error| io_path_error(path.to_path_buf(), error))
}

fn read_bounded_extension_file(
    path: &Path,
    identity_root: &Path,
    total_bytes: &mut u64,
) -> Result<Vec<u8>, SkillEvalError> {
    let bytes = read_candidate_file(path)?;
    let length = u64::try_from(bytes.len()).map_err(|_| {
        SkillEvalError::InvalidConfiguration(
            "candidate extension file size exceeds the supported range".to_owned(),
        )
    })?;
    *total_bytes = total_bytes.checked_add(length).ok_or_else(|| {
        SkillEvalError::InvalidConfiguration(
            "candidate extension directory size overflowed".to_owned(),
        )
    })?;
    if *total_bytes > CANDIDATE_EXTENSION_IDENTITY_SIZE_LIMIT {
        return Err(SkillEvalError::InvalidConfiguration(format!(
            "candidate extension {} exceeds the identity size limit",
            identity_root.display()
        )));
    }
    Ok(bytes)
}

fn searchable_relative_path(root: &Path, path: &Path) -> Result<String, SkillEvalError> {
    let relative = path.strip_prefix(root).map_err(|_| {
        SkillEvalError::InvalidConfiguration("candidate extension path escaped its root".to_owned())
    })?;
    let mut components = Vec::new();
    for component in relative.components() {
        let Component::Normal(component) = component else {
            return Err(SkillEvalError::InvalidConfiguration(
                "candidate extension path is not searchable".to_owned(),
            ));
        };
        components.push(utf8_path_segment(component, "candidate extension path")?);
    }
    Ok(components.join("/"))
}

fn utf8_path_segment(segment: &std::ffi::OsStr, label: &str) -> Result<String, SkillEvalError> {
    segment
        .to_str()
        .map(str::to_owned)
        .ok_or_else(|| SkillEvalError::InvalidConfiguration(format!("{label} is not UTF-8")))
}

fn io_path_error(path: PathBuf, error: std::io::Error) -> SkillEvalError {
    SkillEvalError::Io {
        path,
        message: error.to_string(),
    }
}

struct FileSuiteRuntime<S = FileArtifactSource> {
    source: S,
    store: FileFrontierStore,
}

impl FileSuiteRuntime<FileArtifactSource> {
    fn new(repository_root: &Path) -> Result<Self, SkillEvalError> {
        Ok(Self {
            source: FileArtifactSource,
            store: FileFrontierStore::new(repository_root)?,
        })
    }
}

impl<S: ArtifactSource> ArtifactSource for FileSuiteRuntime<S> {
    fn load(&self, root: &Path) -> Result<ArtifactDefinition, SkillEvalError> {
        self.source.load(root)
    }
}

impl<S> Clock for FileSuiteRuntime<S> {
    fn now(&self) -> Timestamp {
        local_timestamp()
    }
}

fn proposal_artifact_revisions(
    proposal: &FrontierSuiteProposal,
) -> Result<BTreeMap<PathBuf, String>, SkillEvalError> {
    let mut revisions = BTreeMap::new();
    let keys = proposal
        .proposed_tiers
        .values()
        .flat_map(|tier| {
            tier.cases
                .iter()
                .map(|case| (&case.artifact_path, &case.artifact_revision))
        })
        .chain(
            proposal
                .calibration_anchors
                .iter()
                .map(|case| (&case.artifact_path, &case.artifact_revision)),
        )
        .chain(
            proposal
                .holdout_cases
                .iter()
                .map(|case| (&case.artifact_path, &case.artifact_revision)),
        );
    for (path, revision) in keys {
        match revisions.get(path) {
            Some(frozen) if frozen != revision => {
                return Err(SkillEvalError::InvalidConfiguration(format!(
                    "frontier proposal has conflicting revisions for artifact {}",
                    path.display()
                )));
            }
            Some(_) => {}
            None => {
                revisions.insert(path.clone(), revision.clone());
            }
        }
    }
    Ok(revisions)
}

fn validate_proposal_sources(
    source: &dyn ArtifactSource,
    proposal: &FrontierSuiteProposal,
) -> Result<(), SkillEvalError> {
    for (path, frozen_revision) in proposal_artifact_revisions(proposal)? {
        let current = source.load(&path)?;
        if current.revision != frozen_revision {
            return Err(SkillEvalError::InvalidConfiguration(format!(
                "frontier proposal artifact {} revision changed from {} to {}",
                path.display(),
                frozen_revision,
                current.revision
            )));
        }
    }
    Ok(())
}

fn apply_current_frontier_suite(
    source: &dyn ArtifactSource,
    store: &mut FileFrontierStore,
    proposal: &FrontierSuiteProposal,
    output: &Path,
    published_at: &Timestamp,
) -> Result<FrontierSuitePublication, SkillEvalError> {
    validate_proposal_sources(source, proposal)?;
    store.apply_frontier_suite_proposal(proposal, output, published_at)
}

impl<S: ArtifactSource> FrontierSuiteRuntime for FileSuiteRuntime<S> {
    fn load_frontier_suite_construction_plan(
        &self,
        path: &Path,
    ) -> Result<FrontierSuiteConstructionPlan, SkillEvalError> {
        self.store.load_frontier_suite_construction_plan(path)
    }

    fn load_frontier_suite_inventory(
        &self,
        path: &Path,
    ) -> Result<FrontierSuiteInventory, SkillEvalError> {
        self.store.load_frontier_suite_inventory(path)
    }

    fn load_frontier_suite_review_set(
        &self,
        path: &Path,
    ) -> Result<FrontierSuiteReviewSet, SkillEvalError> {
        self.store.load_frontier_suite_review_set(path)
    }

    fn load_frontier_suite_proposal(
        &self,
        path: &Path,
    ) -> Result<FrontierSuiteProposal, SkillEvalError> {
        self.store.load_frontier_suite_proposal(path)
    }

    fn save_frontier_suite_inventory(
        &mut self,
        path: &Path,
        inventory: &FrontierSuiteInventory,
    ) -> Result<(), SkillEvalError> {
        self.store.save_frontier_suite_inventory(path, inventory)
    }

    fn save_frontier_suite_proposal(
        &mut self,
        path: &Path,
        proposal: &FrontierSuiteProposal,
    ) -> Result<(), SkillEvalError> {
        self.store.save_frontier_suite_proposal(path, proposal)
    }

    fn apply_frontier_suite_proposal(
        &mut self,
        proposal: &FrontierSuiteProposal,
        output: &Path,
        published_at: &Timestamp,
    ) -> Result<FrontierSuitePublication, SkillEvalError> {
        apply_current_frontier_suite(
            &self.source,
            &mut self.store,
            proposal,
            output,
            published_at,
        )
    }
}

struct FilePreviewRuntime {
    source: FileArtifactSource,
    repository_root: PathBuf,
}

impl FilePreviewRuntime {
    fn new(repository_root: &Path) -> Self {
        Self {
            source: FileArtifactSource,
            repository_root: repository_root.to_path_buf(),
        }
    }
}

impl Clock for FilePreviewRuntime {
    fn now(&self) -> Timestamp {
        local_timestamp()
    }
}

impl FrontierPreviewRuntime for FilePreviewRuntime {
    fn load_preview_frontier_plan(
        &self,
        path: &Path,
    ) -> Result<(FrontierPlan, FrontierSuite), SkillEvalError> {
        let (plan, suite) = load_frontier_plan_files(&self.repository_root, &self.source, path)?;
        if !plan.policy.is_first_party_only {
            return Err(invalid("frontier plan must require first-party routes"));
        }
        for entrant in &plan.entrants {
            require_first_party_provider(&entrant.provider)?;
        }
        require_first_party_provider(&plan.judge.provider)?;
        Ok((plan, suite))
    }
}

struct FileApplyRuntime {
    source: FileArtifactSource,
    frontier_store: FileFrontierStore,
    repository_root: PathBuf,
    pi_version: RefCell<Option<String>>,
    candidate_environment_manifest_digest: String,
    routing_configuration_sha256: String,
}

impl FileApplyRuntime {
    fn new(repository_root: &Path) -> Result<Self, SkillEvalError> {
        let configuration_path = repository_root.join("config/model-tiers.json");
        let routing_configuration =
            fs::read(&configuration_path).map_err(|error| SkillEvalError::Io {
                path: configuration_path,
                message: error.to_string(),
            })?;
        let candidate_environment_manifest = candidate_environment_manifest()?;
        Ok(Self {
            source: FileArtifactSource,
            frontier_store: FileFrontierStore::new(repository_root)?,
            repository_root: repository_root.to_path_buf(),
            pi_version: RefCell::new(None),
            candidate_environment_manifest_digest: candidate_environment_manifest_digest(
                &candidate_environment_manifest,
            )?,
            routing_configuration_sha256: sha256_digest(&routing_configuration),
        })
    }
}

impl ArtifactSource for FileApplyRuntime {
    fn load(&self, root: &Path) -> Result<ArtifactDefinition, SkillEvalError> {
        self.source.load(root)
    }
}

impl HarnessResolver for FileApplyRuntime {
    fn identity(
        &self,
        artifact: &ArtifactDefinition,
        execution: &ExecutionDefinition,
    ) -> Result<HarnessIdentity, SkillEvalError> {
        let pi_version = self
            .pi_version
            .borrow()
            .clone()
            .ok_or_else(|| invalid("frontier apply plan must load before trial evidence"))?;
        frontier_harness_identity(
            artifact,
            execution,
            &pi_version,
            &self.candidate_environment_manifest_digest,
        )
    }
}

impl Clock for FileApplyRuntime {
    fn now(&self) -> Timestamp {
        local_timestamp()
    }
}

impl FrontierTrialRuntime for FileApplyRuntime {
    fn inspect_frontier_trial(
        &self,
        selector: &FrontierTrialSelector,
    ) -> Result<FrontierInspection, SkillEvalError> {
        self.frontier_store.inspect_frontier(selector)
    }
}

impl FrontierApplyRuntime for FileApplyRuntime {
    fn load_apply_frontier_plan(
        &self,
        path: &Path,
    ) -> Result<(FrontierPlan, FrontierSuite), SkillEvalError> {
        let (plan, suite) = load_frontier_plan_files(&self.repository_root, &self.source, path)?;
        if !plan.policy.is_first_party_only {
            return Err(invalid("frontier plan must require first-party routes"));
        }
        for entrant in &plan.entrants {
            require_first_party_provider(&entrant.provider)?;
        }
        require_first_party_provider(&plan.judge.provider)?;
        self.pi_version
            .replace(Some(plan.capabilities.pi_version.clone()));
        Ok((plan, suite))
    }

    fn load_apply_frontier(
        &self,
        run_id: &FrontierRunId,
    ) -> Result<FrontierRunState, SkillEvalError> {
        self.frontier_store.load_frontier(run_id)
    }

    fn load_apply_frontier_baselines(
        &self,
        path: &Path,
    ) -> Result<FrontierBaselineLedger, SkillEvalError> {
        self.frontier_store.load_frontier_baselines(path)
    }

    fn publish_frontier_routes(
        &mut self,
        state: &FrontierRunState,
    ) -> Result<FrontierApplyReport, SkillEvalError> {
        apply_current_frontier_routes(
            &mut self.frontier_store,
            &self.repository_root,
            &mut self.routing_configuration_sha256,
            state,
        )
    }
}

pub(crate) struct ConcreteRuntime {
    source: FileArtifactSource,
    models: ConfiguredModelResolver,
    runner: PiCandidateRunner,
    verifier: FileVerifier,
    judge: PiJudge,
    store: FileRunStore,
    pool_source: FilePoolPlanSource,
    pool_store: FilePoolStore,
    frontier_store: FileFrontierStore,
    t1_screen_store: FileT1ScreenStore,
    writer: FileTierWriter,
    run_ids: PathRunIdSource,
    repository_root: PathBuf,
    routing_configuration_sha256: String,
    pi_version: String,
    candidate_environment_manifest: Vec<CandidateEnvironmentEntry>,
    candidate_environment_manifest_digest: String,
    t1_capability_snapshot: RefCell<Option<Vec<u8>>>,
    runs_root: PathBuf,
    frontier_run_lock: Option<File>,
}

impl ConcreteRuntime {
    pub(crate) fn new(runs_root: &Path) -> Result<Self, SkillEvalError> {
        fs::create_dir_all(runs_root).map_err(|error| SkillEvalError::Io {
            path: runs_root.to_path_buf(),
            message: error.to_string(),
        })?;
        let catalog = command_output("pi", &["--list-models"])?;
        let rpc_models = pi_available_models()?;
        let pi_version = command_output("pi", &["--version"])?;
        let repository_root = repository_root()?;
        let configuration_path = repository_root.join("config/model-tiers.json");
        let routing_configuration =
            fs::read(&configuration_path).map_err(|error| SkillEvalError::Io {
                path: configuration_path.clone(),
                message: error.to_string(),
            })?;
        let routing_configuration_sha256 = sha256_digest(&routing_configuration);
        let candidate_environment_manifest = candidate_environment_manifest()?;
        let candidate_environment_manifest_digest =
            candidate_environment_manifest_digest(&candidate_environment_manifest)?;
        Ok(Self {
            source: FileArtifactSource,
            models: ConfiguredModelResolver::load(&configuration_path, &catalog, &rpc_models)?,
            runner: PiCandidateRunner::new(runs_root.to_path_buf()),
            verifier: FileVerifier::new(runs_root)?,
            judge: PiJudge::new(),
            store: FileRunStore::new(runs_root)?,
            pool_source: FilePoolPlanSource::new(&repository_root)?,
            pool_store: FilePoolStore::new(runs_root)?,
            frontier_store: FileFrontierStore::new(&repository_root)?,
            t1_screen_store: FileT1ScreenStore::new(&repository_root)?,
            writer: FileTierWriter,
            run_ids: PathRunIdSource::new(runs_root)?,
            repository_root,
            routing_configuration_sha256,
            pi_version: pi_version.trim().to_owned(),
            candidate_environment_manifest,
            candidate_environment_manifest_digest,
            t1_capability_snapshot: RefCell::new(None),
            runs_root: fs::canonicalize(runs_root).map_err(|error| SkillEvalError::Io {
                path: runs_root.to_path_buf(),
                message: error.to_string(),
            })?,
            frontier_run_lock: None,
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

    fn qualification_routes(&self, tier: Tier) -> Result<Vec<ModelIdentity>, SkillEvalError> {
        self.models.qualification_routes(tier)
    }

    fn exact_candidate(&self, requested: &ModelIdentity) -> Result<ModelIdentity, SkillEvalError> {
        resolve_exact_candidate(&self.models, requested)
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

fn resolve_exact_candidate(
    models: &ConfiguredModelResolver,
    requested: &ModelIdentity,
) -> Result<ModelIdentity, SkillEvalError> {
    models.exact_candidate(requested)
}

impl HarnessResolver for ConcreteRuntime {
    fn identity(
        &self,
        artifact: &ArtifactDefinition,
        execution: &ExecutionDefinition,
    ) -> Result<HarnessIdentity, SkillEvalError> {
        frontier_harness_identity(
            artifact,
            execution,
            &self.pi_version,
            &self.candidate_environment_manifest_digest,
        )
    }
}

fn frontier_harness_identity(
    artifact: &ArtifactDefinition,
    execution: &ExecutionDefinition,
    pi_version: &str,
    candidate_environment_manifest_digest: &str,
) -> Result<HarnessIdentity, SkillEvalError> {
    if artifact.revision.trim().is_empty() {
        return Err(SkillEvalError::InvalidConfiguration(
            "harness identity requires an artifact revision".to_owned(),
        ));
    }
    let policy = serde_json::to_vec(&(
        execution,
        "candidate uses every tool and extension discovered by Pi",
        candidate_environment_manifest_digest,
    ))
    .map_err(|error| {
        SkillEvalError::InvalidConfiguration(format!("tool policy serialization failed: {error}"))
    })?;
    Ok(HarnessIdentity {
        runner_version: RUNNER_VERSION.to_owned(),
        pi_version: pi_version.to_owned(),
        artifact_revision: artifact.revision.clone(),
        tool_policy_digest: stable_digest(&policy),
    })
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
        candidate_timeout_seconds: Option<u32>,
    ) -> Result<crate::model::CandidateArtifact, SkillEvalError> {
        self.runner.execute(
            run_id,
            key,
            artifact,
            case,
            model,
            harness,
            candidate_timeout_seconds,
        )
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

impl PoolRunIdSource for ConcreteRuntime {
    fn next_pool(&mut self) -> Result<PoolRunId, SkillEvalError> {
        let run_id = self.run_ids.next()?;
        Ok(PoolRunId(format!("pool-{}", run_id.0)))
    }
}

impl PoolPlanSource for ConcreteRuntime {
    fn load_pool_plan(&self, path: &Path) -> Result<crate::model::PoolPlan, SkillEvalError> {
        self.pool_source.load_pool_plan(path)
    }

    fn validate_pool_plan_freshness(
        &self,
        plan: &crate::model::PoolPlan,
        now: &Timestamp,
    ) -> Result<(), SkillEvalError> {
        self.pool_source.validate_pool_plan_freshness(plan, now)
    }
}

impl PoolStore for ConcreteRuntime {
    fn create_pool(&mut self, state: &PoolRunState) -> Result<(), SkillEvalError> {
        self.pool_store.create_pool(state)
    }

    fn load_pool(&self, run_id: &PoolRunId) -> Result<PoolRunState, SkillEvalError> {
        self.pool_store.load_pool(run_id)
    }

    fn save_pool(&mut self, state: &PoolRunState) -> Result<(), SkillEvalError> {
        self.pool_store.save_pool(state)
    }
}

impl T1ScreenStore for ConcreteRuntime {
    fn create_t1_screen(&mut self, state: &T1ScreenRunState) -> Result<(), SkillEvalError> {
        self.t1_screen_store.create(state)
    }

    fn load_t1_screen(&self, run_id: &T1ScreenRunId) -> Result<T1ScreenRunState, SkillEvalError> {
        self.t1_screen_store.load(run_id)
    }

    fn save_t1_screen(&mut self, state: &T1ScreenRunState) -> Result<(), SkillEvalError> {
        self.t1_screen_store.save(state)
    }

    fn load_t1_screen_campaign(
        &self,
        campaign_id: &T1ScreenCampaignId,
    ) -> Result<crate::model::T1ScreenCampaignState, SkillEvalError> {
        self.t1_screen_store.load_t1_screen_campaign(campaign_id)
    }

    fn reconcile_t1_screen_campaign(
        &mut self,
        campaign_id: &T1ScreenCampaignId,
    ) -> Result<crate::model::T1ScreenCampaignState, SkillEvalError> {
        self.t1_screen_store
            .reconcile_t1_screen_campaign(campaign_id)
    }

    fn pause_t1_screen_campaign_for_budget(
        &mut self,
        campaign_id: &T1ScreenCampaignId,
    ) -> Result<crate::model::T1ScreenCampaignState, SkillEvalError> {
        self.t1_screen_store
            .pause_t1_screen_campaign_for_budget(campaign_id)
    }

    fn register_t1_screen_campaign_run(
        &mut self,
        state: &T1ScreenRunState,
    ) -> Result<crate::model::T1ScreenCampaignState, SkillEvalError> {
        self.t1_screen_store.register_t1_screen_campaign_run(state)
    }

    fn reconcile_t1_screen_campaign_run(
        &mut self,
        state: &T1ScreenRunState,
    ) -> Result<crate::model::T1ScreenCampaignState, SkillEvalError> {
        self.t1_screen_store.reconcile_t1_screen_campaign_run(state)
    }
}

impl T1ScreenRuntime for ConcreteRuntime {
    fn capability_snapshot_bytes(&self, path: &Path) -> Result<Vec<u8>, SkillEvalError> {
        let canonical = fs::canonicalize(path).map_err(|error| SkillEvalError::Io {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
        let metadata = fs::symlink_metadata(path).map_err(|error| SkillEvalError::Io {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
        if canonical != path || !canonical.starts_with(&self.repository_root) || !metadata.is_file()
        {
            return Err(SkillEvalError::InvalidConfiguration(
                "T1 capability snapshot path or type changed".to_owned(),
            ));
        }
        let bytes = fs::read(&canonical).map_err(|error| SkillEvalError::Io {
            path: canonical,
            message: error.to_string(),
        })?;
        *self.t1_capability_snapshot.borrow_mut() = Some(bytes.clone());
        Ok(bytes)
    }

    fn candidate_environment_manifest(
        &self,
    ) -> Result<Vec<CandidateEnvironmentEntry>, SkillEvalError> {
        candidate_environment_manifest()
    }

    fn judge_cost_upper_bound(
        &self,
        model: &ModelIdentity,
        _input: &crate::model::JudgeInput,
    ) -> Result<u64, SkillEvalError> {
        let snapshot = self.t1_capability_snapshot.borrow();
        let bytes = snapshot.as_deref().ok_or_else(|| {
            SkillEvalError::InvalidConfiguration(
                "T1 capability snapshot was not frozen before judge preflight".to_owned(),
            )
        })?;
        model_capabilities::t1_judge_cost_upper_bound(bytes, model)
    }

    fn conservative_next_judge_cost_upper_bound(
        &self,
        model: &ModelIdentity,
    ) -> Result<u64, SkillEvalError> {
        self.judge_cost_upper_bound(
            model,
            &crate::model::JudgeInput {
                candidate: crate::model::CandidateArtifact {
                    key: crate::model::TrialKey {
                        artifact: ArtifactName("campaign-preflight".to_owned()),
                        tier: Tier::T1,
                        route_index: 0,
                        case: CaseId("campaign-preflight".to_owned()),
                        attempt: 1,
                    },
                    model: model.clone(),
                    harness: HarnessIdentity {
                        runner_version: RUNNER_VERSION.to_owned(),
                        pi_version: "campaign-preflight".to_owned(),
                        artifact_revision: "campaign-preflight".to_owned(),
                        tool_policy_digest: "campaign-preflight".to_owned(),
                    },
                    artifact_path: PathBuf::new(),
                    transcript_path: PathBuf::new(),
                    usage: zero_t1_usage(),
                },
                expect: String::new(),
                rubric_path: PathBuf::new(),
                checks: Vec::new(),
            },
        )
    }
}

impl Clock for ConcreteRuntime {
    fn now(&self) -> Timestamp {
        local_timestamp()
    }
}

fn local_timestamp() -> Timestamp {
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

fn load_frontier_plan_files(
    repository_root: &Path,
    source: &dyn ArtifactSource,
    path: &Path,
) -> Result<(FrontierPlan, FrontierSuite), SkillEvalError> {
    let plan: FrontierPlan = read_repository_json(repository_root, path, "frontier plan")?;
    if plan.version != 1 {
        return Err(invalid("frontier plan version must be 1"));
    }
    let suite_path = repository_file(repository_root, &plan.suite.path)?;
    let suite_bytes = fs::read(&suite_path).map_err(|error| SkillEvalError::Io {
        path: suite_path.clone(),
        message: error.to_string(),
    })?;
    if sha256_digest(&suite_bytes) != plan.suite.sha256 {
        return Err(invalid("frontier suite digest changed"));
    }
    let suite: FrontierSuite = strict_json(&suite_bytes, &suite_path, "frontier suite")?;
    if suite.version != plan.suite.version || suite.version != 1 {
        return Err(invalid("frontier suite version changed"));
    }
    validate_suite_sources(source, &suite)?;

    if plan.capabilities.version != 1 {
        return Err(invalid("frontier capability snapshot version must be 1"));
    }
    let capability_path = repository_file(repository_root, &plan.capabilities.path)?;
    let capability_bytes = fs::read(&capability_path).map_err(|error| SkillEvalError::Io {
        path: capability_path.clone(),
        message: error.to_string(),
    })?;
    if sha256_digest(&capability_bytes) != plan.capabilities.sha256 {
        return Err(invalid("frontier capability snapshot digest changed"));
    }
    Ok((plan, suite))
}

fn validate_suite_sources(
    source: &dyn ArtifactSource,
    suite: &FrontierSuite,
) -> Result<(), SkillEvalError> {
    let mut revisions = BTreeMap::new();
    for case in suite.tiers.values().flat_map(|tier| &tier.cases) {
        match revisions.get(&case.artifact_path) {
            Some(revision) if revision != &case.artifact_revision => {
                return Err(invalid(format!(
                    "frontier suite has conflicting revisions for artifact {}",
                    case.artifact_path.display()
                )));
            }
            Some(_) => {}
            None => {
                revisions.insert(case.artifact_path.clone(), case.artifact_revision.clone());
            }
        }
    }
    for (path, revision) in revisions {
        let current = source.load(&path)?;
        if current.revision != revision {
            return Err(invalid(format!(
                "frontier suite artifact {} revision changed from {} to {}",
                path.display(),
                revision,
                current.revision
            )));
        }
    }
    Ok(())
}

fn read_repository_json<T: DeserializeOwned + Serialize>(
    repository_root: &Path,
    relative: &Path,
    kind: &str,
) -> Result<T, SkillEvalError> {
    let path = repository_file(repository_root, relative)?;
    let bytes = fs::read(&path).map_err(|error| SkillEvalError::Io {
        path: path.clone(),
        message: error.to_string(),
    })?;
    strict_json(&bytes, &path, kind)
}

fn strict_json<T: DeserializeOwned + Serialize>(
    bytes: &[u8],
    path: &Path,
    kind: &str,
) -> Result<T, SkillEvalError> {
    let value: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|error| invalid(format!("{kind} {} is malformed: {error}", path.display())))?;
    let parsed: T = serde_json::from_value(value.clone())
        .map_err(|error| invalid(format!("{kind} {} is malformed: {error}", path.display())))?;
    if serde_json::to_value(&parsed)
        .map_err(|error| invalid(format!("{kind} validation failed: {error}")))?
        != value
    {
        return Err(invalid(format!(
            "{kind} {} contains unknown data",
            path.display()
        )));
    }
    Ok(parsed)
}

fn repository_file(repository_root: &Path, relative: &Path) -> Result<PathBuf, SkillEvalError> {
    if relative.as_os_str().is_empty()
        || relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(invalid("frontier repository path is unsafe"));
    }
    let path = repository_root.join(relative);
    let canonical = fs::canonicalize(&path).map_err(|error| SkillEvalError::Io {
        path: path.clone(),
        message: error.to_string(),
    })?;
    if !canonical.starts_with(repository_root) || !canonical.is_file() {
        return Err(invalid("frontier repository file escapes its root"));
    }
    Ok(canonical)
}

fn require_first_party_provider(provider: &str) -> Result<(), SkillEvalError> {
    if matches!(provider, "anthropic" | "openai-codex") {
        Ok(())
    } else {
        Err(invalid(format!(
            "frontier provider {provider:?} is not an approved first-party route"
        )))
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

impl FrontierSuiteRuntime for ConcreteRuntime {
    fn load_frontier_suite_construction_plan(
        &self,
        path: &Path,
    ) -> Result<FrontierSuiteConstructionPlan, SkillEvalError> {
        self.frontier_store
            .load_frontier_suite_construction_plan(path)
    }

    fn load_frontier_suite_inventory(
        &self,
        path: &Path,
    ) -> Result<FrontierSuiteInventory, SkillEvalError> {
        self.frontier_store.load_frontier_suite_inventory(path)
    }

    fn load_frontier_suite_review_set(
        &self,
        path: &Path,
    ) -> Result<FrontierSuiteReviewSet, SkillEvalError> {
        self.frontier_store.load_frontier_suite_review_set(path)
    }

    fn load_frontier_suite_proposal(
        &self,
        path: &Path,
    ) -> Result<FrontierSuiteProposal, SkillEvalError> {
        self.frontier_store.load_frontier_suite_proposal(path)
    }

    fn save_frontier_suite_inventory(
        &mut self,
        path: &Path,
        inventory: &FrontierSuiteInventory,
    ) -> Result<(), SkillEvalError> {
        self.frontier_store
            .save_frontier_suite_inventory(path, inventory)
    }

    fn save_frontier_suite_proposal(
        &mut self,
        path: &Path,
        proposal: &FrontierSuiteProposal,
    ) -> Result<(), SkillEvalError> {
        self.frontier_store
            .save_frontier_suite_proposal(path, proposal)
    }

    fn apply_frontier_suite_proposal(
        &mut self,
        proposal: &FrontierSuiteProposal,
        output: &Path,
        published_at: &Timestamp,
    ) -> Result<FrontierSuitePublication, SkillEvalError> {
        apply_current_frontier_suite(
            &self.source,
            &mut self.frontier_store,
            proposal,
            output,
            published_at,
        )
    }
}

impl FrontierRuntime for ConcreteRuntime {
    fn lock_frontier_run(&mut self, run_id: &FrontierRunId) -> Result<(), SkillEvalError> {
        if self.frontier_run_lock.is_some() {
            return Err(invalid("frontier runtime already owns a run lock"));
        }
        self.frontier_run_lock = Some(self.frontier_store.lock_frontier_run(run_id)?);
        Ok(())
    }

    fn run_frontier_wave(
        &mut self,
        jobs: Vec<FrontierTrialJob>,
    ) -> Result<Vec<FrontierTrialOutcome>, SkillEvalError> {
        let runs_root = self.runs_root.clone();
        let identities = jobs
            .iter()
            .map(|job| {
                (
                    job.model.clone(),
                    job.key.clone(),
                    job.infrastructure_attempt,
                )
            })
            .collect::<Vec<_>>();
        Ok(identities
            .into_iter()
            .zip(run_bounded_frontier_jobs(jobs, |job| {
                run_concrete_frontier_job(&runs_root, job)
            }))
            .map(|((model, key, infrastructure_attempt), outcome)| {
                outcome.unwrap_or_else(|()| FrontierTrialOutcome {
                    model,
                    key,
                    infrastructure_attempt,
                    result: Err(SkillEvalError::Process {
                        program: "frontier worker".to_owned(),
                        exit_code: None,
                        standard_error: "frontier worker panicked".to_owned(),
                    }),
                })
            })
            .collect())
    }

    fn load_frontier_plan(
        &self,
        path: &Path,
    ) -> Result<(FrontierPlan, FrontierSuite), SkillEvalError> {
        let (plan, suite) = load_frontier_plan_files(&self.repository_root, &self.source, path)?;
        if plan.capabilities.pi_version != self.pi_version {
            return Err(invalid("frontier capability Pi version changed"));
        }
        if !plan.policy.is_first_party_only {
            return Err(invalid("frontier plan must require first-party routes"));
        }
        for entrant in &plan.entrants {
            require_first_party_provider(&entrant.provider)?;
            for thinking in &entrant.thinking_levels {
                let requested = ModelIdentity {
                    tier: entrant.entry_tier,
                    provider: entrant.provider.clone(),
                    model: entrant.model.clone(),
                    thinking: thinking.clone(),
                };
                if self.models.exact_candidate(&requested)? != requested {
                    return Err(invalid(
                        "frontier entrant identity changed during resolution",
                    ));
                }
            }
        }
        require_first_party_provider(&plan.judge.provider)?;
        if self.models.exact_candidate(&plan.judge)? != plan.judge {
            return Err(invalid("frontier judge identity changed during resolution"));
        }
        Ok((plan, suite))
    }

    fn next_frontier_run_id(&mut self) -> Result<FrontierRunId, SkillEvalError> {
        next_frontier_run_id(&mut self.run_ids)
    }

    fn create_frontier(&mut self, state: &FrontierRunState) -> Result<(), SkillEvalError> {
        self.frontier_store.create_frontier(state)
    }

    fn load_frontier(&self, run_id: &FrontierRunId) -> Result<FrontierRunState, SkillEvalError> {
        self.frontier_store.load_frontier(run_id)
    }

    fn save_frontier(&mut self, state: &FrontierRunState) -> Result<(), SkillEvalError> {
        self.frontier_store.save_frontier(state)
    }

    fn recover_frontier_trial(
        &mut self,
        state: &FrontierRunState,
        key: &crate::model::TrialKey,
        artifact: &ArtifactDefinition,
        case: &CaseDefinition,
        model: &ModelIdentity,
        harness: &HarnessIdentity,
    ) -> Result<Option<TrialRecord>, SkillEvalError> {
        let run_id = RunId(state.configuration.run_id.0.clone());
        let judge = &state.configuration.plan.judge;
        let Some(candidate) = self
            .runner
            .recover_frontier(&run_id, key, case, model, harness)?
        else {
            return Ok(None);
        };
        let checks = self.verifier.verify(case, &candidate)?;
        let input = JudgeInput {
            candidate: candidate.clone(),
            expect: case.expect.clone(),
            rubric_path: artifact.root.join("evals/rubric.md"),
            checks,
        };
        let Some(judged) = self.judge.recover_frontier_grade(judge, &input)? else {
            return Ok(None);
        };
        Ok(Some(TrialRecord {
            key: key.clone(),
            model: model.clone(),
            harness: harness.clone(),
            artifact_path: candidate.artifact_path,
            transcript_path: candidate.transcript_path,
            candidate_usage: candidate.usage,
            judge_model: judged.model,
            judge_usage: judged.usage,
            verdict: judged.verdict,
        }))
    }

    fn save_frontier_trial(
        &mut self,
        run_id: &FrontierRunId,
        trial: &TrialRecord,
    ) -> Result<(), SkillEvalError> {
        self.frontier_store.save_frontier_trial(run_id, trial)
    }

    fn inspect_frontier(
        &self,
        selector: &FrontierTrialSelector,
    ) -> Result<FrontierInspection, SkillEvalError> {
        self.frontier_store.inspect_frontier(selector)
    }

    fn load_frontier_baselines(
        &self,
        path: &Path,
    ) -> Result<FrontierBaselineLedger, SkillEvalError> {
        self.frontier_store.load_frontier_baselines(path)
    }

    fn accept_frontier_baseline(
        &mut self,
        state: &FrontierRunState,
        path: &Path,
        ledger: &FrontierBaselineLedger,
    ) -> Result<(), SkillEvalError> {
        self.frontier_store
            .accept_frontier_baseline(state, path, ledger)
    }

    fn apply_frontier_routes(
        &mut self,
        state: &FrontierRunState,
    ) -> Result<FrontierApplyReport, SkillEvalError> {
        apply_current_frontier_routes(
            &mut self.frontier_store,
            &self.repository_root,
            &mut self.routing_configuration_sha256,
            state,
        )
    }
}

fn run_bounded_frontier_jobs<T, R>(
    jobs: Vec<T>,
    worker: impl Fn(T) -> R + Sync,
) -> Vec<Result<R, ()>>
where
    T: Send,
    R: Send,
{
    std::thread::scope(|scope| {
        let mut jobs = jobs.into_iter();
        let mut outcomes = Vec::new();
        loop {
            let handles = jobs
                .by_ref()
                .take(FRONTIER_WORKER_LIMIT)
                .map(|job| scope.spawn(|| worker(job)))
                .collect::<Vec<_>>();
            if handles.is_empty() {
                break;
            }
            outcomes.extend(
                handles
                    .into_iter()
                    .map(|handle| handle.join().map_err(|_| ())),
            );
        }
        outcomes
    })
}

fn run_concrete_frontier_job(runs_root: &Path, job: FrontierTrialJob) -> FrontierTrialOutcome {
    let result = (|| {
        let mut runner = PiCandidateRunner::new(runs_root.to_path_buf());
        let mut verifier = FileVerifier::new(runs_root)?;
        let mut judge = PiJudge::new();
        let candidate = match runner.recover_frontier(
            &job.run_id,
            &job.key,
            &job.case,
            &job.model,
            &job.harness,
        )? {
            Some(candidate) => candidate,
            None => runner.execute(
                &job.run_id,
                &job.key,
                &job.artifact,
                &job.case,
                &job.model,
                &job.harness,
                None,
            )?,
        };
        if candidate.key != job.key
            || candidate.model != job.model
            || candidate.harness != job.harness
        {
            return Err(invalid("frontier worker candidate identity drifted"));
        }
        let checks = verifier.verify(&job.case, &candidate)?;
        let input = JudgeInput {
            candidate: candidate.clone(),
            expect: job.case.expect.clone(),
            rubric_path: job.artifact.root.join("evals/rubric.md"),
            checks,
        };
        let judged = match judge.recover_frontier_grade(&job.judge, &input)? {
            Some(judged) => judged,
            None => judge.grade(&job.judge, &input)?,
        };
        if judged.model != job.judge {
            return Err(invalid("frontier worker judge identity drifted"));
        }
        let cost = candidate
            .usage
            .cost_millionths_of_dollar
            .checked_add(judged.usage.cost_millionths_of_dollar)
            .ok_or_else(|| invalid("frontier worker trial cost overflow"))?;
        if cost > job.reserved_cost_millionths_of_dollar {
            return Err(invalid(
                "frontier worker trial cost exceeded its reservation",
            ));
        }
        Ok(TrialRecord {
            key: job.key.clone(),
            model: candidate.model,
            harness: candidate.harness,
            artifact_path: candidate.artifact_path,
            transcript_path: candidate.transcript_path,
            candidate_usage: candidate.usage,
            judge_model: judged.model,
            judge_usage: judged.usage,
            verdict: judged.verdict,
        })
    })();
    FrontierTrialOutcome {
        model: job.model,
        key: job.key,
        infrastructure_attempt: job.infrastructure_attempt,
        result,
    }
}

fn apply_current_frontier_routes(
    frontier_store: &mut FileFrontierStore,
    repository_root: &Path,
    routing_configuration_sha256: &mut String,
    state: &FrontierRunState,
) -> Result<FrontierApplyReport, SkillEvalError> {
    let ledger =
        frontier_store.load_frontier_baselines(Path::new("config/model-frontier-baseline.json"))?;
    let baseline = ledger.baselines.last().ok_or_else(|| {
        invalid("frontier route publication requires a current accepted baseline")
    })?;
    let report = apply_frontier_routes_at(
        repository_root,
        routing_configuration_sha256,
        state,
        baseline,
    )?;
    if report.is_changed {
        let path = repository_root.join("config/model-tiers.json");
        let bytes = fs::read(&path).map_err(|error| SkillEvalError::Io {
            path,
            message: error.to_string(),
        })?;
        *routing_configuration_sha256 = sha256_digest(&bytes);
    }
    Ok(report)
}

#[derive(Serialize)]
struct PublishedFrontierRoute<'a> {
    provider: &'a str,
    model: &'a str,
    thinking: &'a str,
}

pub(crate) fn apply_frontier_routes_at(
    repository_root: &Path,
    routing_configuration_sha256: &str,
    state: &FrontierRunState,
    baseline: &FrontierBaseline,
) -> Result<FrontierApplyReport, SkillEvalError> {
    let active_routes = active_frontier_routes(state, baseline)?;
    let mut state_bytes = serde_json::to_vec_pretty(state).map_err(|error| {
        frontier_route_invalid(format!("accepted state serialization failed: {error}"))
    })?;
    state_bytes.push(b'\n');
    if sha256_digest(&state_bytes) != baseline.run_evidence.sha256 {
        return Err(frontier_route_invalid("accepted state bytes drifted"));
    }
    let path = routing_configuration_path(repository_root)?;
    let stored = fs::read(&path).map_err(|error| SkillEvalError::Io {
        path: path.clone(),
        message: error.to_string(),
    })?;
    if sha256_digest(&stored) != routing_configuration_sha256 {
        return Err(frontier_route_invalid(
            "routing authority changed before apply",
        ));
    }
    let replacement = published_routes_bytes(&active_routes, &stored)?;
    let span = qualification_routes_span(&stored)?;
    let mut next = Vec::with_capacity(stored.len() - span.len() + replacement.len());
    next.extend_from_slice(&stored[..span.start]);
    next.extend_from_slice(&replacement);
    next.extend_from_slice(&stored[span.end..]);
    serde_json::from_slice::<serde_json::Value>(&next)
        .map_err(|error| frontier_route_invalid(format!("routing output is malformed: {error}")))?;
    let is_changed = next != stored;
    if is_changed {
        replace_routing_bytes(&path, &stored, &next)?;
    }
    Ok(FrontierApplyReport {
        run_id: state.configuration.run_id.clone(),
        active_routes,
        is_changed,
    })
}

fn routing_configuration_path(repository_root: &Path) -> Result<PathBuf, SkillEvalError> {
    let canonical_root = fs::canonicalize(repository_root).map_err(|error| SkillEvalError::Io {
        path: repository_root.to_path_buf(),
        message: error.to_string(),
    })?;
    if canonical_root != repository_root || !canonical_root.is_dir() {
        return Err(frontier_route_invalid("repository root is not canonical"));
    }
    let parent = canonical_root.join("config");
    let canonical_parent = fs::canonicalize(&parent).map_err(|error| SkillEvalError::Io {
        path: parent.clone(),
        message: error.to_string(),
    })?;
    if canonical_parent != parent || !canonical_parent.is_dir() {
        return Err(frontier_route_invalid(
            "routing destination directory escapes the repository",
        ));
    }
    let path = parent.join("model-tiers.json");
    let metadata = fs::symlink_metadata(&path).map_err(|error| SkillEvalError::Io {
        path: path.clone(),
        message: error.to_string(),
    })?;
    let canonical = fs::canonicalize(&path).map_err(|error| SkillEvalError::Io {
        path: path.clone(),
        message: error.to_string(),
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || canonical != path {
        return Err(frontier_route_invalid(
            "routing destination is not the owned regular file",
        ));
    }
    Ok(path)
}

fn published_routes_bytes(
    active_routes: &BTreeMap<Tier, Vec<ModelIdentity>>,
    stored: &[u8],
) -> Result<Vec<u8>, SkillEvalError> {
    let routes = active_routes
        .iter()
        .map(|(tier, routes)| {
            (
                tier_label(*tier).to_owned(),
                routes
                    .iter()
                    .map(|route| PublishedFrontierRoute {
                        provider: &route.provider,
                        model: &route.model,
                        thinking: &route.thinking,
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let raw = serde_json::to_string_pretty(&routes).map_err(|error| {
        frontier_route_invalid(format!("active route serialization failed: {error}"))
    })?;
    let newline = if stored.windows(2).any(|window| window == b"\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    Ok(raw
        .split('\n')
        .enumerate()
        .fold(Vec::new(), |mut bytes, (index, line)| {
            if index > 0 {
                bytes.extend_from_slice(newline.as_bytes());
                bytes.extend_from_slice(b"  ");
            }
            bytes.extend_from_slice(line.as_bytes());
            bytes
        }))
}

fn qualification_routes_span(bytes: &[u8]) -> Result<std::ops::Range<usize>, SkillEvalError> {
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|error| {
        frontier_route_invalid(format!("routing configuration is malformed: {error}"))
    })?;
    if value
        .as_object()
        .and_then(|object| object.get("qualification_routes"))
        .is_none_or(|routes| !routes.is_object())
    {
        return Err(frontier_route_invalid(
            "routing configuration has no qualification route object",
        ));
    }

    let mut index = skip_json_whitespace(bytes, 0);
    if bytes.get(index) != Some(&b'{') {
        return Err(frontier_route_invalid(
            "routing configuration is not an object",
        ));
    }
    index += 1;
    let mut found = None;
    let mut keys = BTreeSet::new();
    loop {
        index = skip_json_whitespace(bytes, index);
        if bytes.get(index) == Some(&b'}') {
            break;
        }
        let key_start = index;
        let key_end = json_string_end(bytes, key_start)?;
        let key: String = serde_json::from_slice(&bytes[key_start..key_end]).map_err(|error| {
            frontier_route_invalid(format!("routing key is malformed: {error}"))
        })?;
        if !keys.insert(key.clone()) {
            return Err(frontier_route_invalid(format!(
                "routing configuration duplicates top-level key {key:?}"
            )));
        }
        index = skip_json_whitespace(bytes, key_end);
        if bytes.get(index) != Some(&b':') {
            return Err(frontier_route_invalid("routing member has no value"));
        }
        index = skip_json_whitespace(bytes, index + 1);
        let value_start = index;
        let value_end = json_value_end(bytes, value_start)?;
        if key == "qualification_routes" {
            found = Some(value_start..value_end);
        }
        index = skip_json_whitespace(bytes, value_end);
        match bytes.get(index) {
            Some(b',') => index += 1,
            Some(b'}') => break,
            _ => {
                return Err(frontier_route_invalid(
                    "routing object boundary is malformed",
                ));
            }
        }
    }
    found.ok_or_else(|| frontier_route_invalid("qualification_routes is absent"))
}

fn skip_json_whitespace(bytes: &[u8], mut index: usize) -> usize {
    while bytes
        .get(index)
        .is_some_and(|byte| matches!(byte, b' ' | b'\n' | b'\r' | b'\t'))
    {
        index += 1;
    }
    index
}

fn json_string_end(bytes: &[u8], start: usize) -> Result<usize, SkillEvalError> {
    if bytes.get(start) != Some(&b'"') {
        return Err(frontier_route_invalid("routing object key is not a string"));
    }
    let mut index = start + 1;
    while let Some(byte) = bytes.get(index) {
        match byte {
            b'\\' => index = index.saturating_add(2),
            b'"' => return Ok(index + 1),
            _ => index += 1,
        }
    }
    Err(frontier_route_invalid("routing string is unterminated"))
}

fn json_value_end(bytes: &[u8], start: usize) -> Result<usize, SkillEvalError> {
    match bytes.get(start) {
        Some(b'"') => json_string_end(bytes, start),
        Some(b'{') | Some(b'[') => {
            let mut stack = vec![if bytes[start] == b'{' { b'}' } else { b']' }];
            let mut index = start + 1;
            while let Some(byte) = bytes.get(index) {
                match byte {
                    b'"' => index = json_string_end(bytes, index)?,
                    b'{' => {
                        stack.push(b'}');
                        index += 1;
                    }
                    b'[' => {
                        stack.push(b']');
                        index += 1;
                    }
                    b'}' | b']' => {
                        if stack.pop() != Some(*byte) {
                            return Err(frontier_route_invalid(
                                "routing value nesting is malformed",
                            ));
                        }
                        index += 1;
                        if stack.is_empty() {
                            return Ok(index);
                        }
                    }
                    _ => index += 1,
                }
            }
            Err(frontier_route_invalid("routing value is unterminated"))
        }
        Some(_) => {
            let mut index = start;
            while bytes.get(index).is_some_and(|byte| {
                !matches!(byte, b',' | b'}' | b']' | b' ' | b'\n' | b'\r' | b'\t')
            }) {
                index += 1;
            }
            Ok(index)
        }
        None => Err(frontier_route_invalid("routing value is absent")),
    }
}

fn replace_routing_bytes(path: &Path, expected: &[u8], next: &[u8]) -> Result<(), SkillEvalError> {
    let parent = path
        .parent()
        .ok_or_else(|| frontier_route_invalid("routing destination has no parent"))?;
    let metadata = fs::metadata(path).map_err(|error| SkillEvalError::Io {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    let temporary = routing_temporary_path(path, "next")?;
    write_routing_temporary(&temporary, next, metadata.permissions())?;
    let result = (|| {
        let current = fs::read(path).map_err(|error| SkillEvalError::Io {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
        if current != expected {
            return Err(frontier_route_invalid(
                "routing authority changed before replacement",
            ));
        }
        fs::rename(&temporary, path).map_err(|error| SkillEvalError::Io {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
        sync_routing_directory(parent).or_else(|error| {
            rollback_routing_bytes(path, expected, metadata.permissions()).map_err(|rollback| {
                frontier_route_invalid(format!(
                    "routing sync failed: {error:?}; rollback failed: {rollback:?}"
                ))
            })?;
            Err(error)
        })
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn rollback_routing_bytes(
    path: &Path,
    bytes: &[u8],
    permissions: fs::Permissions,
) -> Result<(), SkillEvalError> {
    let temporary = routing_temporary_path(path, "rollback")?;
    write_routing_temporary(&temporary, bytes, permissions)?;
    fs::rename(&temporary, path).map_err(|error| SkillEvalError::Io {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    let parent = path
        .parent()
        .ok_or_else(|| frontier_route_invalid("routing destination has no parent"))?;
    sync_routing_directory(parent)
}

fn routing_temporary_path(path: &Path, purpose: &str) -> Result<PathBuf, SkillEvalError> {
    let parent = path
        .parent()
        .ok_or_else(|| frontier_route_invalid("routing destination has no parent"))?;
    let name = path
        .file_name()
        .ok_or_else(|| frontier_route_invalid("routing destination has no file name"))?
        .to_string_lossy();
    let sequence = ROUTING_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    Ok(parent.join(format!(
        ".{name}.{}.{sequence}.{purpose}.tmp",
        std::process::id()
    )))
}

fn write_routing_temporary(
    path: &Path,
    bytes: &[u8],
    permissions: fs::Permissions,
) -> Result<(), SkillEvalError> {
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|error| SkillEvalError::Io {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
    let result = file
        .write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| SkillEvalError::Io {
            path: path.to_path_buf(),
            message: error.to_string(),
        })
        .and_then(|()| {
            fs::set_permissions(path, permissions).map_err(|error| SkillEvalError::Io {
                path: path.to_path_buf(),
                message: error.to_string(),
            })
        });
    if result.is_err() {
        let _ = fs::remove_file(path);
    }
    result
}

fn sync_routing_directory(path: &Path) -> Result<(), SkillEvalError> {
    fs::File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| SkillEvalError::Io {
            path: path.to_path_buf(),
            message: error.to_string(),
        })
}

fn frontier_route_invalid(message: impl Into<String>) -> SkillEvalError {
    SkillEvalError::InvalidConfiguration(format!("frontier route publication: {}", message.into()))
}

impl QualificationRuntime for ConcreteRuntime {}
impl PoolRuntime for ConcreteRuntime {}

#[cfg(test)]
thread_local! {
    static TEST_POOL_STATE: RefCell<Option<PoolRunState>> = const { RefCell::new(None) };
}

#[cfg(test)]
impl PoolRunIdSource for crate::testing::FakeQualificationRuntime {
    fn next_pool(&mut self) -> Result<PoolRunId, SkillEvalError> {
        Ok(PoolRunId("pool-test".to_owned()))
    }
}

#[cfg(test)]
impl PoolPlanSource for crate::testing::FakeQualificationRuntime {
    fn load_pool_plan(&self, _path: &Path) -> Result<crate::model::PoolPlan, SkillEvalError> {
        Ok(test_pool_plan())
    }

    fn validate_pool_plan_freshness(
        &self,
        _plan: &crate::model::PoolPlan,
        _now: &Timestamp,
    ) -> Result<(), SkillEvalError> {
        Ok(())
    }
}

#[cfg(test)]
impl PoolStore for crate::testing::FakeQualificationRuntime {
    fn create_pool(&mut self, state: &PoolRunState) -> Result<(), SkillEvalError> {
        TEST_POOL_STATE.with(|slot| *slot.borrow_mut() = Some(state.clone()));
        Ok(())
    }

    fn load_pool(&self, run_id: &PoolRunId) -> Result<PoolRunState, SkillEvalError> {
        TEST_POOL_STATE.with(|slot| {
            slot.borrow()
                .clone()
                .filter(|state| state.configuration.run_id == *run_id)
                .ok_or_else(|| SkillEvalError::NotFound(format!("pool {:?}", run_id.0)))
        })
    }

    fn save_pool(&mut self, state: &PoolRunState) -> Result<(), SkillEvalError> {
        TEST_POOL_STATE.with(|slot| *slot.borrow_mut() = Some(state.clone()));
        Ok(())
    }
}

#[cfg(test)]
impl PoolRuntime for crate::testing::FakeQualificationRuntime {}

#[cfg(test)]
fn test_pool_plan() -> crate::model::PoolPlan {
    use std::collections::BTreeMap;

    use crate::model::{PoolEntrant, PoolPlan, PoolPolicy};

    let mut entrants = BTreeMap::new();
    for tier in [Tier::T1, Tier::T2, Tier::T3, Tier::T4, Tier::T5] {
        entrants.insert(
            tier,
            (0..3)
                .map(|index| {
                    let model = ModelIdentity {
                        tier,
                        provider: format!("provider-{index}"),
                        model: format!("exact-{index}"),
                        thinking: "off".to_owned(),
                    };
                    PoolEntrant {
                        thinking_levels: vec![model.thinking.clone()],
                        retained_lower_thinking_level: None,
                        model,
                        candidate_timeout_seconds: None,
                        catalog_observed_at: Timestamp("2026-08-24T11:59:00-0400".to_owned()),
                    }
                })
                .collect(),
        );
    }
    PoolPlan {
        entrants,
        control: ModelIdentity {
            tier: Tier::T1,
            provider: "control-provider".to_owned(),
            model: "control-model".to_owned(),
            thinking: "off".to_owned(),
        },
        policy: PoolPolicy {
            calibration_repeats_per_case: 1,
            qualification_repeats_per_case: 2,
            promotion_count: 2,
            minimum_score: 8,
            calibration_minimum_reliability_basis_points: 8_000,
            qualification_minimum_reliability_basis_points: 10_000,
            maximum_catalog_age_seconds: 3_600,
            spending_limit_millionths_of_dollar: 10_000_000,
            is_provider_limit_enforced: true,
        },
    }
}

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

fn next_frontier_run_id(source: &mut dyn RunIdSource) -> Result<FrontierRunId, SkillEvalError> {
    Ok(FrontierRunId(format!("frontier-{}", source.next()?.0)))
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
    match &request.command {
        CliCommand::ModelCapabilities { output } => {
            return run_model_capabilities(output, &mut std::io::stdout());
        }
        CliCommand::T1ScreenPreview {
            capabilities,
            format,
        } => {
            return run_t1_screen_preview(capabilities, *format, &mut std::io::stdout());
        }
        CliCommand::T1ScreenCampaignCreate { request, format } => {
            return run_t1_screen_campaign_create(request, *format, &mut std::io::stdout());
        }
        CliCommand::T1ScreenCampaignExtendCap { request, format } => {
            return run_t1_screen_campaign_extend_cap(request, *format, &mut std::io::stdout());
        }
        CliCommand::T1ScreenCampaignRetireRun { request, format } => {
            return run_t1_screen_campaign_retire_run(request, *format, &mut std::io::stdout());
        }
        CliCommand::T1ScreenStart {
            request: start,
            format,
        } => {
            return run_t1_screen_start(start, &request.runs_root, *format, &mut std::io::stdout());
        }
        CliCommand::T1ScreenResume { run_id, format } => {
            return run_t1_screen_resume(
                run_id,
                &request.runs_root,
                *format,
                &mut std::io::stdout(),
            );
        }
        CliCommand::T1ScreenExtendCap {
            request: extension,
            format,
        } => {
            return run_t1_screen_extend_cap(
                extension,
                &request.runs_root,
                *format,
                &mut std::io::stdout(),
            );
        }
        CliCommand::T1ScreenFailRoute {
            request: route_failure,
            format,
        } => {
            return run_t1_screen_fail_route(
                route_failure,
                &request.runs_root,
                *format,
                &mut std::io::stdout(),
            );
        }
        CliCommand::T1ScreenReport { run_id, format } => {
            return run_t1_screen_report(
                run_id,
                &request.runs_root,
                *format,
                &mut std::io::stdout(),
            );
        }
        CliCommand::FrontierSuiteInventory { .. }
        | CliCommand::FrontierSuitePropose { .. }
        | CliCommand::FrontierSuiteCheck { .. }
        | CliCommand::FrontierSuiteApply { .. } => {
            let mut runtime = FileSuiteRuntime::new(&repository_root()?)?;
            return execute_frontier_suite_command(
                &request.command,
                request.output_format,
                &mut runtime,
                &mut std::io::stdout(),
            );
        }
        CliCommand::FrontierPreview { .. } => {
            let runtime = FilePreviewRuntime::new(&repository_root()?);
            return execute_frontier_preview_command(
                &request.command,
                request.output_format,
                &runtime,
                &mut std::io::stdout(),
            );
        }
        CliCommand::FrontierApply { .. } => {
            let mut runtime = FileApplyRuntime::new(&repository_root()?)?;
            return execute_frontier_apply_command(
                &request.command,
                request.output_format,
                &mut runtime,
                &mut std::io::stdout(),
            );
        }
        CliCommand::FrontierStart { .. }
        | CliCommand::FrontierResume { .. }
        | CliCommand::FrontierReport { .. }
        | CliCommand::FrontierInspect { .. }
        | CliCommand::FrontierDecide { .. } => {
            let mut runtime = ConcreteRuntime::new(&request.runs_root)?;
            return execute_frontier_command(
                &request.command,
                request.output_format,
                &mut runtime,
                &mut std::io::stdout(),
            );
        }
        _ => {}
    }
    let mut runtime = ConcreteRuntime::new(&request.runs_root)?;
    execute_command(request, &mut runtime, &mut std::io::stdout())
}

fn run_t1_screen_campaign_retire_run(
    request: &T1ScreenCampaignRunRetirementRequest,
    format: T1ScreenFormat,
    output: &mut dyn Write,
) -> Result<(), SkillEvalError> {
    run_t1_screen_campaign_retire_run_at(
        &repository_root()?,
        request,
        local_timestamp(),
        format,
        output,
    )
}

fn run_t1_screen_campaign_retire_run_at(
    repository_root: &Path,
    request: &T1ScreenCampaignRunRetirementRequest,
    timestamp: Timestamp,
    format: T1ScreenFormat,
    output: &mut dyn Write,
) -> Result<(), SkillEvalError> {
    let mut store = FileT1ScreenCampaignStore::open(repository_root)?;
    let state = store.retire_run(request, timestamp)?;
    render_t1_screen_campaign_retirement(&state, &request.run_id, format, output)
}

fn render_t1_screen_campaign_retirement(
    state: &crate::model::T1ScreenCampaignState,
    run_id: &T1ScreenRunId,
    format: T1ScreenFormat,
    output: &mut dyn Write,
) -> Result<(), SkillEvalError> {
    if format == T1ScreenFormat::Json {
        return write_json_line(state, output);
    }
    let remaining = state
        .approved_judge_total_millionths_of_dollar
        .checked_sub(state.aggregate_judge_spent_millionths_of_dollar)
        .ok_or_else(|| invalid("campaign remaining spend underflow"))?;
    writeln!(
        output,
        "retired T1 screen campaign run {}; total {}, spent {}, remaining {} millionths",
        run_id.0,
        state.approved_judge_total_millionths_of_dollar,
        state.aggregate_judge_spent_millionths_of_dollar,
        remaining
    )
    .map_err(output_error)
}

fn run_t1_screen_campaign_extend_cap(
    request: &T1ScreenCampaignCapExtensionRequest,
    format: T1ScreenFormat,
    output: &mut dyn Write,
) -> Result<(), SkillEvalError> {
    run_t1_screen_campaign_extend_cap_at(
        &repository_root()?,
        request,
        local_timestamp(),
        format,
        output,
    )
}

fn run_t1_screen_campaign_extend_cap_at(
    repository_root: &Path,
    request: &T1ScreenCampaignCapExtensionRequest,
    timestamp: Timestamp,
    format: T1ScreenFormat,
    output: &mut dyn Write,
) -> Result<(), SkillEvalError> {
    let mut store = FileT1ScreenCampaignStore::open(repository_root)?;
    let state = store.extend_cap(request, timestamp)?;
    render_t1_screen_campaign_cap_extension(&state, format, output)
}

fn render_t1_screen_campaign_cap_extension(
    state: &crate::model::T1ScreenCampaignState,
    format: T1ScreenFormat,
    output: &mut dyn Write,
) -> Result<(), SkillEvalError> {
    if format == T1ScreenFormat::Json {
        return write_json_line(state, output);
    }
    let remaining = state
        .approved_judge_total_millionths_of_dollar
        .checked_sub(state.aggregate_judge_spent_millionths_of_dollar)
        .ok_or_else(|| invalid("campaign remaining spend underflow"))?;
    writeln!(
        output,
        "T1 screen campaign {}: {:?}",
        state.campaign_id.0, state.status
    )
    .map_err(output_error)?;
    writeln!(
        output,
        "judge budget: total {}, spent {}, remaining {} millionths",
        state.approved_judge_total_millionths_of_dollar,
        state.aggregate_judge_spent_millionths_of_dollar,
        remaining
    )
    .map_err(output_error)?;
    writeln!(output, "cap extensions: {}", state.cap_extensions.len()).map_err(output_error)
}

fn run_t1_screen_campaign_create(
    request: &T1ScreenCampaignCreateRequest,
    format: T1ScreenFormat,
    output: &mut dyn Write,
) -> Result<(), SkillEvalError> {
    let repository_root = repository_root()?;
    let mut store = FileT1ScreenCampaignStore::new(&repository_root)?;
    let state = store.create_from_runs(
        &request.campaign_id,
        request.judge_cap_millionths_of_dollar,
        &request.owner_reason,
        local_timestamp(),
        &request.run_ids,
    )?;
    if format == T1ScreenFormat::Json {
        return write_json_line(&state, output);
    }
    let remaining = state
        .approved_judge_total_millionths_of_dollar
        .checked_sub(state.aggregate_judge_spent_millionths_of_dollar)
        .ok_or_else(|| invalid("campaign remaining spend underflow"))?;
    writeln!(
        output,
        "T1 screen campaign {}: {:?}",
        state.campaign_id.0, state.status
    )
    .map_err(output_error)?;
    writeln!(
        output,
        "judge budget: total {}, spent {}, remaining {} millionths",
        state.approved_judge_total_millionths_of_dollar,
        state.aggregate_judge_spent_millionths_of_dollar,
        remaining
    )
    .map_err(output_error)?;
    writeln!(output, "runs: {}", state.runs.len()).map_err(output_error)?;
    for run in &state.runs {
        writeln!(
            output,
            "  {}: {:?}; judge {}; candidate {}; resumable={}",
            run.run_id.0,
            run.observed_status,
            run.judge_spend_millionths_of_dollar,
            run.candidate_cost_millionths_of_dollar,
            run.is_resumable
        )
        .map_err(output_error)?;
    }
    writeln!(
        output,
        "active run: {}",
        state
            .active_run_id
            .as_ref()
            .map_or("none", |run_id| run_id.0.as_str())
    )
    .map_err(output_error)
}

fn run_t1_screen_start(
    request: &T1ScreenStartRequest,
    runs_root: &Path,
    format: T1ScreenFormat,
    output: &mut dyn Write,
) -> Result<(), SkillEvalError> {
    preflight_run_id_file()?;
    let repository_root = repository_root()?;
    let mut preview =
        model_capabilities::t1_screen_preview(&repository_root, &request.capabilities)?;
    if preview.eligible.is_empty() {
        return Err(SkillEvalError::InvalidConfiguration(
            "T1 screening snapshot has no eligible models".to_owned(),
        ));
    }
    validate_tracked_capability_snapshot(
        &repository_root,
        &request.capabilities,
        &preview.snapshot.sha256,
    )?;
    preview.snapshot.path = repository_root
        .join(&request.capabilities)
        .canonicalize()
        .map_err(|error| SkillEvalError::Io {
            path: request.capabilities.clone(),
            message: error.to_string(),
        })?;
    let source = FileArtifactSource;
    let exam = source.load(&repository_root.join(&request.exam))?;
    if exam.cases.len() != 5 || exam.cases.iter().any(|case| case.is_holdout) {
        return Err(SkillEvalError::InvalidConfiguration(
            "T1 screening exam must contain exactly five non-holdout cases".to_owned(),
        ));
    }

    let mut runtime = ConcreteRuntime::new(runs_root)?;
    let state = build_t1_screen_initial_state(request, preview, exam, &mut runtime)?;
    let mut progress = T1CliProgress {
        is_run_id_written: false,
    };
    let state = start_t1_screening(state, &mut runtime, &mut progress)?;
    let report = build_t1_screen_report(&state.configuration.run_id, &runtime, &runtime)?;
    render_t1_screen_report(&report, format, output)
}

fn build_t1_screen_initial_state(
    request: &T1ScreenStartRequest,
    preview: T1ScreenPreviewReport,
    exam: ArtifactDefinition,
    runtime: &mut ConcreteRuntime,
) -> Result<T1ScreenRunState, SkillEvalError> {
    if request.provider_enforced_judge_cap_millionths_of_dollar
        > request.owner_approved_judge_cap_millionths_of_dollar
    {
        return Err(SkillEvalError::InvalidConfiguration(
            "T1 provider cap exceeds the owner-approved cap".to_owned(),
        ));
    }
    let parent_id = runtime.next()?;
    let run_id = T1ScreenRunId(format!("t1-screen-{}", parent_id.0));
    let child_runs = preallocate_t1_screen_children(&preview.eligible, runtime)?;
    let judge_tier = runtime.configured_judge_tier()?;
    let mut judge = None;
    for child in &child_runs {
        if runtime.exact_candidate(&child.model)? != child.model {
            return Err(SkillEvalError::InvalidConfiguration(
                "T1 exact candidate identity changed during start".to_owned(),
            ));
        }
        let resolved = runtime.pool_judge(&child.model)?;
        if resolved.tier != judge_tier
            || resolved.provider == child.model.provider && resolved.model == child.model.model
        {
            return Err(SkillEvalError::InvalidConfiguration(
                "T1 screening requires one exact external configured judge".to_owned(),
            ));
        }
        if judge.as_ref().is_some_and(|frozen| frozen != &resolved) {
            return Err(SkillEvalError::InvalidConfiguration(
                "T1 screening judge identity differs across eligible routes".to_owned(),
            ));
        }
        judge = Some(resolved);
    }
    let judge = judge.ok_or_else(|| {
        SkillEvalError::InvalidConfiguration("T1 screening has no eligible judge route".to_owned())
    })?;
    let harnesses = exam
        .cases
        .iter()
        .map(|case| runtime.identity(&exam, &case.execution))
        .collect::<Result<Vec<_>, _>>()?;
    if harnesses.len() != 5 {
        return Err(SkillEvalError::InvalidConfiguration(
            "T1 screening requires exactly five candidate harness identities".to_owned(),
        ));
    }
    let first_harness = &harnesses[0];
    if harnesses.iter().skip(1).any(|harness| {
        harness.runner_version != first_harness.runner_version
            || harness.pi_version != first_harness.pi_version
            || harness.artifact_revision != first_harness.artifact_revision
    }) {
        return Err(SkillEvalError::InvalidConfiguration(
            "T1 screening cases do not share one runner, Pi, and artifact identity".to_owned(),
        ));
    }
    let environment_manifest = runtime.candidate_environment_manifest.clone();
    let environment_digest = runtime.candidate_environment_manifest_digest.clone();
    let models = preview
        .eligible
        .iter()
        .map(|row| T1ScreenModelState {
            provider: row.provider.clone(),
            model: row.model.clone(),
            attempts: Vec::new(),
            outcome: None,
        })
        .collect();
    let classification_sha256 =
        t1_screen_classification_digest(&preview.eligible, &preview.excluded)?;
    Ok(T1ScreenRunState {
        configuration: T1ScreenRunConfiguration {
            run_id,
            campaign_id: request.campaign_id.clone(),
            created_at: runtime.now(),
            capability_snapshot: preview.snapshot,
            classification_sha256,
            eligible: preview.eligible,
            excluded: preview.excluded,
            exam,
            judge,
            candidate_environment: T1ScreenCandidateEnvironment {
                harnesses,
                manifest: environment_manifest,
                digest: environment_digest,
            },
            policy: T1ScreenPolicy {
                minimum_score: 8,
                calibration_minimum_reliability_basis_points: 8_000,
                maximum_catastrophic_trials: 0,
                repeats_per_case: 1,
                candidate_timeout_seconds: None,
            },
            is_complete_thinking_coverage: true,
            candidate_calls: preview.candidate_calls,
            judge_calls: preview.judge_calls,
            candidate_price: T1ScreenCandidatePrice {
                input_per_million_tokens: 0,
                output_per_million_tokens: 0,
            },
            owner_approved_judge_cap_millionths_of_dollar: request
                .owner_approved_judge_cap_millionths_of_dollar,
            provider_enforced_judge_cap_millionths_of_dollar: request
                .provider_enforced_judge_cap_millionths_of_dollar,
        },
        cap_extensions: Vec::new(),
        route_failures: Vec::new(),
        status: T1ScreenRunStatus::Pending,
        child_runs,
        models,
        candidate_usage: zero_t1_usage(),
        judge_usage: zero_t1_usage(),
        spent_judge_millionths_of_dollar: 0,
        pause: None,
    })
}

fn run_t1_screen_resume(
    run_id: &T1ScreenRunId,
    runs_root: &Path,
    format: T1ScreenFormat,
    output: &mut dyn Write,
) -> Result<(), SkillEvalError> {
    let mut runtime = ConcreteRuntime::new(runs_root)?;
    let stored = runtime.load_t1_screen(run_id)?;
    let pending = pending_t1_screen_state(&stored)?;
    let mut progress = T1CliProgress {
        is_run_id_written: true,
    };
    let state = resume_t1_screening(&pending, &mut runtime, &mut progress)?;
    let report = build_t1_screen_report(&state.configuration.run_id, &runtime, &runtime)?;
    render_t1_screen_report(&report, format, output)
}

fn run_t1_screen_extend_cap(
    request: &T1ScreenCapExtensionRequest,
    runs_root: &Path,
    format: T1ScreenFormat,
    output: &mut dyn Write,
) -> Result<(), SkillEvalError> {
    let repository_root = repository_root()?;
    let mut parent_store = FileT1ScreenStore::open(&repository_root)?;
    let child_store = FileRunStore::open(runs_root)?;
    let report = extend_t1_screen_cap(request, &mut parent_store, &child_store, &T1CliClock)?;
    render_t1_screen_report(&report, format, output)
}

fn run_t1_screen_fail_route(
    request: &T1ScreenRouteFailureRequest,
    runs_root: &Path,
    format: T1ScreenFormat,
    output: &mut dyn Write,
) -> Result<(), SkillEvalError> {
    run_t1_screen_fail_route_at(
        &repository_root()?,
        request,
        runs_root,
        &T1CliClock,
        format,
        output,
    )
}

fn run_t1_screen_fail_route_at(
    repository_root: &Path,
    request: &T1ScreenRouteFailureRequest,
    runs_root: &Path,
    clock: &dyn Clock,
    format: T1ScreenFormat,
    output: &mut dyn Write,
) -> Result<(), SkillEvalError> {
    let mut parent_store = FileT1ScreenStore::open(repository_root)?;
    let child_store = FileRunStore::open(runs_root)?;
    let report = fail_t1_screen_route(request, &mut parent_store, &child_store, clock)?;
    render_t1_screen_report(&report, format, output)
}

struct T1CliClock;

impl Clock for T1CliClock {
    fn now(&self) -> Timestamp {
        local_timestamp()
    }
}

fn run_t1_screen_report(
    run_id: &T1ScreenRunId,
    runs_root: &Path,
    format: T1ScreenFormat,
    output: &mut dyn Write,
) -> Result<(), SkillEvalError> {
    let repository_root = repository_root()?;
    let parent_store = FileT1ScreenStore::open(&repository_root)?;
    let child_store = FileRunStore::open(runs_root)?;
    let report = build_t1_screen_report(run_id, &parent_store, &child_store)?;
    render_t1_screen_report(&report, format, output)
}

struct T1CliProgress {
    is_run_id_written: bool,
}

impl T1ScreenProgressSink for T1CliProgress {
    fn emit_t1_screen(&mut self, state: &T1ScreenRunState) -> Result<(), SkillEvalError> {
        if !self.is_run_id_written {
            write_run_id_value(&state.configuration.run_id.0)?;
            self.is_run_id_written = true;
        }
        Ok(())
    }
}

fn zero_t1_usage() -> TrialUsage {
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

fn run_model_capabilities(
    output_path: &Path,
    output: &mut dyn Write,
) -> Result<(), SkillEvalError> {
    let repository_root = repository_root()?;
    model_capabilities::preflight_output(&repository_root, output_path)?;
    let list_output = command_output("pi", &["--list-models"])?;
    let rpc_output = pi_available_models()?;
    let pi_version = command_output("pi", &["--version"])?;
    let observed_at = model_capabilities::observed_at_unix_seconds()?;
    model_capabilities::capture(
        &repository_root,
        output_path,
        &list_output,
        &rpc_output,
        &pi_version,
        observed_at,
    )?;
    writeln!(
        output,
        "model capabilities written: {}",
        output_path.display()
    )
    .map_err(output_error)
}

fn run_t1_screen_preview(
    capabilities_path: &Path,
    format: T1ScreenFormat,
    output: &mut dyn Write,
) -> Result<(), SkillEvalError> {
    let repository_root = repository_root()?;
    let report = model_capabilities::t1_screen_preview(&repository_root, capabilities_path)?;
    render_t1_screen_preview(&report, format, output)
}

fn render_t1_screen_preview(
    report: &T1ScreenPreviewReport,
    format: T1ScreenFormat,
    output: &mut dyn Write,
) -> Result<(), SkillEvalError> {
    if format == T1ScreenFormat::Json {
        return write_json_line(report, output);
    }
    writeln!(output, "snapshot path: {}", report.snapshot.path.display()).map_err(output_error)?;
    writeln!(output, "snapshot sha256: {}", report.snapshot.sha256).map_err(output_error)?;
    writeln!(output, "snapshot version: {}", report.snapshot.version).map_err(output_error)?;
    writeln!(
        output,
        "snapshot observed at unix seconds: {}",
        report.snapshot.observed_at_unix_seconds
    )
    .map_err(output_error)?;
    writeln!(output, "Pi version: {}", report.snapshot.pi_version).map_err(output_error)?;
    writeln!(output, "total rows: {}", report.total_rows).map_err(output_error)?;
    writeln!(output, "eligible count: {}", report.eligible_count).map_err(output_error)?;
    writeln!(output, "excluded count: {}", report.excluded_count).map_err(output_error)?;
    writeln!(output, "eligible rows:").map_err(output_error)?;
    for row in &report.eligible {
        writeln!(
            output,
            "  {}/{}; preview={}; thinking=[{}]",
            row.provider,
            row.model,
            row.is_preview,
            row.supported_pi_thinking_levels.join(", ")
        )
        .map_err(output_error)?;
    }
    writeln!(output, "excluded rows:").map_err(output_error)?;
    for row in &report.excluded {
        let reasons = row
            .reasons
            .iter()
            .map(|reason| t1_exclusion_reason_label(*reason))
            .collect::<Vec<_>>()
            .join(", ");
        writeln!(
            output,
            "  {}/{}; preview={}; reasons=[{}]",
            row.provider, row.model, row.is_preview, reasons
        )
        .map_err(output_error)?;
    }
    writeln!(output, "exam case count: {}", report.exam_case_count).map_err(output_error)?;
    writeln!(
        output,
        "candidate calls: minimum {}, maximum {}",
        report.candidate_calls.minimum, report.candidate_calls.maximum
    )
    .map_err(output_error)?;
    writeln!(
        output,
        "judge calls: minimum {}, maximum {}",
        report.judge_calls.minimum, report.judge_calls.maximum
    )
    .map_err(output_error)?;
    writeln!(
        output,
        "projected candidate money cost USD: {}",
        report.projected_candidate_money_cost_usd
    )
    .map_err(output_error)?;
    writeln!(output, "judge money: {}", report.judge_money_note).map_err(output_error)
}

fn render_t1_screen_matrix(
    report: &T1ScreenReport,
    output: &mut dyn Write,
) -> Result<(), SkillEvalError> {
    if report.models.is_empty() {
        return Err(t1_matrix_error("configured models are empty"));
    }
    let mut configured = BTreeMap::new();
    for (model_index, model) in report.models.iter().enumerate() {
        if model.provider.trim().is_empty() || model.model.trim().is_empty() {
            return Err(t1_matrix_error("configured model is empty"));
        }
        if configured
            .insert((model.provider.as_str(), model.model.as_str()), model_index)
            .is_some()
        {
            return Err(t1_matrix_error("configured models contain a duplicate"));
        }
    }

    let mut children = BTreeMap::new();
    let mut child_slots = BTreeSet::new();
    let mut configured_children = BTreeSet::new();
    for child in &report.child_runs {
        if child.model.tier != Tier::T1 {
            return Err(t1_matrix_error("child tier differs from T1"));
        }
        thinking_level_index(&child.model.thinking)
            .map_err(|_| t1_matrix_error("child model uses an unknown thinking level"))?;
        let model_index = configured
            .get(&(child.model.provider.as_str(), child.model.model.as_str()))
            .copied()
            .ok_or_else(|| t1_matrix_error("child belongs to a foreign model"))?;
        if child.model_index != u64::try_from(model_index).unwrap_or(u64::MAX) {
            return Err(t1_matrix_error("child model index differs"));
        }
        if children.insert(&child.run_id, child).is_some()
            || !child_slots.insert((child.model_index, child.thinking_index))
        {
            return Err(t1_matrix_error("child identity is duplicated"));
        }
        configured_children.insert(model_index);
    }
    if configured_children.len() != report.models.len() {
        return Err(t1_matrix_error("configured model has no child identity"));
    }

    let mut seen_attempts = BTreeSet::new();
    let mut rows = Vec::with_capacity(report.models.len());
    for model in &report.models {
        let mut cells = [None; RESULT_MATRIX_LEVELS.len()];
        for attempt in &model.attempts {
            if attempt.model.tier != Tier::T1
                || attempt.model.provider != model.provider
                || attempt.model.model != model.model
            {
                return Err(t1_matrix_error("attempt model identity differs"));
            }
            let level_index = thinking_level_index(&attempt.model.thinking)
                .map_err(|_| t1_matrix_error("attempt uses an unknown thinking level"))?;
            let child = children
                .get(&attempt.child_run_id)
                .ok_or_else(|| t1_matrix_error("attempt child identity is unknown"))?;
            if child.model != attempt.model || child.status != attempt.status {
                return Err(t1_matrix_error("attempt child identity differs"));
            }
            if !seen_attempts.insert(&attempt.child_run_id)
                || cells[level_index].is_some()
                || model
                    .attempts
                    .iter()
                    .filter(|candidate| candidate.model.thinking == attempt.model.thinking)
                    .count()
                    != 1
            {
                return Err(t1_matrix_error("attempt evidence is duplicated"));
            }
            if let Some(evidence) = &attempt.evidence {
                if evidence.stage != crate::model::PoolStage::Calibration
                    || evidence.requested_model != attempt.model
                    || evidence.effective_model != attempt.model
                {
                    return Err(t1_matrix_error("attempt evidence identity differs"));
                }
                if attempt.status == crate::model::T1ScreenChildStatus::Completed
                    && evidence.expected_trials > 0
                    && evidence.completed_trials == evidence.expected_trials
                {
                    cells[level_index] = Some(evidence.is_passing);
                }
            }
        }
        rows.push((format!("{}/{}", model.provider, model.model), cells));
    }
    render_result_matrix(&rows, output)
}

fn t1_matrix_error(message: impl Into<String>) -> SkillEvalError {
    SkillEvalError::InvalidConfiguration(format!("malformed T1 result matrix: {}", message.into()))
}

fn render_t1_screen_report(
    report: &T1ScreenReport,
    format: T1ScreenFormat,
    output: &mut dyn Write,
) -> Result<(), SkillEvalError> {
    if format == T1ScreenFormat::Json {
        return write_json_line(report, output);
    }
    render_t1_screen_matrix(report, output)?;
    writeln!(output, "T1 screen {}: {:?}", report.run_id.0, report.status).map_err(output_error)?;
    writeln!(
        output,
        "campaign {}: {:?}; total {}, spent {}, remaining {}; active {}",
        report.campaign_id.0,
        report.campaign_status,
        report.campaign_approved_judge_total_millionths_of_dollar,
        report.campaign_aggregate_judge_spent_millionths_of_dollar,
        report.campaign_remaining_judge_millionths_of_dollar,
        report
            .campaign_active_run_id
            .as_ref()
            .map_or("none", |run_id| run_id.0.as_str())
    )
    .map_err(output_error)?;
    writeln!(output, "campaign runs:").map_err(output_error)?;
    for run in &report.campaign_runs {
        writeln!(
            output,
            "  {}: {:?}; judge {}; candidate {}; resumable={}",
            run.run_id.0,
            run.observed_status,
            run.judge_spend_millionths_of_dollar,
            run.candidate_cost_millionths_of_dollar,
            run.is_resumable
        )
        .map_err(output_error)?;
    }
    writeln!(
        output,
        "inventory: {} total, {} eligible, {} excluded",
        report.total_inventory_count, report.eligible_count, report.excluded_count
    )
    .map_err(output_error)?;
    writeln!(
        output,
        "snapshot: {} sha256 {} version {} observed {} Pi {}",
        report.snapshot.path.display(),
        report.snapshot.sha256,
        report.snapshot.version,
        report.snapshot.observed_at_unix_seconds,
        report.snapshot.pi_version
    )
    .map_err(output_error)?;
    writeln!(output, "eligible identities:").map_err(output_error)?;
    for row in &report.eligible {
        writeln!(
            output,
            "  {}/{}; preview={}; thinking=[{}]",
            row.provider,
            row.model,
            row.is_preview,
            row.supported_pi_thinking_levels.join(", ")
        )
        .map_err(output_error)?;
    }
    writeln!(output, "excluded inventory:").map_err(output_error)?;
    for row in &report.excluded {
        writeln!(
            output,
            "  {}/{}; preview={}; reasons=[{}]",
            row.provider,
            row.model,
            row.is_preview,
            row.reasons
                .iter()
                .map(|reason| t1_exclusion_reason_label(*reason))
                .collect::<Vec<_>>()
                .join(", ")
        )
        .map_err(output_error)?;
    }
    writeln!(
        output,
        "candidate environment: sha256 {}; {} manifest entries",
        report.candidate_environment_manifest_digest,
        report.candidate_environment_manifest_entry_count
    )
    .map_err(output_error)?;
    writeln!(
        output,
        "call projection: candidate {}-{}; judge {}-{}",
        report.candidate_calls.minimum,
        report.candidate_calls.maximum,
        report.judge_calls.minimum,
        report.judge_calls.maximum
    )
    .map_err(output_error)?;
    writeln!(
        output,
        "judge cap: {} spent / {} effective provider / {} effective owner millionths; judge {}",
        report.spent_judge_millionths_of_dollar,
        report.effective_provider_enforced_judge_cap_millionths_of_dollar,
        report.effective_owner_approved_judge_cap_millionths_of_dollar,
        model_label(&report.judge)
    )
    .map_err(output_error)?;
    writeln!(
        output,
        "base judge caps: {} provider / {} owner millionths",
        report.provider_enforced_judge_cap_millionths_of_dollar,
        report.owner_approved_judge_cap_millionths_of_dollar
    )
    .map_err(output_error)?;
    writeln!(output, "cap extension history:").map_err(output_error)?;
    if report.cap_extensions.is_empty() {
        writeln!(output, "  none").map_err(output_error)?;
    }
    for extension in &report.cap_extensions {
        writeln!(
            output,
            "  {}: provider {} -> {}; owner {} -> {}; reason {}",
            extension.timestamp.0,
            extension.previous_provider_cap_millionths_of_dollar,
            extension.new_provider_cap_millionths_of_dollar,
            extension.previous_owner_cap_millionths_of_dollar,
            extension.new_owner_cap_millionths_of_dollar,
            extension.owner_reason
        )
        .map_err(output_error)?;
    }
    writeln!(output, "route failure history:").map_err(output_error)?;
    if report.route_failures.is_empty() {
        writeln!(output, "  none").map_err(output_error)?;
    }
    for failure in &report.route_failures {
        writeln!(
            output,
            "  {}: child {} exact {}; pause sha256 {}; reason {}",
            failure.timestamp.0,
            failure.child_run_id.0,
            model_label(&failure.model),
            failure.paused_message_sha256,
            failure.owner_reason
        )
        .map_err(output_error)?;
    }
    writeln!(
        output,
        "candidate usage: {} input, {} output, {} ms, {} failures priced at {} millionths",
        report.candidate_usage.input_tokens,
        report.candidate_usage.output_tokens,
        report.candidate_usage.elapsed_milliseconds,
        report
            .models
            .iter()
            .flat_map(|model| &model.attempts)
            .filter_map(|attempt| attempt.evidence.as_ref())
            .map(|evidence| u64::from(evidence.failed_trials))
            .sum::<u64>(),
        report.candidate_usage.cost_millionths_of_dollar
    )
    .map_err(output_error)?;
    writeln!(
        output,
        "judge usage: {} input, {} output, {} ms, {} millionths",
        report.judge_usage.input_tokens,
        report.judge_usage.output_tokens,
        report.judge_usage.elapsed_milliseconds,
        report.judge_usage.cost_millionths_of_dollar
    )
    .map_err(output_error)?;
    writeln!(
        output,
        "active child: {}",
        report
            .active_child_run_id
            .as_ref()
            .map_or("none", |run_id| run_id.0.as_str())
    )
    .map_err(output_error)?;
    if let Some(pause) = &report.pause {
        writeln!(output, "pause: {pause:?}").map_err(output_error)?;
    }
    for model in &report.models {
        writeln!(
            output,
            "model {}/{}: {}",
            model.provider,
            model.model,
            match &model.outcome {
                Some(crate::model::T1ScreenModelOutcome::Selected { model }) => {
                    format!("selected {}", model_label(model))
                }
                Some(crate::model::T1ScreenModelOutcome::Exhausted) => "exhausted".to_owned(),
                Some(crate::model::T1ScreenModelOutcome::InfrastructureFailed {
                    model,
                    child_run_id,
                }) => format!(
                    "infrastructure_failed {} child {}",
                    model_label(model),
                    child_run_id.0
                ),
                None => "pending".to_owned(),
            }
        )
        .map_err(output_error)?;
        for (attempt_index, attempt) in model.attempts.iter().enumerate() {
            writeln!(
                output,
                "  attempt {} child {} exact {} status {:?}",
                attempt_index + 1,
                attempt.child_run_id.0,
                model_label(&attempt.model),
                attempt.status
            )
            .map_err(output_error)?;
            if let Some(evidence) = &attempt.evidence {
                writeln!(
                    output,
                    "    result {} score {:.2} [{:.2}, {:.2}] reliability {}/{} failures {} catastrophic {}; candidate {} millionths {} ms; judge {} millionths {} ms",
                    if evidence.is_passing { "pass" } else { "fail" },
                    evidence.score.estimate,
                    evidence.score.lower,
                    evidence.score.upper,
                    evidence.completed_trials - evidence.failed_trials,
                    evidence.completed_trials,
                    evidence.failed_trials,
                    evidence.catastrophic_trials,
                    evidence.candidate_usage.cost_millionths_of_dollar,
                    evidence.candidate_usage.elapsed_milliseconds,
                    evidence.judge_usage.cost_millionths_of_dollar,
                    evidence.judge_usage.elapsed_milliseconds
                )
                .map_err(output_error)?;
            }
            for case in &attempt.cases {
                match &case.trial {
                    Some(trial) => {
                        writeln!(
                            output,
                            "    case {}: score {} catastrophic {} failure {:?}; candidate {} millionths {} ms; judge {} millionths {} ms",
                            case.case.0,
                            trial.verdict.score,
                            trial.verdict.is_catastrophic,
                            trial.verdict.failure_mode,
                            trial.candidate_usage.cost_millionths_of_dollar,
                            trial.candidate_usage.elapsed_milliseconds,
                            trial.judge_usage.cost_millionths_of_dollar,
                            trial.judge_usage.elapsed_milliseconds
                        )
                        .map_err(output_error)?;
                        for check in &trial.verdict.checks {
                            writeln!(
                                output,
                                "      check {}: {:?}; detail {:?}",
                                check.name, check.status, check.detail
                            )
                            .map_err(output_error)?;
                        }
                    }
                    None if case.candidate.is_some() => writeln!(
                        output,
                        "    case {}: candidate checkpoint saved; judge pending",
                        case.case.0
                    )
                    .map_err(output_error)?,
                    None => writeln!(output, "    case {}: pending", case.case.0)
                        .map_err(output_error)?,
                }
            }
        }
    }
    match &report.ranking {
        None => writeln!(
            output,
            "ranking: unavailable until every eligible model is terminal"
        )
        .map_err(output_error)?,
        Some(ranking) if ranking.recommendation_shortage_count > 0 => {
            writeln!(
                output,
                "ranking: no recommendation; {} more passing route(s) required",
                ranking.recommendation_shortage_count
            )
            .map_err(output_error)?;
            render_t1_ranked_routes("ordered passing routes", &ranking.alternates, output)?;
        }
        Some(ranking) => {
            render_t1_ranked_routes("recommendations", &ranking.recommendations, output)?;
            render_t1_ranked_routes("ordered alternates", &ranking.alternates, output)?;
        }
    }
    writeln!(
        output,
        "owner approval required: {}",
        report.is_owner_approval_required
    )
    .map_err(output_error)
}

fn render_t1_ranked_routes(
    label: &str,
    routes: &[crate::model::T1ScreenRankedRoute],
    output: &mut dyn Write,
) -> Result<(), SkillEvalError> {
    writeln!(output, "{label}:").map_err(output_error)?;
    if routes.is_empty() {
        writeln!(output, "  none").map_err(output_error)?;
    }
    for route in routes {
        let inputs = &route.ranking_inputs;
        writeln!(
            output,
            "  {}. {}; candidate cost {} millionths, latency {} ms, failures {}/{}; judge overhead ignored",
            route.rank,
            model_label(&route.model),
            inputs.candidate_cost_millionths_of_dollar,
            inputs.candidate_latency_milliseconds,
            inputs.candidate_failed_trials,
            inputs.candidate_completed_trials
        )
        .map_err(output_error)?;
    }
    Ok(())
}

fn validate_tracked_capability_snapshot(
    repository_root: &Path,
    relative: &Path,
    expected_sha256: &str,
) -> Result<(), SkillEvalError> {
    let path = relative.to_string_lossy();
    let output = Command::new("git")
        .arg("-C")
        .arg(repository_root)
        .args(["show", &format!("HEAD:{path}")])
        .output()
        .map_err(|error| SkillEvalError::Process {
            program: "git".to_owned(),
            exit_code: None,
            standard_error: error.to_string(),
        })?;
    if !output.status.success() {
        return Err(SkillEvalError::InvalidConfiguration(format!(
            "T1 capability snapshot {} is not tracked at HEAD",
            relative.display()
        )));
    }
    let tracked_digest = Sha256::digest(&output.stdout)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if tracked_digest != expected_sha256 {
        return Err(SkillEvalError::InvalidConfiguration(
            "T1 capability snapshot bytes differ from the tracked snapshot".to_owned(),
        ));
    }
    Ok(())
}

fn preflight_run_id_file() -> Result<(), SkillEvalError> {
    let path = RUN_ID_FILE.with(|slot| slot.borrow().clone());
    let Some(path) = path else {
        return Ok(());
    };
    match fs::symlink_metadata(&path) {
        Ok(_) => {
            return Err(invalid(format!(
                "run-id file {} already exists",
                path.display()
            )));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(SkillEvalError::Io {
                path,
                message: error.to_string(),
            });
        }
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut current = if parent.is_absolute() {
        PathBuf::from("/")
    } else {
        PathBuf::from(".")
    };
    for component in parent.components() {
        if matches!(component, Component::RootDir | Component::CurDir) {
            continue;
        }
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(invalid(format!(
                    "run-id file parent {} is not a real directory",
                    current.display()
                )));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => {
                return Err(SkillEvalError::Io {
                    path: current,
                    message: error.to_string(),
                });
            }
        }
    }
    Ok(())
}

fn t1_exclusion_reason_label(reason: T1ScreenExclusionReason) -> &'static str {
    match reason {
        T1ScreenExclusionReason::MissingList => "missing_list",
        T1ScreenExclusionReason::MissingRpc => "missing_rpc",
        T1ScreenExclusionReason::MovingAlias => "moving_alias",
        T1ScreenExclusionReason::NotExactEvidence => "not_exact_evidence",
        T1ScreenExclusionReason::MovingRouterOrControl => "moving_router_or_control",
        T1ScreenExclusionReason::MissingPrice => "missing_price",
        T1ScreenExclusionReason::NonzeroInputPrice => "nonzero_input_price",
        T1ScreenExclusionReason::NonzeroOutputPrice => "nonzero_output_price",
        T1ScreenExclusionReason::MissingTextInput => "missing_text_input",
        T1ScreenExclusionReason::MissingThinkingLevels => "missing_thinking_levels",
        T1ScreenExclusionReason::MalformedThinkingLevels => "malformed_thinking_levels",
    }
}

fn print_help(output: &mut dyn Write) -> std::io::Result<()> {
    writeln!(output, "skill-eval <command> [options]")?;
    writeln!(output, "commands:")?;
    for command in [
        "model-capabilities",
        "t1-screen-preview",
        "t1-screen-campaign-create",
        "t1-screen-campaign-extend-cap",
        "t1-screen-campaign-retire-run",
        "t1-screen-start",
        "t1-screen-resume",
        "t1-screen-extend-cap",
        "t1-screen-fail-route",
        "t1-screen-report",
        "qualify",
        "report",
        "inspect",
        "resume",
        "decide",
        "apply",
        "audit-briefs",
        "judge",
        "pool-qualify",
        "pool-report",
        "pool-resume",
        "pool-replacement",
        "frontier-suite-inventory",
        "frontier-suite-propose",
        "frontier-suite-check",
        "frontier-suite-apply",
        "frontier-preview",
        "frontier-start",
        "frontier-resume",
        "frontier-report",
        "frontier-inspect",
        "frontier-decide",
        "frontier-apply",
    ] {
        writeln!(output, "  {command}")?;
    }
    writeln!(output, "model catalog: model-capabilities --output PATH")?;
    writeln!(
        output,
        "T1 preview: t1-screen-preview --capabilities PATH [--format text|json]"
    )?;
    writeln!(
        output,
        "T1 campaign import: t1-screen-campaign-create --campaign ID --judge-cap-millionths 20000000 --reason TEXT --run ID... [--format text|json]"
    )?;
    writeln!(
        output,
        "T1 campaign extension: t1-screen-campaign-extend-cap --campaign ID --judge-cap-millionths N --reason TEXT [--format text|json]"
    )?;
    writeln!(
        output,
        "T1 paused-run retirement: t1-screen-campaign-retire-run --campaign ID --run ID --reason TEXT [--format text|json]"
    )?;
    writeln!(
        output,
        "T1 start: t1-screen-start --campaign ID --capabilities PATH --exam PATH --judge-cap-millionths N --provider-cap-millionths N [--run-id-file PATH] [--format text|json]"
    )?;
    writeln!(
        output,
        "T1 continue: t1-screen-resume --run ID [--format text|json]; t1-screen-extend-cap --run ID --judge-cap-millionths N --provider-cap-millionths N --reason TEXT [--format text|json]; t1-screen-fail-route --run PARENT --child CHILD --reason TEXT [--format text|json]; t1-screen-report --run ID [--format text|json]"
    )?;
    writeln!(
        output,
        "suite construction: frontier-suite-inventory --plan PATH --output PATH; frontier-suite-propose --plan PATH --inventory PATH --reviews PATH --output PATH; frontier-suite-check --proposal PATH; frontier-suite-apply --proposal PATH --output PATH"
    )?;
    writeln!(
        output,
        "frontier: frontier-preview --plan PATH; frontier-start --plan PATH [--run-id-file PATH]; frontier-resume|frontier-report|frontier-apply --run ID; frontier-inspect --run ID --provider NAME --model NAME --tier TIER --thinking LEVEL --artifact NAME --case ID --attempt N; frontier-decide --run ID (--accept|--reject) --reason TEXT"
    )?;
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
        "pool options: --plan PATH --artifact PATH... [--tiers TIER...] [--dry-run] [--run-id-file PATH]; replacement: --run ID --entrant-index N [--run-id-file PATH]"
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

fn parse_t1_screen_campaign_id(value: &str) -> Result<T1ScreenCampaignId, SkillEvalError> {
    nonempty(value, "T1 screening campaign identifier")?;
    if !value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err(invalid(format!(
            "T1 screening campaign identifier {value:?} is not a safe path component"
        )));
    }
    Ok(T1ScreenCampaignId(value.to_owned()))
}

fn parse_t1_screen_run_id(value: &str) -> Result<T1ScreenRunId, SkillEvalError> {
    nonempty(value, "T1 screening run identifier")?;
    if !is_t1_screen_identifier(value) {
        return Err(invalid(
            "T1 screening run identifier must be one safe path component",
        ));
    }
    Ok(T1ScreenRunId(value.to_owned()))
}

fn parse_t1_screen_child_run_id(value: &str) -> Result<RunId, SkillEvalError> {
    nonempty(value, "T1 screening child identifier")?;
    if !is_t1_screen_identifier(value) {
        return Err(invalid(
            "T1 screening child identifier must be one safe path component",
        ));
    }
    Ok(RunId(value.to_owned()))
}

fn is_t1_screen_identifier(value: &str) -> bool {
    value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
}

fn parse_pool_run_id(value: &str) -> Result<PoolRunId, SkillEvalError> {
    nonempty(value, "pool run identifier")?;
    let path = Path::new(value);
    if path.is_absolute()
        || path.components().count() != 1
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err(invalid(
            "pool run identifier must be one safe path component",
        ));
    }
    Ok(PoolRunId(value.to_owned()))
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

fn pi_available_models() -> Result<String, SkillEvalError> {
    let mut command = pi_available_models_command("pi");
    let mut child = command.spawn().map_err(|error| SkillEvalError::Process {
        program: "pi".to_owned(),
        exit_code: None,
        standard_error: error.to_string(),
    })?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| invalid_configuration("Pi RPC capability probe has no stdin"))?;
    let write_result = stdin.write_all(MODELS_RPC_REQUEST);
    drop(stdin);
    let output = child
        .wait_with_output()
        .map_err(|error| SkillEvalError::Process {
            program: "pi".to_owned(),
            exit_code: None,
            standard_error: error.to_string(),
        })?;
    if !output.status.success() {
        return Err(SkillEvalError::Process {
            program: "pi".to_owned(),
            exit_code: output.status.code(),
            standard_error: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }
    write_result.map_err(|error| SkillEvalError::Process {
        program: "pi".to_owned(),
        exit_code: output.status.code(),
        standard_error: format!("failed to write Pi RPC capability request: {error}"),
    })?;
    let stdout = String::from_utf8(output.stdout).map_err(|error| {
        invalid_configuration(format!("Pi RPC capability output is not UTF-8: {error}"))
    })?;
    parse_available_models_response(&stdout)
}

fn pi_available_models_command(program: &str) -> Command {
    let mut command = Command::new(program);
    command
        .args(MODELS_RPC_ARGUMENTS)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

fn parse_available_models_response(output: &str) -> Result<String, SkillEvalError> {
    let mut response = None;
    for raw_line in output.split('\n') {
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        if line.is_empty() {
            continue;
        }
        let value: serde_json::Value = serde_json::from_str(line).map_err(|error| {
            invalid_configuration(format!(
                "Pi RPC capability output is malformed JSONL: {error}"
            ))
        })?;
        if response.is_some() {
            return Err(invalid_configuration(
                "Pi RPC capability output contains duplicate responses",
            ));
        }
        if value.get("type").and_then(serde_json::Value::as_str) != Some("response") {
            return Err(invalid_configuration(
                "Pi RPC capability output contains a non-response record",
            ));
        }
        if value.get("id").and_then(serde_json::Value::as_str) != Some(MODELS_RPC_ID) {
            return Err(invalid_configuration(
                "Pi RPC capability response has the wrong id",
            ));
        }
        if value.get("command").and_then(serde_json::Value::as_str) != Some("get_available_models")
        {
            return Err(invalid_configuration(
                "Pi RPC capability response has the wrong command",
            ));
        }
        if value.get("success").and_then(serde_json::Value::as_bool) != Some(true) {
            return Err(invalid_configuration(
                "Pi RPC capability response was not successful",
            ));
        }
        let data = value
            .get("data")
            .ok_or_else(|| invalid_configuration("Pi RPC capability response is missing data"))?;
        let data = serde_json::to_string(data).map_err(|error| {
            invalid_configuration(format!(
                "Pi RPC capability data cannot be serialized: {error}"
            ))
        })?;
        validate_rpc_models_data(&data)?;
        response = Some(data);
    }
    response.ok_or_else(|| invalid_configuration("Pi RPC capability response is missing"))
}

fn invalid_configuration(message: impl Into<String>) -> SkillEvalError {
    SkillEvalError::InvalidConfiguration(message.into())
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

fn sha256_digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
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
    write_run_id_value(&run_id.0)
}

fn write_run_id_value(run_id: &str) -> Result<(), SkillEvalError> {
    let path = RUN_ID_FILE.with(|slot| slot.borrow_mut().take());
    let Some(path) = path else {
        return Ok(());
    };
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|error| SkillEvalError::Io {
        path: parent.to_path_buf(),
        message: error.to_string(),
    })?;
    let file_name = path
        .file_name()
        .ok_or_else(|| invalid("run-id file requires a file name"))?
        .to_string_lossy();
    let sequence = RUN_ID_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temporary = parent.join(format!(
        ".{file_name}.{}.{sequence}.tmp",
        std::process::id()
    ));
    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|error| SkillEvalError::Io {
                path: temporary.clone(),
                message: error.to_string(),
            })?;
        file.write_all(format!("{run_id}\n").as_bytes())
            .and_then(|()| file.sync_all())
            .map_err(|error| SkillEvalError::Io {
                path: temporary.clone(),
                message: error.to_string(),
            })?;
        fs::hard_link(&temporary, &path).map_err(|error| SkillEvalError::Io {
            path: path.clone(),
            message: error.to_string(),
        })?;
        fs::remove_file(&temporary).map_err(|error| SkillEvalError::Io {
            path: temporary.clone(),
            message: error.to_string(),
        })?;
        fs::File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| SkillEvalError::Io {
                path: parent.to_path_buf(),
                message: error.to_string(),
            })
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn write_json_line<T: Serialize + ?Sized>(
    value: &T,
    output: &mut dyn Write,
) -> Result<(), SkillEvalError> {
    serde_json::to_writer(&mut *output, value).map_err(|error| {
        if error.is_io() {
            SkillEvalError::Io {
                path: PathBuf::from("<stdout>"),
                message: error.to_string(),
            }
        } else {
            SkillEvalError::InvalidConfiguration(format!("output serialization failed: {error}"))
        }
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

fn tier_label(tier: Tier) -> &'static str {
    match tier {
        Tier::T1 => "T1",
        Tier::T2 => "T2",
        Tier::T3 => "T3",
        Tier::T4 => "T4",
        Tier::T5 => "T5",
    }
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

fn validate_repository_path(path: &Path, label: &str) -> Result<(), SkillEvalError> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        || !path.to_string_lossy().chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '/' | '-' | '_' | '.')
        })
    {
        return Err(invalid(format!(
            "{label} must be a safe repository-relative path"
        )));
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

fn ensure_unique_tiers(tiers: &[Tier]) -> Result<(), SkillEvalError> {
    let mut seen = BTreeSet::new();
    if tiers.iter().any(|tier| !seen.insert(*tier)) {
        return Err(invalid("selected tiers must be unique"));
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

fn parse_frontier_suite_inventory(
    parser: &mut ArgumentParser<'_>,
) -> Result<CliCommand, SkillEvalError> {
    let mut plan_path = None;
    let mut output = None;
    while parser.peek().is_some() {
        let flag = parser.peek().expect("checked above").to_owned();
        match flag.as_str() {
            "--plan" => {
                let path = PathBuf::from(parser.value_once("--plan")?);
                validate_repository_path(&path, "frontier suite plan")?;
                plan_path = Some(path);
            }
            "--output" => {
                let path = PathBuf::from(parser.value_once("--output")?);
                validate_repository_path(&path, "frontier suite inventory output")?;
                output = Some(path);
            }
            _ if parser.take_common()? => {}
            _ => break,
        }
    }
    Ok(CliCommand::FrontierSuiteInventory {
        plan_path: plan_path.ok_or_else(|| invalid("frontier-suite-inventory requires --plan"))?,
        output: output.ok_or_else(|| invalid("frontier-suite-inventory requires --output"))?,
    })
}

fn parse_frontier_suite_propose(
    parser: &mut ArgumentParser<'_>,
) -> Result<CliCommand, SkillEvalError> {
    let mut plan_path = None;
    let mut inventory_path = None;
    let mut review_set_path = None;
    let mut output = None;
    while parser.peek().is_some() {
        let flag = parser.peek().expect("checked above").to_owned();
        match flag.as_str() {
            "--plan" => {
                let path = PathBuf::from(parser.value_once("--plan")?);
                validate_repository_path(&path, "frontier suite plan")?;
                plan_path = Some(path);
            }
            "--inventory" => {
                let path = PathBuf::from(parser.value_once("--inventory")?);
                validate_repository_path(&path, "frontier suite inventory")?;
                inventory_path = Some(path);
            }
            "--reviews" => {
                let path = PathBuf::from(parser.value_once("--reviews")?);
                validate_repository_path(&path, "frontier suite reviews")?;
                review_set_path = Some(path);
            }
            "--output" => {
                let path = PathBuf::from(parser.value_once("--output")?);
                validate_repository_path(&path, "frontier suite proposal output")?;
                output = Some(path);
            }
            _ if parser.take_common()? => {}
            _ => break,
        }
    }
    Ok(CliCommand::FrontierSuitePropose {
        plan_path: plan_path.ok_or_else(|| invalid("frontier-suite-propose requires --plan"))?,
        inventory_path: inventory_path
            .ok_or_else(|| invalid("frontier-suite-propose requires --inventory"))?,
        review_set_path: review_set_path
            .ok_or_else(|| invalid("frontier-suite-propose requires --reviews"))?,
        output: output.ok_or_else(|| invalid("frontier-suite-propose requires --output"))?,
    })
}

fn parse_frontier_suite_check(
    parser: &mut ArgumentParser<'_>,
) -> Result<CliCommand, SkillEvalError> {
    let mut proposal_path = None;
    while parser.peek().is_some() {
        let flag = parser.peek().expect("checked above").to_owned();
        match flag.as_str() {
            "--proposal" => {
                let path = PathBuf::from(parser.value_once("--proposal")?);
                validate_repository_path(&path, "frontier suite proposal")?;
                proposal_path = Some(path);
            }
            _ if parser.take_common()? => {}
            _ => break,
        }
    }
    Ok(CliCommand::FrontierSuiteCheck {
        proposal_path: proposal_path
            .ok_or_else(|| invalid("frontier-suite-check requires --proposal"))?,
    })
}

fn parse_frontier_suite_apply(
    parser: &mut ArgumentParser<'_>,
) -> Result<CliCommand, SkillEvalError> {
    let mut proposal_path = None;
    let mut output = None;
    while parser.peek().is_some() {
        let flag = parser.peek().expect("checked above").to_owned();
        match flag.as_str() {
            "--proposal" => {
                let path = PathBuf::from(parser.value_once("--proposal")?);
                validate_repository_path(&path, "frontier suite proposal")?;
                proposal_path = Some(path);
            }
            "--output" => {
                let path = PathBuf::from(parser.value_once("--output")?);
                validate_repository_path(&path, "frontier suite output")?;
                output = Some(path);
            }
            _ if parser.take_common()? => {}
            _ => break,
        }
    }
    Ok(CliCommand::FrontierSuiteApply {
        proposal_path: proposal_path
            .ok_or_else(|| invalid("frontier-suite-apply requires --proposal"))?,
        output: output.ok_or_else(|| invalid("frontier-suite-apply requires --output"))?,
    })
}

/// Dispatches one complete-bank suite command without candidate, judge, or Pi execution.
///
/// The inputs are a parsed command, output format, suite runtime, and output writer. The function
/// produces the selected inventory, proposal, capacity report, or publication rendering.
///
/// # Errors
///
/// Returns an error for a non-suite command, invalid input, source or digest drift, incomplete
/// review, blocked publication, unsafe path, or failed read, write, or rendering operation.
pub(crate) fn execute_frontier_suite_command(
    command: &CliCommand,
    format: OutputFormat,
    runtime: &mut dyn FrontierSuiteRuntime,
    output: &mut dyn Write,
) -> Result<(), SkillEvalError> {
    match command {
        CliCommand::FrontierSuiteInventory {
            plan_path,
            output: inventory_path,
        } => render_frontier_suite_inventory(
            &inventory_frontier_suite(plan_path, inventory_path, runtime)?,
            format,
            output,
        ),
        CliCommand::FrontierSuitePropose {
            plan_path,
            inventory_path,
            review_set_path,
            output: proposal_path,
        } => render_frontier_suite_proposal(
            &propose_frontier_suite(
                plan_path,
                inventory_path,
                review_set_path,
                proposal_path,
                runtime,
            )?,
            format,
            output,
        ),
        CliCommand::FrontierSuiteCheck { proposal_path } => render_frontier_suite_proposal(
            &check_frontier_suite(proposal_path, runtime)?,
            format,
            output,
        ),
        CliCommand::FrontierSuiteApply {
            proposal_path,
            output: suite_path,
        } => render_frontier_suite_publication(
            &apply_frontier_suite(proposal_path, suite_path, runtime)?,
            format,
            output,
        ),
        _ => Err(invalid("command is not a complete-bank suite command")),
    }
}

fn render_frontier_suite_inventory(
    inventory: &FrontierSuiteInventory,
    format: OutputFormat,
    output: &mut dyn Write,
) -> Result<(), SkillEvalError> {
    if format == OutputFormat::JsonLines {
        return write_json_line(inventory, output);
    }
    writeln!(output, "version: {}", inventory.version).map_err(output_error)?;
    writeln!(output, "generated at: {}", inventory.generated_at.0).map_err(output_error)?;
    writeln!(output, "case count: {}", inventory.cases.len()).map_err(output_error)?;
    for entry in &inventory.cases {
        writeln!(
            output,
            "case: {}@{} {} holdout={} drive={:?}",
            entry.key.artifact_path.display(),
            entry.key.artifact_revision,
            entry.key.case.0,
            entry.is_holdout,
            entry.drive,
        )
        .map_err(output_error)?;
    }
    Ok(())
}

fn render_frontier_suite_proposal(
    proposal: &FrontierSuiteProposal,
    format: OutputFormat,
    output: &mut dyn Write,
) -> Result<(), SkillEvalError> {
    if format == OutputFormat::JsonLines {
        return write_json_line(proposal, output);
    }
    writeln!(output, "status: {:?}", proposal.status).map_err(output_error)?;
    for tier in &proposal.policy.required_tiers {
        let capacity = proposal.tier_capacity.get(tier).ok_or_else(|| {
            SkillEvalError::InvalidConfiguration(format!(
                "frontier proposal has no capacity for required tier {tier:?}"
            ))
        })?;
        writeln!(
            output,
            "{tier:?}: accepted {}, required {}, shortfall {}, duplicates {}, rejects {}, complete {}",
            capacity.accepted_unique_cases,
            capacity.required_unique_cases,
            capacity.shortfall,
            capacity.duplicate_cases,
            capacity.rejected_cases,
            capacity.is_complete,
        )
        .map_err(output_error)?;
    }
    write!(output, "weights:").map_err(output_error)?;
    for group in [
        FrontierCaseGroup::Normal,
        FrontierCaseGroup::Edge,
        FrontierCaseGroup::Adversarial,
        FrontierCaseGroup::Critical,
    ] {
        let weight = proposal
            .policy
            .group_weights_basis_points
            .get(&group)
            .ok_or_else(|| {
                SkillEvalError::InvalidConfiguration(format!(
                    "frontier proposal has no weight for group {group:?}"
                ))
            })?;
        write!(output, " {}={weight}", frontier_group_label(group)).map_err(output_error)?;
    }
    writeln!(output).map_err(output_error)?;
    writeln!(output, "holdout cases: {}", proposal.holdout_cases.len()).map_err(output_error)?;
    writeln!(
        output,
        "calibration anchors: {}",
        proposal.calibration_anchors.len()
    )
    .map_err(output_error)?;
    for tier in &proposal.policy.required_tiers {
        let suite = proposal.proposed_tiers.get(tier).ok_or_else(|| {
            SkillEvalError::InvalidConfiguration(format!(
                "frontier proposal has no cases for required tier {tier:?}"
            ))
        })?;
        for case in &suite.cases {
            writeln!(
                output,
                "case {tier:?}: {}@{} {} group={} confirmation={}",
                case.artifact_path.display(),
                case.artifact_revision,
                case.case.0,
                frontier_group_label(case.group),
                case.is_confirmation,
            )
            .map_err(output_error)?;
        }
    }
    Ok(())
}

fn frontier_group_label(group: FrontierCaseGroup) -> &'static str {
    match group {
        FrontierCaseGroup::Normal => "normal",
        FrontierCaseGroup::Edge => "edge",
        FrontierCaseGroup::Adversarial => "adversarial",
        FrontierCaseGroup::Critical => "critical",
    }
}

fn render_frontier_suite_publication(
    publication: &FrontierSuitePublication,
    format: OutputFormat,
    output: &mut dyn Write,
) -> Result<(), SkillEvalError> {
    if format == OutputFormat::JsonLines {
        return write_json_line(publication, output);
    }
    writeln!(output, "proposal digest: {}", publication.proposal_sha256).map_err(output_error)?;
    writeln!(output, "suite path: {}", publication.suite_path.display()).map_err(output_error)?;
    writeln!(output, "suite digest: {}", publication.suite_sha256).map_err(output_error)?;
    writeln!(output, "published at: {}", publication.published_at.0).map_err(output_error)
}

fn parse_frontier_preview(parser: &mut ArgumentParser<'_>) -> Result<CliCommand, SkillEvalError> {
    Ok(CliCommand::FrontierPreview {
        plan_path: parse_frontier_plan(parser, "frontier-preview")?,
    })
}

fn parse_frontier_start(parser: &mut ArgumentParser<'_>) -> Result<CliCommand, SkillEvalError> {
    let mut plan_path = None;
    while parser.peek().is_some() {
        let flag = parser.peek().expect("checked above").to_owned();
        match flag.as_str() {
            "--plan" => {
                let path = PathBuf::from(parser.value_once("--plan")?);
                validate_repository_path(&path, "frontier plan")?;
                plan_path = Some(path);
            }
            "--run-id-file" => {
                let path = PathBuf::from(parser.value_once("--run-id-file")?);
                validate_repository_path(&path, "run-id file")?;
                RUN_ID_FILE.with(|slot| *slot.borrow_mut() = Some(path));
            }
            _ if parser.take_common()? => {}
            _ => break,
        }
    }
    Ok(CliCommand::FrontierStart {
        plan_path: plan_path.ok_or_else(|| invalid("frontier-start requires --plan"))?,
    })
}

fn parse_frontier_plan(
    parser: &mut ArgumentParser<'_>,
    command: &str,
) -> Result<PathBuf, SkillEvalError> {
    let mut plan_path = None;
    while parser.peek().is_some() {
        let flag = parser.peek().expect("checked above").to_owned();
        match flag.as_str() {
            "--plan" => {
                let path = PathBuf::from(parser.value_once("--plan")?);
                validate_repository_path(&path, "frontier plan")?;
                plan_path = Some(path);
            }
            _ if parser.take_common()? => {}
            _ => break,
        }
    }
    plan_path.ok_or_else(|| invalid(format!("{command} requires --plan")))
}

fn parse_frontier_resume(parser: &mut ArgumentParser<'_>) -> Result<CliCommand, SkillEvalError> {
    Ok(CliCommand::FrontierResume {
        run_id: parse_frontier_run(parser, "frontier-resume")?,
    })
}

fn parse_frontier_report(parser: &mut ArgumentParser<'_>) -> Result<CliCommand, SkillEvalError> {
    let mut run_id = None;
    let mut baseline_path = None;
    while parser.peek().is_some() {
        let flag = parser.peek().expect("checked above").to_owned();
        match flag.as_str() {
            "--run" => {
                run_id = Some(parse_frontier_run_id(parser.value_once("--run")?)?);
            }
            "--baseline" => {
                let path = PathBuf::from(parser.value_once("--baseline")?);
                validate_repository_path(&path, "frontier baseline")?;
                baseline_path = Some(path);
            }
            _ if parser.take_common()? => {}
            _ => break,
        }
    }
    Ok(CliCommand::FrontierReport {
        run_id: run_id.ok_or_else(|| invalid("frontier-report requires --run"))?,
        baseline_path,
    })
}

fn parse_frontier_inspect(parser: &mut ArgumentParser<'_>) -> Result<CliCommand, SkillEvalError> {
    let mut run_id = None;
    let mut provider = None;
    let mut model = None;
    let mut tier = None;
    let mut thinking = None;
    let mut artifact = None;
    let mut case = None;
    let mut attempt = None;
    while parser.peek().is_some() {
        let flag = parser.peek().expect("checked above").to_owned();
        match flag.as_str() {
            "--run" => run_id = Some(parse_frontier_run_id(parser.value_once("--run")?)?),
            "--provider" => {
                provider = Some(parse_frontier_name(
                    parser.value_once("--provider")?,
                    "frontier provider",
                )?);
            }
            "--model" => {
                model = Some(parse_frontier_name(
                    parser.value_once("--model")?,
                    "frontier model",
                )?);
            }
            "--tier" => tier = Some(parse_tier(parser.value_once("--tier")?)?),
            "--thinking" => {
                thinking = Some(parse_frontier_thinking(parser.value_once("--thinking")?)?);
            }
            "--artifact" => {
                artifact = Some(ArtifactName(parse_frontier_name(
                    parser.value_once("--artifact")?,
                    "frontier artifact",
                )?));
            }
            "--case" => {
                case = Some(CaseId(parse_frontier_name(
                    parser.value_once("--case")?,
                    "frontier case",
                )?));
            }
            "--attempt" => {
                attempt = Some(parse_positive_frontier_attempt(
                    parser.value_once("--attempt")?,
                )?);
            }
            _ if parser.take_common()? => {}
            _ => break,
        }
    }
    Ok(CliCommand::FrontierInspect {
        selector: FrontierTrialSelector {
            run_id: run_id.ok_or_else(|| invalid("frontier-inspect requires --run"))?,
            provider: provider.ok_or_else(|| invalid("frontier-inspect requires --provider"))?,
            model: model.ok_or_else(|| invalid("frontier-inspect requires --model"))?,
            tier: tier.ok_or_else(|| invalid("frontier-inspect requires --tier"))?,
            thinking: thinking.ok_or_else(|| invalid("frontier-inspect requires --thinking"))?,
            artifact: artifact.ok_or_else(|| invalid("frontier-inspect requires --artifact"))?,
            case: case.ok_or_else(|| invalid("frontier-inspect requires --case"))?,
            attempt: attempt.ok_or_else(|| invalid("frontier-inspect requires --attempt"))?,
        },
    })
}

fn parse_frontier_decide(parser: &mut ArgumentParser<'_>) -> Result<CliCommand, SkillEvalError> {
    let mut run_id = None;
    let mut decision = None;
    let mut reason = None;
    while parser.peek().is_some() {
        let flag = parser.peek().expect("checked above").to_owned();
        match flag.as_str() {
            "--run" => run_id = Some(parse_frontier_run_id(parser.value_once("--run")?)?),
            "--accept" => {
                parser.take_once("--accept")?;
                set_decision(&mut decision, Decision::Accepted)?;
            }
            "--reject" => {
                parser.take_once("--reject")?;
                set_decision(&mut decision, Decision::Rejected)?;
            }
            "--reason" => {
                reason = Some(
                    nonempty(parser.value_once("--reason")?, "frontier decision reason")?
                        .trim()
                        .to_owned(),
                );
            }
            _ if parser.take_common()? => {}
            _ => break,
        }
    }
    Ok(CliCommand::FrontierDecide {
        request: FrontierDecisionRequest {
            run_id: run_id.ok_or_else(|| invalid("frontier-decide requires --run"))?,
            decision: decision
                .ok_or_else(|| invalid("frontier-decide requires --accept or --reject"))?,
            reason: reason.ok_or_else(|| invalid("frontier-decide requires --reason"))?,
        },
    })
}

fn parse_frontier_apply(parser: &mut ArgumentParser<'_>) -> Result<CliCommand, SkillEvalError> {
    Ok(CliCommand::FrontierApply {
        run_id: parse_frontier_run(parser, "frontier-apply")?,
    })
}

fn parse_frontier_run(
    parser: &mut ArgumentParser<'_>,
    command: &str,
) -> Result<FrontierRunId, SkillEvalError> {
    let mut run_id = None;
    while parser.peek().is_some() {
        let flag = parser.peek().expect("checked above").to_owned();
        match flag.as_str() {
            "--run" => run_id = Some(parse_frontier_run_id(parser.value_once("--run")?)?),
            _ if parser.take_common()? => {}
            _ => break,
        }
    }
    run_id.ok_or_else(|| invalid(format!("{command} requires --run")))
}

fn parse_frontier_run_id(value: &str) -> Result<FrontierRunId, SkillEvalError> {
    Ok(FrontierRunId(parse_frontier_name(
        value,
        "frontier run identifier",
    )?))
}

fn parse_frontier_name(value: &str, label: &str) -> Result<String, SkillEvalError> {
    let value = nonempty(value, label)?;
    if value != value.trim()
        || matches!(value, "." | "..")
        || !value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
    {
        return Err(invalid(format!(
            "{label} must be one safe exact identifier"
        )));
    }
    Ok(value.to_owned())
}

fn parse_frontier_thinking(value: &str) -> Result<String, SkillEvalError> {
    match value {
        "off" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max" => Ok(value.to_owned()),
        _ => Err(invalid(format!(
            "unknown frontier thinking level {value:?}"
        ))),
    }
}

fn parse_positive_frontier_attempt(value: &str) -> Result<u16, SkillEvalError> {
    let attempt = parse_number(value, "frontier attempt")?;
    if attempt == 0 {
        return Err(invalid("frontier attempt must be positive"));
    }
    Ok(attempt)
}

fn execute_frontier_preview_command<R: FrontierPreviewRuntime + ?Sized>(
    command: &CliCommand,
    format: OutputFormat,
    runtime: &R,
    output: &mut dyn Write,
) -> Result<(), SkillEvalError> {
    let CliCommand::FrontierPreview { plan_path } = command else {
        return Err(invalid("command is not frontier preview"));
    };
    render_frontier_preview(&preview_frontier(plan_path, runtime)?, format, output)
}

fn execute_frontier_apply_command<R: FrontierApplyRuntime + ?Sized>(
    command: &CliCommand,
    format: OutputFormat,
    runtime: &mut R,
    output: &mut dyn Write,
) -> Result<(), SkillEvalError> {
    let CliCommand::FrontierApply { run_id } = command else {
        return Err(invalid("command is not frontier apply"));
    };
    render_frontier_apply(&apply_frontier_baseline(run_id, runtime)?, format, output)
}

pub(crate) fn execute_frontier_command(
    command: &CliCommand,
    format: OutputFormat,
    runtime: &mut dyn FrontierRuntime,
    output: &mut dyn Write,
) -> Result<(), SkillEvalError> {
    match command {
        CliCommand::FrontierPreview { .. } => {
            execute_frontier_preview_command(command, format, runtime, output)
        }
        CliCommand::FrontierStart { plan_path } => {
            preflight_run_id_file()?;
            let mut progress = RenderFrontierProgress { format, output };
            let state = start_frontier(plan_path, runtime, &mut progress)?;
            write_run_id_value(&state.configuration.run_id.0)
        }
        CliCommand::FrontierResume { run_id } => {
            let mut progress = RenderFrontierProgress { format, output };
            resume_frontier(run_id, runtime, &mut progress).map(|_| ())
        }
        CliCommand::FrontierReport {
            run_id,
            baseline_path,
        } => render_frontier_report(
            &crate::service::build_frontier_report(run_id, baseline_path.as_deref(), runtime)?,
            format,
            output,
        ),
        CliCommand::FrontierInspect { selector } => {
            render_frontier_inspection(&inspect_frontier(selector, runtime)?, format, output)
        }
        CliCommand::FrontierDecide { request } => {
            let state = record_frontier_decision(request, runtime)?;
            RenderFrontierProgress { format, output }.emit_frontier(&state)
        }
        CliCommand::FrontierApply { .. } => {
            execute_frontier_apply_command(command, format, runtime, output)
        }
        _ => Err(invalid("command is not a cumulative frontier command")),
    }
}

fn render_frontier_preview(
    report: &FrontierPreviewReport,
    format: OutputFormat,
    output: &mut dyn Write,
) -> Result<(), SkillEvalError> {
    validate_frontier_preview(report)?;
    if format == OutputFormat::JsonLines {
        return write_json_line(report, output);
    }
    writeln!(output, "suite capacity:").map_err(output_error)?;
    for tier in frontier_tiers() {
        writeln!(
            output,
            "  {}: {} cases",
            tier_label(tier),
            report.tier_case_counts[&tier]
        )
        .map_err(output_error)?;
    }
    writeln!(
        output,
        "guards: capacity=passed; owner_approval_required={}",
        report.is_owner_approval_required
    )
    .map_err(output_error)?;
    writeln!(output, "plan sha256: {}", report.plan_sha256).map_err(output_error)?;
    writeln!(output, "routes: {}", report.route_count).map_err(output_error)?;
    writeln!(
        output,
        "candidate calls: minimum {}, maximum {}",
        report.candidate_calls.minimum, report.candidate_calls.maximum
    )
    .map_err(output_error)?;
    writeln!(
        output,
        "judge calls: minimum {}, maximum {}",
        report.judge_calls.minimum, report.judge_calls.maximum
    )
    .map_err(output_error)?;
    writeln!(
        output,
        "maximum spending: {} millionths of a dollar",
        report.maximum_spending_millionths_of_dollar
    )
    .map_err(output_error)
}

fn render_frontier_report(
    report: &FrontierReport,
    format: OutputFormat,
    output: &mut dyn Write,
) -> Result<(), SkillEvalError> {
    validate_frontier_report(report)?;
    if format == OutputFormat::JsonLines {
        return write_json_line(report, output);
    }
    render_frontier_matrix(report, output)?;
    writeln!(
        output,
        "frontier {}: {:?}; spent {} millionths of a dollar",
        report.run_id.0, report.status, report.spent_millionths_of_dollar
    )
    .map_err(output_error)?;
    if let Some(pause) = &report.pause {
        writeln!(output, "infrastructure/pause: {pause:?}").map_err(output_error)?;
    }
    if let Some(decision) = &report.decision {
        writeln!(
            output,
            "decision: {:?}; reason {}; at {}",
            decision.decision, decision.reason, decision.decided_at.0
        )
        .map_err(output_error)?;
    }
    for model in &report.models {
        writeln!(
            output,
            "model {}/{}: highest {}; baseline {:?}",
            model.provider,
            model.model,
            model.highest_passing_tier.map_or("none", tier_label),
            model.baseline_change
        )
        .map_err(output_error)?;
        writeln!(
            output,
            "  selected routes: {}",
            frontier_route_list(&model.selected_routes)
        )
        .map_err(output_error)?;
        let memberships = model
            .pool_memberships
            .iter()
            .map(|(tier, membership)| {
                format!(
                    "{} rank {} active={}",
                    tier_label(*tier),
                    membership.rank,
                    membership.is_active
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        writeln!(
            output,
            "  pool memberships: {}",
            if memberships.is_empty() {
                "none"
            } else {
                &memberships
            }
        )
        .map_err(output_error)?;
        render_frontier_usage("  total usage", &model.total_usage, output)?;
        for cell in &model.cells {
            write!(
                output,
                "  cell {} {}: {:?}; trials {}/{}; failures {}",
                tier_label(cell.model.tier),
                cell.model.thinking,
                cell.status,
                cell.completed_trials,
                cell.expected_trials,
                cell.failed_trials
            )
            .map_err(output_error)?;
            if let Some(score) = &cell.score {
                write!(
                    output,
                    "; weighted {}; lower {}; critical {}/{}; coverage={}",
                    score.weighted_pass_basis_points,
                    score.lower_bound_basis_points,
                    score.critical_passed_trials,
                    score.critical_expected_trials,
                    score.is_group_coverage_complete
                )
                .map_err(output_error)?;
            }
            writeln!(output).map_err(output_error)?;
            render_frontier_usage("    usage", &cell.total_usage, output)?;
        }
    }
    Ok(())
}

fn render_frontier_inspection(
    inspection: &FrontierInspection,
    format: OutputFormat,
    output: &mut dyn Write,
) -> Result<(), SkillEvalError> {
    validate_frontier_inspection(inspection)?;
    if format == OutputFormat::JsonLines {
        return write_json_line(inspection, output);
    }
    let value = serde_json::to_string_pretty(inspection)
        .map_err(|error| malformed_frontier_render(format!("inspection serialization: {error}")))?;
    writeln!(output, "frontier inspection:").map_err(output_error)?;
    writeln!(output, "{value}").map_err(output_error)
}

fn render_frontier_apply(
    report: &FrontierApplyReport,
    format: OutputFormat,
    output: &mut dyn Write,
) -> Result<(), SkillEvalError> {
    validate_frontier_apply(report)?;
    if format == OutputFormat::JsonLines {
        return write_json_line(report, output);
    }
    writeln!(output, "frontier apply {}:", report.run_id.0).map_err(output_error)?;
    for tier in frontier_tiers() {
        writeln!(
            output,
            "  {}: {}",
            tier_label(tier),
            frontier_route_list(&report.active_routes[&tier])
        )
        .map_err(output_error)?;
    }
    writeln!(
        output,
        "status: {}",
        if report.is_changed {
            "changed"
        } else {
            "no-op"
        }
    )
    .map_err(output_error)
}

fn render_frontier_matrix(
    report: &FrontierReport,
    output: &mut dyn Write,
) -> Result<(), SkillEvalError> {
    writeln!(
        output,
        "| Model | off | minimal | low | medium | high | xhigh | max |"
    )
    .map_err(output_error)?;
    writeln!(output, "| --- | --- | --- | --- | --- | --- | --- | --- |").map_err(output_error)?;
    for model in &report.models {
        let mut cells = vec!["N/A".to_owned(); RESULT_MATRIX_LEVELS.len()];
        for thinking in &model.supported_thinking_levels {
            cells[frontier_thinking_level_index(thinking)?].clear();
        }
        for cell in &model.cells {
            let index = frontier_thinking_level_index(&cell.model.thinking)?;
            match cell.status {
                crate::model::FrontierCellStatus::Passed if cell.model.tier == Tier::T5 => {
                    cells[index] = "P5".to_owned();
                }
                crate::model::FrontierCellStatus::Passed => {}
                crate::model::FrontierCellStatus::Failed => {
                    cells[index] = format!("F{}", frontier_tier_number(cell.model.tier));
                }
                crate::model::FrontierCellStatus::Indeterminate => {
                    cells[index] = format!("I{}", frontier_tier_number(cell.model.tier));
                }
                crate::model::FrontierCellStatus::Pending
                | crate::model::FrontierCellStatus::Skipped => {}
                crate::model::FrontierCellStatus::Running => {
                    return Err(malformed_frontier_render("running cell is not renderable"));
                }
            }
        }
        writeln!(
            output,
            "| {}/{} | {} |",
            model.provider,
            model.model,
            cells.join(" | ")
        )
        .map_err(output_error)?;
    }
    Ok(())
}

fn validate_frontier_preview(report: &FrontierPreviewReport) -> Result<(), SkillEvalError> {
    if !is_sha256(&report.plan_sha256)
        || report.tier_case_counts.len() != frontier_tiers().len()
        || frontier_tiers().iter().any(|tier| {
            report
                .tier_case_counts
                .get(tier)
                .is_none_or(|count| *count < 30)
        })
        || report.route_count == 0
        || report.candidate_calls.minimum == 0
        || report.candidate_calls.minimum > report.candidate_calls.maximum
        || report.judge_calls != report.candidate_calls
        || report.maximum_spending_millionths_of_dollar == 0
    {
        return Err(malformed_frontier_render("preview guards are incomplete"));
    }
    Ok(())
}

fn validate_frontier_report(report: &FrontierReport) -> Result<(), SkillEvalError> {
    if report.run_id.0.trim().is_empty() || report.models.is_empty() {
        return Err(malformed_frontier_render("report identity is incomplete"));
    }
    match report.status {
        crate::model::FrontierRunStatus::Paused if report.pause.is_none() => {
            return Err(malformed_frontier_render("paused report has no pause"));
        }
        crate::model::FrontierRunStatus::Accepted => {
            validate_frontier_decision(report, Decision::Accepted)?;
        }
        crate::model::FrontierRunStatus::Rejected => {
            validate_frontier_decision(report, Decision::Rejected)?;
        }
        _ if report.decision.is_some() => {
            return Err(malformed_frontier_render("report decision is forged"));
        }
        _ => {}
    }
    if report.status != crate::model::FrontierRunStatus::Paused && report.pause.is_some() {
        return Err(malformed_frontier_render("report pause is forged"));
    }

    let mut models = BTreeSet::new();
    let mut pool_ranks = BTreeMap::<Tier, BTreeSet<u16>>::new();
    for model in &report.models {
        let supported = model
            .supported_thinking_levels
            .iter()
            .map(|thinking| {
                frontier_thinking_level_index(thinking).map(|index| (index, thinking.as_str()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if model.provider.trim().is_empty()
            || model.model.trim().is_empty()
            || !models.insert((model.provider.as_str(), model.model.as_str()))
            || supported.is_empty()
            || supported
                .windows(2)
                .any(|window| window[0].0 >= window[1].0)
        {
            return Err(malformed_frontier_render(
                "model identity is incomplete or duplicated",
            ));
        }
        let mut routes = BTreeSet::new();
        let mut selected_routes = Vec::new();
        let mut highest_passing_tier = None;
        let mut total_usage = zero_usage();
        for cell in &model.cells {
            frontier_thinking_level_index(&cell.model.thinking)?;
            if cell.model.provider != model.provider
                || cell.model.model != model.model
                || !model
                    .supported_thinking_levels
                    .contains(&cell.model.thinking)
                || !routes.insert((cell.model.tier, cell.model.thinking.as_str()))
                || cell.failed_trials > cell.completed_trials
                || cell.completed_trials > cell.expected_trials
            {
                return Err(malformed_frontier_render(
                    "frontier cell identity is invalid",
                ));
            }
            match cell.status {
                crate::model::FrontierCellStatus::Passed
                | crate::model::FrontierCellStatus::Failed
                | crate::model::FrontierCellStatus::Indeterminate => {
                    validate_frontier_score(cell)?;
                }
                crate::model::FrontierCellStatus::Pending
                | crate::model::FrontierCellStatus::Skipped => {
                    if cell.score.is_some()
                        || cell.completed_trials != 0
                        || cell.expected_trials != 0
                        || cell.failed_trials != 0
                        || cell.total_usage != zero_usage()
                    {
                        return Err(malformed_frontier_render("blank cell contains evidence"));
                    }
                }
                crate::model::FrontierCellStatus::Running => {
                    return Err(malformed_frontier_render("running cell is not renderable"));
                }
            }
            if cell.status == crate::model::FrontierCellStatus::Passed {
                selected_routes.push(cell.model.clone());
                highest_passing_tier = Some(
                    highest_passing_tier
                        .map_or(cell.model.tier, |tier: Tier| tier.max(cell.model.tier)),
                );
            }
            add_frontier_usage(&mut total_usage, &cell.total_usage)?;
        }
        if selected_routes != model.selected_routes
            || highest_passing_tier != model.highest_passing_tier
            || total_usage != model.total_usage
        {
            return Err(malformed_frontier_render(
                "model summary differs from its cells",
            ));
        }
        for (tier, membership) in &model.pool_memberships {
            if membership.model.provider != model.provider
                || membership.model.model != model.model
                || membership.model.tier != *tier
                || membership.rank == 0
                || !selected_routes.contains(&membership.model)
                || !pool_ranks.entry(*tier).or_default().insert(membership.rank)
            {
                return Err(malformed_frontier_render("pool membership is invalid"));
            }
        }
    }
    Ok(())
}

fn validate_frontier_decision(
    report: &FrontierReport,
    expected: Decision,
) -> Result<(), SkillEvalError> {
    if report.decision.as_ref().is_none_or(|decision| {
        decision.decision != expected
            || decision.reason.trim().is_empty()
            || decision.decided_at.0.trim().is_empty()
    }) {
        return Err(malformed_frontier_render(
            "terminal report decision is incomplete",
        ));
    }
    Ok(())
}

fn validate_frontier_score(
    cell: &crate::model::FrontierCellEvidence,
) -> Result<(), SkillEvalError> {
    if cell.completed_trials == 0
        || cell.completed_trials != cell.expected_trials
        || cell.score.as_ref().is_none_or(|score| {
            score.weighted_pass_basis_points > 10_000
                || score.lower_bound_basis_points > 10_000
                || score.critical_passed_trials > score.critical_expected_trials
                || score.critical_expected_trials > cell.completed_trials
        })
    {
        return Err(malformed_frontier_render(
            "terminal cell evidence is incomplete",
        ));
    }
    Ok(())
}

fn validate_frontier_inspection(inspection: &FrontierInspection) -> Result<(), SkillEvalError> {
    match inspection {
        FrontierInspection::Trial { trial } => {
            frontier_thinking_level_index(&trial.model.thinking)?;
            if trial.key.artifact.0.trim().is_empty()
                || trial.key.case.0.trim().is_empty()
                || trial.key.attempt == 0
                || trial.key.tier != trial.model.tier
                || trial.model.provider.trim().is_empty()
                || trial.model.model.trim().is_empty()
                || trial.harness.runner_version.trim().is_empty()
                || trial.harness.pi_version.trim().is_empty()
                || trial.harness.artifact_revision.trim().is_empty()
                || trial.harness.tool_policy_digest.trim().is_empty()
                || trial.artifact_path.as_os_str().is_empty()
                || trial.transcript_path.as_os_str().is_empty()
                || trial.judge_model.provider.trim().is_empty()
                || trial.judge_model.model.trim().is_empty()
                || trial.judge_model.thinking.trim().is_empty()
                || trial.verdict.score > 10
                || trial
                    .verdict
                    .checks
                    .iter()
                    .any(|check| check.name.trim().is_empty())
            {
                return Err(malformed_frontier_render("trial inspection is incomplete"));
            }
        }
        FrontierInspection::Infrastructure { event } => {
            frontier_thinking_level_index(&event.model.thinking)?;
            if event.model.provider.trim().is_empty()
                || event.model.model.trim().is_empty()
                || event.artifact.0.trim().is_empty()
                || event.case.0.trim().is_empty()
                || event.attempt == 0
                || event.infrastructure_attempt == 0
                || event.message.trim().is_empty()
                || event.occurred_at.0.trim().is_empty()
            {
                return Err(malformed_frontier_render(
                    "infrastructure inspection is incomplete",
                ));
            }
        }
    }
    Ok(())
}

fn validate_frontier_apply(report: &FrontierApplyReport) -> Result<(), SkillEvalError> {
    if report.run_id.0.trim().is_empty() || report.active_routes.len() != frontier_tiers().len() {
        return Err(malformed_frontier_render("apply identity is incomplete"));
    }
    let mut routes = BTreeSet::new();
    for tier in frontier_tiers() {
        let tier_routes = report
            .active_routes
            .get(&tier)
            .ok_or_else(|| malformed_frontier_render("apply tier is missing"))?;
        if tier_routes.is_empty() {
            return Err(malformed_frontier_render("apply tier has no active route"));
        }
        for route in tier_routes {
            frontier_thinking_level_index(&route.thinking)?;
            if route.tier != tier
                || route.provider.trim().is_empty()
                || route.model.trim().is_empty()
                || !routes.insert((
                    route.provider.as_str(),
                    route.model.as_str(),
                    route.tier,
                    route.thinking.as_str(),
                ))
            {
                return Err(malformed_frontier_render("active route is invalid"));
            }
        }
    }
    Ok(())
}

fn render_frontier_usage(
    label: &str,
    usage: &TrialUsage,
    output: &mut dyn Write,
) -> Result<(), SkillEvalError> {
    writeln!(
        output,
        "{label}: input {}; output {}; cache read {}; cache write {}; turns {}; tools {}; latency {} ms; cost {} millionths",
        usage.input_tokens,
        usage.output_tokens,
        usage.cache_read_tokens,
        usage.cache_write_tokens,
        usage.turns,
        usage.tool_calls,
        usage.elapsed_milliseconds,
        usage.cost_millionths_of_dollar
    )
    .map_err(output_error)
}

fn frontier_route_list(routes: &[ModelIdentity]) -> String {
    if routes.is_empty() {
        return "none".to_owned();
    }
    routes
        .iter()
        .map(model_label)
        .collect::<Vec<_>>()
        .join(", ")
}

fn frontier_tiers() -> [Tier; 5] {
    [Tier::T1, Tier::T2, Tier::T3, Tier::T4, Tier::T5]
}

fn frontier_tier_number(tier: Tier) -> u8 {
    match tier {
        Tier::T1 => 1,
        Tier::T2 => 2,
        Tier::T3 => 3,
        Tier::T4 => 4,
        Tier::T5 => 5,
    }
}

fn frontier_thinking_level_index(level: &str) -> Result<usize, SkillEvalError> {
    RESULT_MATRIX_LEVELS
        .iter()
        .position(|candidate| *candidate == level)
        .ok_or_else(|| malformed_frontier_render("unknown frontier thinking level"))
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
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

fn add_frontier_usage(total: &mut TrialUsage, usage: &TrialUsage) -> Result<(), SkillEvalError> {
    total.input_tokens = total
        .input_tokens
        .checked_add(usage.input_tokens)
        .ok_or_else(|| malformed_frontier_render("input token total overflowed"))?;
    total.output_tokens = total
        .output_tokens
        .checked_add(usage.output_tokens)
        .ok_or_else(|| malformed_frontier_render("output token total overflowed"))?;
    total.cache_read_tokens = total
        .cache_read_tokens
        .checked_add(usage.cache_read_tokens)
        .ok_or_else(|| malformed_frontier_render("cache read total overflowed"))?;
    total.cache_write_tokens = total
        .cache_write_tokens
        .checked_add(usage.cache_write_tokens)
        .ok_or_else(|| malformed_frontier_render("cache write total overflowed"))?;
    total.turns = total
        .turns
        .checked_add(usage.turns)
        .ok_or_else(|| malformed_frontier_render("turn total overflowed"))?;
    total.tool_calls = total
        .tool_calls
        .checked_add(usage.tool_calls)
        .ok_or_else(|| malformed_frontier_render("tool call total overflowed"))?;
    total.elapsed_milliseconds = total
        .elapsed_milliseconds
        .checked_add(usage.elapsed_milliseconds)
        .ok_or_else(|| malformed_frontier_render("latency total overflowed"))?;
    total.cost_millionths_of_dollar = total
        .cost_millionths_of_dollar
        .checked_add(usage.cost_millionths_of_dollar)
        .ok_or_else(|| malformed_frontier_render("cost total overflowed"))?;
    Ok(())
}

fn malformed_frontier_render(message: impl Into<String>) -> SkillEvalError {
    SkillEvalError::InvalidConfiguration(format!(
        "malformed frontier render state: {}",
        message.into()
    ))
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

fn parse_positive_number(value: &str, label: &str) -> Result<u64, SkillEvalError> {
    let number = parse_number(value, label)?;
    if number == 0 {
        return Err(invalid(format!("{label} must be positive")));
    }
    Ok(number)
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
include!("../tests/pool_report.rs");
#[cfg(test)]
include!("../tests/frontier_suite_cli.rs");
#[cfg(test)]
include!("../tests/frontier_runtime.rs");
#[cfg(test)]
include!("../tests/frontier_cli.rs");
#[cfg(test)]
include!("../tests/frontier_render.rs");
#[cfg(test)]
include!("../tests/frontier_apply.rs");

#[cfg(test)]
cli_tests!();
#[cfg(test)]
pool_report_tests!();
#[cfg(test)]
frontier_suite_cli_tests!();
#[cfg(test)]
frontier_runtime_tests!();
#[cfg(test)]
frontier_cli_tests!();
#[cfg(test)]
frontier_render_tests!();
#[cfg(test)]
frontier_apply_tests!();
