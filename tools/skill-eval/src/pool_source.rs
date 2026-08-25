use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

use crate::model::{
    ModelIdentity, PoolEntrant, PoolPlan, PoolPolicy, SkillEvalError, Tier, Timestamp,
};
use crate::ports::PoolPlanSource;

const ENTRANTS_PER_TIER: usize = 3;
const MAXIMUM_THINKING_LEVELS: usize = 3;
const PROMOTION_COUNT: u8 = 2;
const THINKING_LEVELS: [&str; 7] = ["off", "minimal", "low", "medium", "high", "xhigh", "max"];

/// Loads frozen model-pool plans from one repository root.
///
/// The input is a repository directory. The output is a source restricted to that directory.
///
/// # Errors
///
/// Returns an error when the repository path is missing, unreadable, or not a directory.
pub(crate) struct FilePoolPlanSource {
    repository_root: PathBuf,
}

impl FilePoolPlanSource {
    /// Creates a source restricted to `repository_root`.
    ///
    /// The input is a repository directory. The output is a canonical repository-backed source.
    ///
    /// # Errors
    ///
    /// Returns an error when the path is missing, unreadable, or not a directory.
    pub(crate) fn new(repository_root: &Path) -> Result<Self, SkillEvalError> {
        let repository_root =
            fs::canonicalize(repository_root).map_err(|error| io_error(repository_root, error))?;
        if !repository_root.is_dir() {
            return Err(invalid(format!(
                "pool-plan repository root {} is not a directory",
                repository_root.display()
            )));
        }
        Ok(Self { repository_root })
    }
}

impl PoolPlanSource for FilePoolPlanSource {
    fn load_pool_plan(&self, path: &Path) -> Result<PoolPlan, SkillEvalError> {
        validate_relative_path(path)?;
        let joined = self.repository_root.join(path);
        let canonical = fs::canonicalize(&joined).map_err(|error| io_error(&joined, error))?;
        if !canonical.starts_with(&self.repository_root) {
            return Err(invalid(format!(
                "pool-plan path {} escapes repository root {}",
                path.display(),
                self.repository_root.display()
            )));
        }
        if !canonical.is_file() {
            return Err(invalid(format!(
                "pool-plan path {} is not a file",
                path.display()
            )));
        }

        let text = fs::read_to_string(&canonical).map_err(|error| io_error(&canonical, error))?;
        let raw: RawPoolPlan = serde_json::from_str(&text).map_err(|error| {
            invalid(format!(
                "pool plan {} is malformed: {error}",
                path.display()
            ))
        })?;
        normalize(raw)
    }

    fn validate_pool_plan_freshness(
        &self,
        plan: &PoolPlan,
        now: &Timestamp,
    ) -> Result<(), SkillEvalError> {
        if plan.policy.maximum_catalog_age_seconds == 0 {
            return Err(invalid(
                "pool policy maximum_catalog_age_seconds must be positive",
            ));
        }

        let now_seconds = parse_timestamp(&now.0, "runtime timestamp")?;
        let maximum_age = u64::from(plan.policy.maximum_catalog_age_seconds);
        for (tier, entrants) in &plan.entrants {
            for entrant in entrants {
                let field = format!(
                    "{} catalog_observed_at for {}/{}",
                    tier_name(*tier),
                    entrant.model.provider,
                    entrant.model.model
                );
                let observed_seconds = parse_timestamp(&entrant.catalog_observed_at.0, &field)?;
                let age = now_seconds.checked_sub(observed_seconds).ok_or_else(|| {
                    invalid(format!(
                        "pool plan {field} is in the future relative to runtime timestamp"
                    ))
                })?;
                if age > maximum_age {
                    return Err(invalid(format!(
                        "pool plan {field} is older than maximum_catalog_age_seconds"
                    )));
                }
            }
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPoolPlan {
    entrants: RawEntrants,
    control: RawControl,
    policy: RawPoolPolicy,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEntrants {
    #[serde(rename = "T1")]
    t1: Vec<RawEntrant>,
    #[serde(rename = "T2")]
    t2: Vec<RawEntrant>,
    #[serde(rename = "T3")]
    t3: Vec<RawEntrant>,
    #[serde(rename = "T4")]
    t4: Vec<RawEntrant>,
    #[serde(rename = "T5")]
    t5: Vec<RawEntrant>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEntrant {
    model: RawModelIdentity,
    thinking_levels: Vec<String>,
    catalog_observed_at: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawControl {
    model: RawModelIdentity,
    maximum_tier: RawTier,
    is_read_only: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawModelIdentity {
    provider: String,
    model: String,
    thinking: String,
}

#[derive(Clone, Copy, Deserialize)]
enum RawTier {
    T1,
    T2,
    T3,
    T4,
    T5,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPoolPolicy {
    calibration_repeats_per_case: u16,
    qualification_repeats_per_case: u16,
    promotion_count: u8,
    minimum_score: u8,
    minimum_reliability_basis_points: u16,
    maximum_catalog_age_seconds: u32,
    spending_limit_millionths_of_dollar: u64,
    is_provider_limit_enforced: bool,
}

fn normalize(raw: RawPoolPlan) -> Result<PoolPlan, SkillEvalError> {
    let policy = normalize_policy(raw.policy)?;
    let control = normalize_control(raw.control)?;
    let control_route = (control.provider.clone(), control.model.clone());
    let mut identities = BTreeSet::new();
    let mut entrants = BTreeMap::new();

    for (tier, raw_entrants) in [
        (Tier::T1, raw.entrants.t1),
        (Tier::T2, raw.entrants.t2),
        (Tier::T3, raw.entrants.t3),
        (Tier::T4, raw.entrants.t4),
        (Tier::T5, raw.entrants.t5),
    ] {
        let tier_name = tier_name(tier);
        if raw_entrants.len() != ENTRANTS_PER_TIER {
            return Err(invalid(format!(
                "pool plan tier {tier_name} must contain exactly {ENTRANTS_PER_TIER} entrants"
            )));
        }

        let mut normalized = Vec::with_capacity(ENTRANTS_PER_TIER);
        for entrant in raw_entrants {
            parse_timestamp(
                &entrant.catalog_observed_at,
                &format!("{tier_name} catalog_observed_at"),
            )?;
            let model = normalize_model(tier, entrant.model)?;
            let identity = (
                model.provider.clone(),
                model.model.clone(),
                model.thinking.clone(),
            );
            if !identities.insert(identity) {
                return Err(invalid(format!(
                    "pool plan contains duplicate model identity {}/{} with thinking {:?}",
                    model.provider, model.model, model.thinking
                )));
            }
            if (model.provider.clone(), model.model.clone()) == control_route {
                return Err(invalid(format!(
                    "unranked control {}/{} cannot be a ranked entrant",
                    model.provider, model.model
                )));
            }
            let thinking_levels = normalize_thinking_levels(&model, entrant.thinking_levels)?;
            normalized.push(PoolEntrant {
                model,
                thinking_levels,
                catalog_observed_at: Timestamp(entrant.catalog_observed_at),
            });
        }
        entrants.insert(tier, normalized);
    }

    Ok(PoolPlan {
        entrants,
        control,
        policy,
    })
}

fn normalize_control(raw: RawControl) -> Result<ModelIdentity, SkillEvalError> {
    if !raw.is_read_only {
        return Err(invalid("pool-plan control must be read-only"));
    }
    if !matches!(raw.maximum_tier, RawTier::T1) {
        return Err(invalid("pool-plan control maximum tier must be T1"));
    }
    normalize_model(Tier::T1, raw.model)
}

fn normalize_model(tier: Tier, raw: RawModelIdentity) -> Result<ModelIdentity, SkillEvalError> {
    if !is_exact_segment(&raw.provider)
        || raw.provider.contains('/')
        || !raw.model.split('/').all(is_exact_segment)
    {
        return Err(invalid(format!(
            "pool plan contains malformed model identity {}/{}",
            raw.provider, raw.model
        )));
    }
    if is_moving_alias(&raw.model) {
        return Err(invalid(format!(
            "pool plan model {}/{} uses a moving alias",
            raw.provider, raw.model
        )));
    }
    if !THINKING_LEVELS.contains(&raw.thinking.as_str()) {
        return Err(invalid(format!(
            "pool plan model {}/{} has invalid thinking value {:?}",
            raw.provider, raw.model, raw.thinking
        )));
    }
    Ok(ModelIdentity {
        tier,
        provider: raw.provider,
        model: raw.model,
        thinking: raw.thinking,
    })
}

// TODO(AGNT-0032.T103): Validate and preserve one to three ordered model-specific levels.
fn normalize_thinking_levels(
    model: &ModelIdentity,
    levels: Vec<String>,
) -> Result<Vec<String>, SkillEvalError> {
    if levels.is_empty() || levels.len() > MAXIMUM_THINKING_LEVELS {
        return Err(invalid(format!(
            "pool plan model {}/{} must declare one to {MAXIMUM_THINKING_LEVELS} thinking levels",
            model.provider, model.model
        )));
    }

    let mut previous_rank = None;
    let mut start_count = 0;
    for level in &levels {
        let rank = THINKING_LEVELS
            .iter()
            .position(|supported| *supported == level.as_str())
            .ok_or_else(|| {
                invalid(format!(
                    "pool plan model {}/{} has unsupported thinking level {level:?}",
                    model.provider, model.model
                ))
            })?;
        if let Some(previous_rank) = previous_rank {
            if rank == previous_rank {
                return Err(invalid(format!(
                    "pool plan model {}/{} contains duplicate thinking level {level:?}",
                    model.provider, model.model
                )));
            }
            if rank < previous_rank {
                return Err(invalid(format!(
                    "pool plan model {}/{} thinking levels must be ordered cheapest to strongest",
                    model.provider, model.model
                )));
            }
        }
        start_count += usize::from(level == &model.thinking);
        previous_rank = Some(rank);
    }

    if start_count != 1 {
        return Err(invalid(format!(
            "pool plan model {}/{} starting thinking {:?} must appear exactly once",
            model.provider, model.model, model.thinking
        )));
    }

    Ok(levels)
}

fn normalize_policy(raw: RawPoolPolicy) -> Result<PoolPolicy, SkillEvalError> {
    if raw.calibration_repeats_per_case == 0 {
        return Err(invalid(
            "pool policy calibration_repeats_per_case must be positive",
        ));
    }
    if raw.qualification_repeats_per_case == 0 {
        return Err(invalid(
            "pool policy qualification_repeats_per_case must be positive",
        ));
    }
    if raw.promotion_count != PROMOTION_COUNT {
        return Err(invalid(format!(
            "pool policy promotion_count must be {PROMOTION_COUNT}"
        )));
    }
    if raw.minimum_score > 10 {
        return Err(invalid("pool policy minimum_score must not exceed 10"));
    }
    if raw.minimum_reliability_basis_points > 10_000 {
        return Err(invalid(
            "pool policy minimum_reliability_basis_points must not exceed 10000",
        ));
    }
    if raw.maximum_catalog_age_seconds == 0 {
        return Err(invalid(
            "pool policy maximum_catalog_age_seconds must be positive",
        ));
    }
    if raw.spending_limit_millionths_of_dollar == 0 {
        return Err(invalid(
            "pool policy spending_limit_millionths_of_dollar must be positive",
        ));
    }
    if !raw.is_provider_limit_enforced {
        return Err(invalid(
            "paid pool plan requires a provider-enforced spending limit",
        ));
    }

    Ok(PoolPolicy {
        calibration_repeats_per_case: raw.calibration_repeats_per_case,
        qualification_repeats_per_case: raw.qualification_repeats_per_case,
        promotion_count: raw.promotion_count,
        minimum_score: raw.minimum_score,
        minimum_reliability_basis_points: raw.minimum_reliability_basis_points,
        maximum_catalog_age_seconds: raw.maximum_catalog_age_seconds,
        spending_limit_millionths_of_dollar: raw.spending_limit_millionths_of_dollar,
        is_provider_limit_enforced: raw.is_provider_limit_enforced,
    })
}

fn validate_relative_path(path: &Path) -> Result<(), SkillEvalError> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err(invalid(format!(
            "pool-plan path {} must be repository-relative",
            path.display()
        )));
    }
    for component in path.components() {
        if !matches!(component, Component::Normal(_)) {
            return Err(invalid(format!(
                "pool-plan path {} contains an invalid component",
                path.display()
            )));
        }
    }
    Ok(())
}

fn parse_timestamp(value: &str, field: &str) -> Result<u64, SkillEvalError> {
    const SECONDS_PER_DAY: u64 = 86_400;
    const SECONDS_PER_HOUR: u64 = 3_600;
    const SECONDS_PER_MINUTE: u64 = 60;

    let bytes = value.as_bytes();
    let is_shape_valid = bytes.len() == 24
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[10] == b'T'
        && bytes[13] == b':'
        && bytes[16] == b':'
        && matches!(bytes[19], b'+' | b'-')
        && bytes.iter().enumerate().all(|(index, byte)| {
            matches!(index, 4 | 7 | 10 | 13 | 16 | 19) || byte.is_ascii_digit()
        });
    if !is_shape_valid {
        return Err(invalid(format!(
            "pool plan {field} must use YYYY-MM-DDTHH:MM:SS+HHMM"
        )));
    }

    let year = parse_time_part(value, 0, 4, field)?;
    let month = parse_time_part(value, 5, 7, field)?;
    let day = parse_time_part(value, 8, 10, field)?;
    let hour = parse_time_part(value, 11, 13, field)?;
    let minute = parse_time_part(value, 14, 16, field)?;
    let second = parse_time_part(value, 17, 19, field)?;
    let offset_hour = parse_time_part(value, 20, 22, field)?;
    let offset_minute = parse_time_part(value, 22, 24, field)?;
    let maximum_day = days_in_month(year, month);

    if year == 0
        || maximum_day == 0
        || day == 0
        || day > maximum_day
        || hour > 23
        || minute > 59
        || second > 59
        || offset_hour > 14
        || offset_minute > 59
        || (offset_hour == 14 && offset_minute != 0)
    {
        return Err(invalid(format!(
            "pool plan {field} contains an invalid date, time, or numeric offset"
        )));
    }

    let days =
        days_before_year(year) + u64::from(days_before_month(year, month)) + u64::from(day - 1);
    let local_seconds = days * SECONDS_PER_DAY
        + u64::from(hour) * SECONDS_PER_HOUR
        + u64::from(minute) * SECONDS_PER_MINUTE
        + u64::from(second);
    let offset_seconds =
        u64::from(offset_hour) * SECONDS_PER_HOUR + u64::from(offset_minute) * SECONDS_PER_MINUTE;
    let maximum_seconds = days_before_year(10_000) * SECONDS_PER_DAY - 1;
    let utc_seconds = match bytes[19] {
        b'+' => local_seconds.checked_sub(offset_seconds),
        b'-' => local_seconds.checked_add(offset_seconds),
        _ => unreachable!(),
    }
    .filter(|seconds| *seconds <= maximum_seconds)
    .ok_or_else(|| invalid(format!("pool plan {field} timestamp arithmetic overflow")))?;

    Ok(utc_seconds)
}

fn parse_time_part(
    value: &str,
    start: usize,
    end: usize,
    field: &str,
) -> Result<u32, SkillEvalError> {
    value[start..end]
        .parse()
        .map_err(|_| invalid(format!("pool plan {field} contains an invalid number")))
}

fn days_before_year(year: u32) -> u64 {
    let previous_year = u64::from(year - 1);
    previous_year * 365 + previous_year / 4 - previous_year / 100 + previous_year / 400
}

fn days_before_month(year: u32, month: u32) -> u32 {
    (1..month).map(|prior| days_in_month(year, prior)).sum()
}

fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_leap_year(year: u32) -> bool {
    year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
}

fn is_exact_segment(segment: &str) -> bool {
    !segment.is_empty()
        && !matches!(segment, "." | "..")
        && !segment
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
}

fn is_moving_alias(model: &str) -> bool {
    model
        .rsplit(['/', '-', ':', '@'])
        .next()
        .is_some_and(|segment| segment.eq_ignore_ascii_case("latest"))
}

fn tier_name(tier: Tier) -> &'static str {
    match tier {
        Tier::T1 => "T1",
        Tier::T2 => "T2",
        Tier::T3 => "T3",
        Tier::T4 => "T4",
        Tier::T5 => "T5",
    }
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
