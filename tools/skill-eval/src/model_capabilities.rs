use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::model::{
    ModelIdentity, SkillEvalError, T1ScreenCallRange, T1ScreenEligibleRow, T1ScreenExcludedRow,
    T1ScreenExclusionReason, T1ScreenPreviewReport, T1ScreenSnapshotIdentity,
};
use crate::models::{derive_supported_thinking_levels, normalize_rpc_model_id};

const SNAPSHOT_VERSION: u64 = 1;
const T1_EXAM_CASE_COUNT: u64 = 5;
const THINKING_LEVELS: [&str; 7] = ["off", "minimal", "low", "medium", "high", "xhigh", "max"];
static TEMPORARY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ModelCapabilitySnapshot {
    snapshot_version: u64,
    observed_at_unix_seconds: u64,
    pi_version: String,
    probe_commands: Vec<ProbeCommand>,
    counts: SnapshotCounts,
    models: Vec<ModelCapabilityRow>,
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct ProbeCommand {
    purpose: String,
    program: String,
    arguments: Vec<String>,
    extension_mode: Option<String>,
    request_id: Option<String>,
    request_type: Option<String>,
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct SnapshotCounts {
    union: usize,
    all_extension_list: usize,
    core_rpc: usize,
    list_only: usize,
    rpc_only: usize,
    moving_aliases: usize,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ModelCapabilityRow {
    provider: String,
    model: String,
    display_name: Option<String>,
    #[serde(rename = "reasoning")]
    is_reasoning_supported: Option<bool>,
    supported_pi_thinking_levels: Option<Vec<String>>,
    context_window: Option<u64>,
    maximum_output: Option<u64>,
    input_modes: Option<Vec<String>>,
    pricing_per_million_tokens: Option<ModelPricing>,
    is_in_all_extension_list: bool,
    is_in_core_rpc: bool,
    is_moving_alias: bool,
    is_exact_qualification_evidence: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ModelPricing {
    input: f64,
    output: f64,
    cache_read: f64,
    cache_write: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tiers: Vec<ModelPricingTier>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ModelPricingTier {
    input_tokens_above: u64,
    input: f64,
    output: f64,
    cache_read: f64,
    cache_write: f64,
}

#[derive(Debug, Deserialize)]
struct RpcCatalog {
    models: Vec<RpcModel>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RpcModel {
    provider: String,
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(rename = "reasoning")]
    is_reasoning_supported: bool,
    #[serde(default)]
    thinking_level_map: Option<BTreeMap<String, Option<String>>>,
    #[serde(default)]
    input: Option<Vec<String>>,
    #[serde(default)]
    cost: Option<ModelPricing>,
    #[serde(default)]
    context_window: Option<u64>,
    #[serde(default)]
    max_tokens: Option<u64>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ModelKey {
    provider: String,
    model: String,
}

#[derive(Clone, Copy, Debug)]
struct ListPresence {
    is_moving_alias: bool,
}

#[derive(Debug)]
struct RpcPresence {
    model: RpcModel,
    is_moving_alias: bool,
}

pub(crate) fn preflight_output(
    repository_root: &Path,
    output_path: &Path,
) -> Result<(), SkillEvalError> {
    prepare_destination(repository_root, output_path).map(|_| ())
}

pub(crate) fn capture(
    repository_root: &Path,
    output_path: &Path,
    list_output: &str,
    rpc_output: &str,
    pi_version_output: &str,
    observed_at_unix_seconds: u64,
) -> Result<(), SkillEvalError> {
    let destination = prepare_destination(repository_root, output_path)?;
    let snapshot = build_snapshot(
        list_output,
        rpc_output,
        pi_version_output,
        observed_at_unix_seconds,
    )?;
    let mut bytes = serde_json::to_vec_pretty(&snapshot).map_err(|error| {
        invalid(format!(
            "model capability snapshot cannot be serialized: {error}"
        ))
    })?;
    bytes.push(b'\n');
    write_new_atomic(&destination, &bytes)
}

pub(crate) fn observed_at_unix_seconds() -> Result<u64, SkillEvalError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| invalid(format!("system clock is invalid: {error}")))
        .map(|duration| duration.as_secs())
}

pub(crate) fn t1_screen_preview(
    repository_root: &Path,
    capabilities_path: &Path,
) -> Result<T1ScreenPreviewReport, SkillEvalError> {
    let bytes = read_capability_snapshot(repository_root, capabilities_path)?;
    let snapshot = parse_capability_snapshot(&bytes)?;
    validate_capability_snapshot(&snapshot)?;

    let mut eligible = Vec::new();
    let mut excluded = Vec::new();
    let mut supported_level_count = 0_u64;
    for row in snapshot.models {
        let is_preview = identity_has_preview_token(&row.provider, &row.model);
        let reasons = exclusion_reasons(&row);
        if reasons.is_empty() {
            let levels = row
                .supported_pi_thinking_levels
                .ok_or_else(|| invalid("eligible T1 row has no thinking levels"))?;
            supported_level_count = supported_level_count
                .checked_add(u64::try_from(levels.len()).map_err(|_| {
                    invalid("T1 supported thinking level count exceeds the supported range")
                })?)
                .ok_or_else(|| invalid("T1 supported thinking level count overflowed"))?;
            eligible.push(T1ScreenEligibleRow {
                provider: row.provider,
                model: row.model,
                supported_pi_thinking_levels: levels,
                is_preview,
            });
        } else {
            excluded.push(T1ScreenExcludedRow {
                provider: row.provider,
                model: row.model,
                is_preview,
                reasons,
            });
        }
    }

    let classified_count = eligible
        .len()
        .checked_add(excluded.len())
        .ok_or_else(|| invalid("T1 total row count overflowed"))?;
    let total_rows = checked_count(classified_count, "T1 total row count")?;
    let eligible_count = checked_count(eligible.len(), "T1 eligible row count")?;
    let excluded_count = checked_count(excluded.len(), "T1 excluded row count")?;
    if total_rows != checked_count(snapshot.counts.union, "snapshot union count")?
        || eligible_count
            .checked_add(excluded_count)
            .is_none_or(|count| count != total_rows)
    {
        return Err(invalid("T1 screening classification count mismatch"));
    }
    let exact_calls = supported_level_count
        .checked_mul(T1_EXAM_CASE_COUNT)
        .ok_or_else(|| invalid("T1 complete call count overflowed"))?;
    let call_range = T1ScreenCallRange {
        minimum: exact_calls,
        maximum: exact_calls,
    };

    Ok(T1ScreenPreviewReport {
        snapshot: T1ScreenSnapshotIdentity {
            path: capabilities_path.to_path_buf(),
            sha256: sha256_hex(&bytes),
            version: snapshot.snapshot_version,
            observed_at_unix_seconds: snapshot.observed_at_unix_seconds,
            pi_version: snapshot.pi_version,
        },
        total_rows,
        eligible_count,
        excluded_count,
        eligible,
        excluded,
        exam_case_count: T1_EXAM_CASE_COUNT,
        candidate_calls: call_range.clone(),
        judge_calls: call_range,
        projected_candidate_money_cost_usd: 0,
        is_judge_money_projected_from_candidate_price: false,
        is_owner_approved_judge_cap_required_before_execution: true,
        judge_money_note: "judge money is not projected from candidate price and requires an owner-approved cap before execution".to_owned(),
    })
}

/// Computes a conservative per-call judge cost bound from a frozen capability snapshot.
///
/// The inputs are exact snapshot bytes and a judge identity. The output is millionths of a dollar
/// at the model's full context and output limits. It returns an error for malformed snapshots,
/// missing exact identities, incomplete pricing or token limits, and numeric overflow.
pub(crate) fn t1_judge_cost_upper_bound(
    bytes: &[u8],
    judge: &ModelIdentity,
) -> Result<u64, SkillEvalError> {
    let snapshot = parse_capability_snapshot(bytes)?;
    validate_capability_snapshot(&snapshot)?;
    let row = snapshot
        .models
        .iter()
        .find(|row| row.provider == judge.provider && row.model == judge.model)
        .ok_or_else(|| invalid("T1 judge is absent from the frozen capability snapshot"))?;
    let context_window = row
        .context_window
        .ok_or_else(|| invalid("T1 judge context-window limit is absent"))?;
    let maximum_output = row
        .maximum_output
        .ok_or_else(|| invalid("T1 judge output limit is absent"))?;
    let pricing = row
        .pricing_per_million_tokens
        .as_ref()
        .ok_or_else(|| invalid("T1 judge pricing is absent"))?;
    let input_price = pricing
        .input
        .max(pricing.cache_read)
        .max(pricing.cache_write);
    let bound = input_price.mul_add(
        context_window as f64,
        pricing.output * maximum_output as f64,
    );
    if !bound.is_finite() || bound.is_sign_negative() || bound.ceil() > u64::MAX as f64 {
        return Err(invalid("T1 judge cost upper bound overflowed"));
    }
    Ok(bound.ceil() as u64)
}

fn read_capability_snapshot(
    repository_root: &Path,
    capabilities_path: &Path,
) -> Result<Vec<u8>, SkillEvalError> {
    if capabilities_path.as_os_str().is_empty()
        || capabilities_path.is_absolute()
        || capabilities_path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        || capabilities_path
            .to_string_lossy()
            .chars()
            .any(char::is_control)
    {
        return Err(invalid(
            "T1 capability snapshot must be a safe repository-relative path",
        ));
    }
    let repository_root =
        fs::canonicalize(repository_root).map_err(|error| SkillEvalError::Io {
            path: repository_root.to_path_buf(),
            message: error.to_string(),
        })?;
    let mut current = repository_root.clone();
    for (index, component) in capabilities_path.components().enumerate() {
        current.push(component.as_os_str());
        let metadata = fs::symlink_metadata(&current).map_err(|error| SkillEvalError::Io {
            path: current.clone(),
            message: error.to_string(),
        })?;
        if metadata.file_type().is_symlink() {
            return Err(invalid(format!(
                "T1 capability snapshot path contains symlink {}",
                current.display()
            )));
        }
        let is_last = index + 1 == capabilities_path.components().count();
        if (!is_last && !metadata.is_dir()) || (is_last && !metadata.is_file()) {
            return Err(invalid(format!(
                "T1 capability snapshot path component {} has the wrong type",
                current.display()
            )));
        }
    }
    let canonical_path = fs::canonicalize(&current).map_err(|error| SkillEvalError::Io {
        path: current.clone(),
        message: error.to_string(),
    })?;
    if !canonical_path.starts_with(&repository_root) {
        return Err(invalid(
            "T1 capability snapshot escapes the repository root",
        ));
    }
    fs::read(&canonical_path).map_err(|error| SkillEvalError::Io {
        path: canonical_path,
        message: error.to_string(),
    })
}

fn parse_capability_snapshot(bytes: &[u8]) -> Result<ModelCapabilitySnapshot, SkillEvalError> {
    let snapshot = serde_json::from_slice::<ModelCapabilitySnapshot>(bytes)
        .map_err(|error| invalid(format!("T1 capability snapshot is malformed: {error}")))?;
    let value = serde_json::from_slice::<serde_json::Value>(bytes)
        .map_err(|error| invalid(format!("T1 capability snapshot is malformed: {error}")))?;
    validate_snapshot_field_presence(&value)?;
    Ok(snapshot)
}

fn validate_snapshot_field_presence(value: &serde_json::Value) -> Result<(), SkillEvalError> {
    validate_object_fields(
        value,
        &[
            "snapshot_version",
            "observed_at_unix_seconds",
            "pi_version",
            "probe_commands",
            "counts",
            "models",
        ],
        "snapshot",
    )?;
    validate_object_fields(
        &value["counts"],
        &[
            "union",
            "all_extension_list",
            "core_rpc",
            "list_only",
            "rpc_only",
            "moving_aliases",
        ],
        "snapshot counts",
    )?;
    let probes = value["probe_commands"]
        .as_array()
        .ok_or_else(|| invalid("T1 capability snapshot probe_commands is malformed"))?;
    for probe in probes {
        validate_object_fields(
            probe,
            &[
                "purpose",
                "program",
                "arguments",
                "extension_mode",
                "request_id",
                "request_type",
            ],
            "snapshot probe command",
        )?;
    }
    let models = value["models"]
        .as_array()
        .ok_or_else(|| invalid("T1 capability snapshot models is malformed"))?;
    for row in models {
        validate_object_fields(
            row,
            &[
                "provider",
                "model",
                "display_name",
                "reasoning",
                "supported_pi_thinking_levels",
                "context_window",
                "maximum_output",
                "input_modes",
                "pricing_per_million_tokens",
                "is_in_all_extension_list",
                "is_in_core_rpc",
                "is_moving_alias",
                "is_exact_qualification_evidence",
            ],
            "snapshot model row",
        )?;
        if !row["pricing_per_million_tokens"].is_null() {
            validate_snapshot_pricing_fields(&row["pricing_per_million_tokens"])?;
        }
    }
    Ok(())
}

fn validate_snapshot_pricing_fields(value: &serde_json::Value) -> Result<(), SkillEvalError> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid("T1 capability snapshot model pricing is malformed"))?;
    let mut expected = vec!["input", "output", "cacheRead", "cacheWrite"];
    if object.contains_key("tiers") {
        expected.push("tiers");
    }
    if object.len() != expected.len() || expected.iter().any(|field| !object.contains_key(*field)) {
        return Err(invalid(
            "T1 capability snapshot model pricing has missing or unknown fields",
        ));
    }
    let Some(tiers) = object.get("tiers") else {
        return Ok(());
    };
    let tiers = tiers
        .as_array()
        .ok_or_else(|| invalid("T1 capability snapshot model pricing tiers are malformed"))?;
    for tier in tiers {
        validate_object_fields(
            tier,
            &[
                "inputTokensAbove",
                "input",
                "output",
                "cacheRead",
                "cacheWrite",
            ],
            "snapshot model pricing tier",
        )?;
    }
    Ok(())
}

fn validate_object_fields(
    value: &serde_json::Value,
    expected: &[&str],
    label: &str,
) -> Result<(), SkillEvalError> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid(format!("T1 capability {label} is malformed")))?;
    if object.len() != expected.len() || expected.iter().any(|field| !object.contains_key(*field)) {
        return Err(invalid(format!(
            "T1 capability {label} has missing or unknown fields"
        )));
    }
    Ok(())
}

fn validate_capability_snapshot(snapshot: &ModelCapabilitySnapshot) -> Result<(), SkillEvalError> {
    if snapshot.snapshot_version != SNAPSHOT_VERSION {
        return Err(invalid(format!(
            "unsupported T1 capability snapshot version {}",
            snapshot.snapshot_version
        )));
    }
    if snapshot.observed_at_unix_seconds == 0
        || snapshot.pi_version.trim().is_empty()
        || snapshot.pi_version.lines().count() != 1
        || snapshot.pi_version.chars().any(char::is_control)
    {
        return Err(invalid("T1 capability snapshot metadata is malformed"));
    }
    validate_probe_commands(&snapshot.probe_commands)?;
    validate_snapshot_rows(&snapshot.models)?;
    validate_snapshot_counts(snapshot)
}

fn validate_probe_commands(probes: &[ProbeCommand]) -> Result<(), SkillEvalError> {
    let expected = [
        (
            "all_extension_availability",
            vec!["--list-models"],
            Some("normal_deployed_discovery"),
            None,
            None,
        ),
        (
            "core_model_metadata",
            vec![
                "--mode",
                "rpc",
                "--no-session",
                "--no-context-files",
                "--no-extensions",
            ],
            Some("disabled"),
            Some("skill-eval-models"),
            Some("get_available_models"),
        ),
        ("pi_version", vec!["--version"], None, None, None),
    ];
    if probes.len() != expected.len() {
        return Err(invalid(
            "T1 capability snapshot probe commands are malformed",
        ));
    }
    for (probe, (purpose, arguments, extension_mode, request_id, request_type)) in
        probes.iter().zip(expected)
    {
        if probe.purpose != purpose
            || probe.program != "pi"
            || probe.arguments.iter().map(String::as_str).ne(arguments)
            || probe.extension_mode.as_deref() != extension_mode
            || probe.request_id.as_deref() != request_id
            || probe.request_type.as_deref() != request_type
        {
            return Err(invalid(
                "T1 capability snapshot probe commands are malformed",
            ));
        }
    }
    Ok(())
}

fn validate_snapshot_rows(rows: &[ModelCapabilityRow]) -> Result<(), SkillEvalError> {
    let mut previous = None::<ModelKey>;
    for row in rows {
        let key = model_key(&row.provider, &row.model, "T1 capability snapshot")?;
        if previous.as_ref().is_some_and(|prior| prior >= &key) {
            return Err(invalid(
                "T1 capability snapshot identities are duplicate or unsorted",
            ));
        }
        previous = Some(key);
        if row.display_name.as_ref().is_some_and(|name| {
            name.trim().is_empty()
                || name.lines().count() != 1
                || name.chars().any(char::is_control)
        }) || row.context_window == Some(0)
            || row.maximum_output == Some(0)
        {
            return Err(invalid(format!(
                "T1 capability snapshot row {}/{} is malformed",
                row.provider, row.model
            )));
        }
        validate_input_modes(row)?;
        validate_snapshot_price(row)?;
    }
    Ok(())
}

fn validate_input_modes(row: &ModelCapabilityRow) -> Result<(), SkillEvalError> {
    let Some(modes) = &row.input_modes else {
        return Ok(());
    };
    let canonical = ["text", "image"]
        .into_iter()
        .filter(|mode| modes.iter().any(|candidate| candidate == mode))
        .collect::<Vec<_>>();
    if modes.is_empty()
        || modes.len() != canonical.len()
        || modes.iter().map(String::as_str).ne(canonical)
    {
        return Err(invalid(format!(
            "T1 capability snapshot row {}/{} has malformed input modes",
            row.provider, row.model
        )));
    }
    Ok(())
}

fn validate_snapshot_price(row: &ModelCapabilityRow) -> Result<(), SkillEvalError> {
    let Some(pricing) = &row.pricing_per_million_tokens else {
        return Ok(());
    };
    if !is_valid_pricing(pricing) {
        return Err(invalid(format!(
            "T1 capability snapshot row {}/{} has malformed pricing",
            row.provider, row.model
        )));
    }
    Ok(())
}

fn is_valid_pricing(pricing: &ModelPricing) -> bool {
    if [
        pricing.input,
        pricing.output,
        pricing.cache_read,
        pricing.cache_write,
    ]
    .into_iter()
    .any(|price| !price.is_finite() || price.is_sign_negative())
    {
        return false;
    }
    let mut previous_threshold = 0;
    pricing.tiers.iter().all(|tier| {
        let is_threshold_valid = tier.input_tokens_above > previous_threshold;
        previous_threshold = tier.input_tokens_above;
        is_threshold_valid
            && [tier.input, tier.output, tier.cache_read, tier.cache_write]
                .into_iter()
                .all(|price| price.is_finite() && !price.is_sign_negative())
    })
}

fn validate_snapshot_counts(snapshot: &ModelCapabilitySnapshot) -> Result<(), SkillEvalError> {
    let rows = &snapshot.models;
    let expected = SnapshotCounts {
        union: rows.len(),
        all_extension_list: rows
            .iter()
            .filter(|row| row.is_in_all_extension_list)
            .count(),
        core_rpc: rows.iter().filter(|row| row.is_in_core_rpc).count(),
        list_only: rows
            .iter()
            .filter(|row| row.is_in_all_extension_list && !row.is_in_core_rpc)
            .count(),
        rpc_only: rows
            .iter()
            .filter(|row| !row.is_in_all_extension_list && row.is_in_core_rpc)
            .count(),
        moving_aliases: rows.iter().filter(|row| row.is_moving_alias).count(),
    };
    if snapshot.counts != expected {
        return Err(invalid("T1 capability snapshot count mismatch"));
    }
    Ok(())
}

fn exclusion_reasons(row: &ModelCapabilityRow) -> Vec<T1ScreenExclusionReason> {
    let mut reasons = Vec::new();
    if !row.is_in_all_extension_list {
        reasons.push(T1ScreenExclusionReason::MissingList);
    }
    if !row.is_in_core_rpc {
        reasons.push(T1ScreenExclusionReason::MissingRpc);
    }
    if row.is_moving_alias {
        reasons.push(T1ScreenExclusionReason::MovingAlias);
    }
    if !row.is_exact_qualification_evidence {
        reasons.push(T1ScreenExclusionReason::NotExactEvidence);
    }
    if is_moving_router_or_control(&row.provider, &row.model) {
        reasons.push(T1ScreenExclusionReason::MovingRouterOrControl);
    }
    match &row.pricing_per_million_tokens {
        None => reasons.push(T1ScreenExclusionReason::MissingPrice),
        Some(pricing) => {
            if pricing.input != 0.0 || pricing.tiers.iter().any(|tier| tier.input != 0.0) {
                reasons.push(T1ScreenExclusionReason::NonzeroInputPrice);
            }
            if pricing.output != 0.0 || pricing.tiers.iter().any(|tier| tier.output != 0.0) {
                reasons.push(T1ScreenExclusionReason::NonzeroOutputPrice);
            }
        }
    }
    if row
        .input_modes
        .as_ref()
        .is_none_or(|modes| !modes.iter().any(|mode| mode == "text"))
    {
        reasons.push(T1ScreenExclusionReason::MissingTextInput);
    }
    match &row.supported_pi_thinking_levels {
        None => reasons.push(T1ScreenExclusionReason::MissingThinkingLevels),
        Some(levels) if levels.is_empty() => {
            reasons.push(T1ScreenExclusionReason::MissingThinkingLevels)
        }
        Some(levels) if !is_valid_thinking_levels(levels) => {
            reasons.push(T1ScreenExclusionReason::MalformedThinkingLevels)
        }
        Some(_) => {}
    }
    reasons
}

fn is_valid_thinking_levels(levels: &[String]) -> bool {
    let mut previous = None;
    for level in levels {
        let Some(index) = THINKING_LEVELS.iter().position(|known| level == known) else {
            return false;
        };
        if previous.is_some_and(|prior| prior >= index) {
            return false;
        }
        previous = Some(index);
    }
    true
}

fn is_moving_router_or_control(provider: &str, model: &str) -> bool {
    is_openrouter_auto_variant(provider, model)
        || provider == "openrouter" && matches!(model, "openrouter/free" | "openrouter/fusion")
}

fn is_openrouter_auto_variant(provider: &str, model: &str) -> bool {
    provider == "openrouter"
        && (matches!(model, "auto" | "openrouter/auto")
            || model.starts_with("auto-")
            || model.starts_with("openrouter/auto-"))
}

fn is_nested_openrouter_auto_variant(key: &ModelKey) -> bool {
    key.provider == "openrouter"
        && (key.model == "openrouter/auto" || key.model.starts_with("openrouter/auto-"))
}

fn identity_has_preview_token(provider: &str, model: &str) -> bool {
    format!("{provider}/{model}")
        .split(|character: char| !character.is_ascii_alphanumeric())
        .any(|token| token == "preview")
}

fn checked_count(count: usize, label: &str) -> Result<u64, SkillEvalError> {
    u64::try_from(count).map_err(|_| invalid(format!("{label} exceeds the supported range")))
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn build_snapshot(
    list_output: &str,
    rpc_output: &str,
    pi_version_output: &str,
    observed_at_unix_seconds: u64,
) -> Result<ModelCapabilitySnapshot, SkillEvalError> {
    let listed = parse_list_output(list_output)?;
    let rpc = parse_rpc_output(rpc_output)?;
    let pi_version = parse_pi_version(pi_version_output)?;
    let mut rows = BTreeMap::<ModelKey, ModelCapabilityRow>::new();

    for (key, presence) in &listed {
        rows.insert(
            key.clone(),
            ModelCapabilityRow {
                provider: key.provider.clone(),
                model: key.model.clone(),
                is_in_all_extension_list: true,
                is_moving_alias: presence.is_moving_alias,
                ..ModelCapabilityRow::default()
            },
        );
    }
    for (key, presence) in rpc {
        let model = presence.model;
        let row = rows
            .entry(key.clone())
            .or_insert_with(|| ModelCapabilityRow {
                provider: key.provider.clone(),
                model: key.model.clone(),
                ..ModelCapabilityRow::default()
            });
        row.display_name = model.name;
        row.is_reasoning_supported = Some(model.is_reasoning_supported);
        row.supported_pi_thinking_levels = Some(derive_supported_thinking_levels(
            &key.provider,
            &key.model,
            model.is_reasoning_supported,
            model.thinking_level_map.as_ref(),
        )?);
        row.context_window = model.context_window;
        row.maximum_output = model.max_tokens;
        row.input_modes = normalize_input_modes(model.input, &key)?;
        row.pricing_per_million_tokens = validate_pricing(model.cost, &key)?;
        row.is_in_core_rpc = true;
        row.is_moving_alias |= presence.is_moving_alias;
    }
    for row in rows.values_mut() {
        row.is_exact_qualification_evidence =
            row.is_in_all_extension_list && row.is_in_core_rpc && !row.is_moving_alias;
    }

    let models = rows.into_values().collect::<Vec<_>>();
    let counts = SnapshotCounts {
        union: models.len(),
        all_extension_list: models
            .iter()
            .filter(|model| model.is_in_all_extension_list)
            .count(),
        core_rpc: models.iter().filter(|model| model.is_in_core_rpc).count(),
        list_only: models
            .iter()
            .filter(|model| model.is_in_all_extension_list && !model.is_in_core_rpc)
            .count(),
        rpc_only: models
            .iter()
            .filter(|model| !model.is_in_all_extension_list && model.is_in_core_rpc)
            .count(),
        moving_aliases: models.iter().filter(|model| model.is_moving_alias).count(),
    };

    Ok(ModelCapabilitySnapshot {
        snapshot_version: SNAPSHOT_VERSION,
        observed_at_unix_seconds,
        pi_version,
        probe_commands: vec![
            ProbeCommand {
                purpose: "all_extension_availability".to_owned(),
                program: "pi".to_owned(),
                arguments: vec!["--list-models".to_owned()],
                extension_mode: Some("normal_deployed_discovery".to_owned()),
                request_id: None,
                request_type: None,
            },
            ProbeCommand {
                purpose: "core_model_metadata".to_owned(),
                program: "pi".to_owned(),
                arguments: [
                    "--mode",
                    "rpc",
                    "--no-session",
                    "--no-context-files",
                    "--no-extensions",
                ]
                .into_iter()
                .map(str::to_owned)
                .collect(),
                extension_mode: Some("disabled".to_owned()),
                request_id: Some("skill-eval-models".to_owned()),
                request_type: Some("get_available_models".to_owned()),
            },
            ProbeCommand {
                purpose: "pi_version".to_owned(),
                program: "pi".to_owned(),
                arguments: vec!["--version".to_owned()],
                extension_mode: None,
                request_id: None,
                request_type: None,
            },
        ],
        counts,
        models,
    })
}

fn parse_list_output(output: &str) -> Result<BTreeMap<ModelKey, ListPresence>, SkillEvalError> {
    let mut lines = output.lines().filter(|line| !line.trim().is_empty());
    let header = lines
        .next()
        .ok_or_else(|| invalid("Pi all-extension model list is empty"))?;
    let expected_header = [
        "provider", "model", "context", "max-out", "thinking", "images",
    ];
    if !header.split_whitespace().eq(expected_header) {
        return Err(invalid("Pi all-extension model list header is malformed"));
    }

    let mut listed = BTreeMap::new();
    for (index, line) in lines.enumerate() {
        let line_number = index + 2;
        let columns = line.split_whitespace().collect::<Vec<_>>();
        if columns.len() != expected_header.len()
            || parse_token_count(columns.get(2).copied().unwrap_or_default()).is_none()
            || parse_token_count(columns.get(3).copied().unwrap_or_default()).is_none()
            || !matches!(columns.get(4), Some(&"yes") | Some(&"no"))
            || !matches!(columns.get(5), Some(&"yes") | Some(&"no"))
        {
            return Err(invalid(format!(
                "Pi all-extension model list line {line_number} is malformed"
            )));
        }
        let provider = columns[0];
        let (model, is_moving_alias) = match columns[1].strip_prefix('~') {
            Some(model) => (model, true),
            None => (columns[1], false),
        };
        let key = model_key(provider, model, "Pi all-extension model list")?;
        if listed
            .insert(key.clone(), ListPresence { is_moving_alias })
            .is_some()
        {
            return Err(invalid(format!(
                "Pi all-extension model list duplicates or conflicts at {}/{}",
                key.provider, key.model
            )));
        }
    }
    Ok(listed)
}

fn parse_rpc_output(output: &str) -> Result<BTreeMap<ModelKey, RpcPresence>, SkillEvalError> {
    let catalog: RpcCatalog = serde_json::from_str(output)
        .map_err(|error| invalid(format!("Pi RPC model capability data is invalid: {error}")))?;
    let mut models = BTreeMap::new();
    for model in catalog.models {
        let (normalized_id, is_moving_alias) =
            normalize_rpc_model_id(&model.id).ok_or_else(|| {
                invalid(format!(
                    "Pi RPC model capability data has malformed model identity {}/{}",
                    model.provider, model.id
                ))
            })?;
        let key = model_key(
            &model.provider,
            normalized_id,
            "Pi RPC model capability data",
        )?;
        validate_rpc_model(&model, &key)?;
        if models
            .insert(
                key.clone(),
                RpcPresence {
                    model,
                    is_moving_alias,
                },
            )
            .is_some()
        {
            return Err(invalid(format!(
                "Pi RPC model capability data duplicates {}/{}",
                key.provider, key.model
            )));
        }
    }
    Ok(models)
}

fn validate_rpc_model(model: &RpcModel, key: &ModelKey) -> Result<(), SkillEvalError> {
    if model
        .name
        .as_ref()
        .is_some_and(|name| name.trim().is_empty() || name.chars().any(char::is_control))
        || model.context_window == Some(0)
        || model.max_tokens == Some(0)
    {
        return Err(invalid(format!(
            "Pi RPC model capability data for {}/{} is malformed",
            key.provider, key.model
        )));
    }
    derive_supported_thinking_levels(
        &key.provider,
        &key.model,
        model.is_reasoning_supported,
        model.thinking_level_map.as_ref(),
    )?;
    Ok(())
}

fn normalize_input_modes(
    input: Option<Vec<String>>,
    key: &ModelKey,
) -> Result<Option<Vec<String>>, SkillEvalError> {
    let Some(input) = input else {
        return Ok(None);
    };
    let modes = input.into_iter().collect::<BTreeSet<_>>();
    if modes.is_empty()
        || modes.len() > 2
        || !modes
            .iter()
            .all(|mode| matches!(mode.as_str(), "text" | "image"))
    {
        return Err(invalid(format!(
            "Pi RPC model capability data for {}/{} has malformed input modes",
            key.provider, key.model
        )));
    }
    Ok(Some(
        ["text", "image"]
            .into_iter()
            .filter(|mode| modes.contains(*mode))
            .map(str::to_owned)
            .collect(),
    ))
}

fn validate_pricing(
    pricing: Option<ModelPricing>,
    key: &ModelKey,
) -> Result<Option<ModelPricing>, SkillEvalError> {
    let Some(pricing) = pricing else {
        return Ok(None);
    };
    if is_nested_openrouter_auto_variant(key)
        && pricing.input.is_finite()
        && pricing.input < 0.0
        && pricing.output.is_finite()
        && pricing.output < 0.0
        && pricing.cache_read.is_finite()
        && pricing.cache_read >= 0.0
        && pricing.cache_write.is_finite()
        && pricing.cache_write >= 0.0
        && pricing.tiers.is_empty()
    {
        return Ok(None);
    }
    if !is_valid_pricing(&pricing) {
        return Err(invalid(format!(
            "Pi RPC model capability data for {}/{} has malformed pricing",
            key.provider, key.model
        )));
    }
    Ok(Some(pricing))
}

fn parse_pi_version(output: &str) -> Result<String, SkillEvalError> {
    let version = output.trim();
    if version.is_empty() || version.lines().count() != 1 || version.chars().any(char::is_control) {
        return Err(invalid("Pi version output is malformed"));
    }
    Ok(version.to_owned())
}

fn model_key(provider: &str, model: &str, source: &str) -> Result<ModelKey, SkillEvalError> {
    if provider.contains('/')
        || !is_identity_segment(provider)
        || !model.split('/').all(is_identity_segment)
    {
        return Err(invalid(format!(
            "{source} has malformed model identity {provider}/{model}"
        )));
    }
    Ok(ModelKey {
        provider: provider.to_owned(),
        model: model.to_owned(),
    })
}

fn is_identity_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment != "."
        && segment != ".."
        && segment.chars().all(|character| {
            character.is_ascii_alphanumeric()
                || matches!(character, '-' | '.' | '_' | ':' | '+' | '@')
        })
}

fn parse_token_count(value: &str) -> Option<u64> {
    let (number, multiplier) = match value.as_bytes().last().copied() {
        Some(b'K') => (&value[..value.len() - 1], 1_000_u64),
        Some(b'M') => (&value[..value.len() - 1], 1_000_000_u64),
        _ => (value, 1_u64),
    };
    let (whole, fractional) = number.split_once('.').unwrap_or((number, ""));
    if whole.is_empty()
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || !fractional.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let scale = 10_u64.checked_pow(u32::try_from(fractional.len()).ok()?)?;
    let fractional = if fractional.is_empty() {
        0
    } else {
        fractional.parse::<u64>().ok()?
    };
    let numerator = whole
        .parse::<u64>()
        .ok()?
        .checked_mul(scale)?
        .checked_add(fractional)?
        .checked_mul(multiplier)?;
    (numerator > 0 && numerator % scale == 0).then_some(numerator / scale)
}

fn prepare_destination(
    repository_root: &Path,
    output_path: &Path,
) -> Result<PathBuf, SkillEvalError> {
    if output_path.as_os_str().is_empty()
        || output_path.is_absolute()
        || output_path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        || output_path.to_string_lossy().chars().any(char::is_control)
    {
        return Err(invalid(
            "model capability output must be a safe repository-relative path",
        ));
    }
    let repository_root =
        fs::canonicalize(repository_root).map_err(|error| SkillEvalError::Io {
            path: repository_root.to_path_buf(),
            message: error.to_string(),
        })?;
    let destination = repository_root.join(output_path);
    let parent = destination
        .parent()
        .ok_or_else(|| invalid("model capability output has no parent directory"))?;
    reject_symlink_components(
        &repository_root,
        output_path.parent().unwrap_or(Path::new("")),
    )?;
    fs::create_dir_all(parent).map_err(|error| SkillEvalError::Io {
        path: parent.to_path_buf(),
        message: error.to_string(),
    })?;
    reject_symlink_components(
        &repository_root,
        output_path.parent().unwrap_or(Path::new("")),
    )?;
    let canonical_parent = fs::canonicalize(parent).map_err(|error| SkillEvalError::Io {
        path: parent.to_path_buf(),
        message: error.to_string(),
    })?;
    if !canonical_parent.starts_with(&repository_root) {
        return Err(invalid(
            "model capability output escapes the repository root",
        ));
    }
    match fs::symlink_metadata(&destination) {
        Ok(_) => Err(invalid(format!(
            "model capability output {} already exists",
            output_path.display()
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(destination),
        Err(error) => Err(SkillEvalError::Io {
            path: destination,
            message: error.to_string(),
        }),
    }
}

fn reject_symlink_components(root: &Path, relative: &Path) -> Result<(), SkillEvalError> {
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(invalid(format!(
                    "model capability output path contains symlink {}",
                    current.display()
                )));
            }
            Ok(metadata) if !metadata.is_dir() => {
                return Err(invalid(format!(
                    "model capability output parent {} is not a directory",
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

fn write_new_atomic(path: &Path, bytes: &[u8]) -> Result<(), SkillEvalError> {
    let parent = path
        .parent()
        .ok_or_else(|| invalid("model capability output has no parent directory"))?;
    let file_name = path
        .file_name()
        .ok_or_else(|| invalid("model capability output has no file name"))?
        .to_string_lossy();
    let sequence = TEMPORARY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temporary = parent.join(format!(
        ".{file_name}.{}.{sequence}.tmp",
        std::process::id()
    ));
    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|error| io_error(&temporary, error))?;
        file.write_all(bytes)
            .and_then(|()| file.sync_all())
            .map_err(|error| io_error(&temporary, error))?;
        fs::hard_link(&temporary, path).map_err(|error| io_error(path, error))?;
        fs::remove_file(&temporary).map_err(|error| io_error(&temporary, error))?;
        File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| io_error(parent, error))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn io_error(path: &Path, error: std::io::Error) -> SkillEvalError {
    SkillEvalError::Io {
        path: path.to_path_buf(),
        message: error.to_string(),
    }
}

fn invalid(message: impl Into<String>) -> SkillEvalError {
    SkillEvalError::InvalidConfiguration(message.into())
}
