use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::Deserialize;

use crate::model::{ModelIdentity, SkillEvalError, Tier};
use crate::ports::ModelResolver as ModelResolverPort;

#[derive(Debug)]
pub(crate) struct ConfiguredModelResolver {
    configuration: RoutingConfiguration,
    catalog: BTreeMap<ModelKey, CatalogModel>,
}

impl ConfiguredModelResolver {
    pub(crate) fn load(
        configuration_path: &Path,
        catalog_output: &str,
    ) -> Result<Self, SkillEvalError> {
        let configuration_text =
            fs::read_to_string(configuration_path).map_err(|error| SkillEvalError::Io {
                path: configuration_path.to_path_buf(),
                message: error.to_string(),
            })?;
        Self::from_text(&configuration_text, catalog_output)
    }

    fn from_text(configuration_text: &str, catalog_output: &str) -> Result<Self, SkillEvalError> {
        let configuration = serde_json::from_str(configuration_text).map_err(|error| {
            SkillEvalError::InvalidConfiguration(format!(
                "model tier configuration is malformed: {error}"
            ))
        })?;
        let catalog = parse_catalog(catalog_output)?;
        Ok(Self {
            configuration,
            catalog,
        })
    }

    fn route(&self, tier: Tier) -> Result<&TierRoute, SkillEvalError> {
        self.configuration
            .tiers
            .get(tier_name(tier))
            .ok_or_else(|| {
                SkillEvalError::InvalidConfiguration(format!(
                    "model tier {} is not configured",
                    tier_name(tier)
                ))
            })
    }

    fn identity(
        &self,
        tier: Tier,
        route: &TierRoute,
        identifier: &str,
    ) -> Result<ModelIdentity, SkillEvalError> {
        let key = parse_identifier(identifier)?;
        let model = self.catalog.get(&key).ok_or_else(|| {
            SkillEvalError::InvalidConfiguration(format!(
                "model {identifier} is not available in the Pi catalog"
            ))
        })?;
        Ok(identity(tier, route, key, model))
    }

    fn is_control(&self, requested: &ModelKey) -> Result<bool, SkillEvalError> {
        for control in self.configuration.unranked_controls.values() {
            if parse_identifier(&control.pi)? == *requested {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

// TODO(AGNT-0032.T102): Resolve a distinct configured external judge for pool candidates.
impl ModelResolverPort for ConfiguredModelResolver {
    fn candidates(&self, tier: Tier) -> Result<Vec<ModelIdentity>, SkillEvalError> {
        let route = self.route(tier)?;
        validate_thinking(&route.thinking)?;
        let identifiers =
            std::iter::once(route.pi.as_str()).chain(route.fallbacks.iter().map(String::as_str));
        let candidates = identifiers
            .map(|identifier| self.identity(tier, route, identifier))
            .collect::<Result<Vec<_>, _>>()?;
        if candidates.is_empty() {
            return Err(SkillEvalError::InvalidConfiguration(format!(
                "model tier {} has an empty route",
                tier_name(tier)
            )));
        }
        Ok(candidates)
    }

    fn exact_candidate(&self, requested: &ModelIdentity) -> Result<ModelIdentity, SkillEvalError> {
        validate_thinking(&requested.thinking)?;
        let key = exact_model_key(requested)?;
        if is_moving_alias(&key.model) {
            return Err(SkillEvalError::InvalidConfiguration(format!(
                "exact model route {}/{} uses a moving alias",
                key.provider, key.model
            )));
        }
        if self.is_control(&key)? {
            return Err(SkillEvalError::InvalidConfiguration(format!(
                "exact model route {}/{} is an unranked control",
                key.provider, key.model
            )));
        }
        let catalog_model = self.catalog.get(&key).ok_or_else(|| {
            SkillEvalError::InvalidConfiguration(format!(
                "exact model route {}/{} is not available in the Pi catalog",
                key.provider, key.model
            ))
        })?;
        if requested.thinking != "off" && !catalog_model.is_thinking_supported {
            return Err(SkillEvalError::InvalidConfiguration(format!(
                "exact model route {}/{} does not support thinking level {:?}",
                key.provider, key.model, requested.thinking
            )));
        }

        let effective = ModelIdentity {
            tier: requested.tier,
            provider: key.provider,
            model: key.model,
            thinking: requested.thinking.clone(),
        };
        if effective != *requested {
            return Err(SkillEvalError::InvalidConfiguration(
                "exact model route returned a different effective identity".to_owned(),
            ));
        }
        Ok(effective)
    }

    fn configured_judge_tier(&self) -> Result<Tier, SkillEvalError> {
        let tier = parse_tier(&self.configuration.judge)?;
        self.route(tier)?;
        Ok(tier)
    }

    fn judge(
        &self,
        judge_tier: Tier,
        candidate: Option<&ModelIdentity>,
    ) -> Result<ModelIdentity, SkillEvalError> {
        if let Some(candidate) = candidate
            && judge_tier <= candidate.tier
        {
            return Err(SkillEvalError::InvalidConfiguration(format!(
                "judge tier {} must be above candidate tier {}",
                tier_name(judge_tier),
                tier_name(candidate.tier)
            )));
        }

        let route = self.route(judge_tier)?;
        validate_thinking(&route.thinking)?;
        let identifiers =
            std::iter::once(route.pi.as_str()).chain(route.fallbacks.iter().map(String::as_str));
        for identifier in identifiers {
            let key = parse_identifier(identifier)?;
            if candidate.is_some_and(|candidate| is_same_model(candidate, &key)) {
                continue;
            }
            if let Some(model) = self.catalog.get(&key) {
                return Ok(identity(judge_tier, route, key, model));
            }
        }

        match candidate {
            Some(candidate) => Err(SkillEvalError::JudgeUnavailable {
                candidate: candidate.clone(),
                judge_tier,
            }),
            None => Err(SkillEvalError::InvalidConfiguration(format!(
                "judge tier {} has no available model",
                tier_name(judge_tier)
            ))),
        }
    }
}

#[derive(Debug, Deserialize)]
struct RoutingConfiguration {
    tiers: BTreeMap<String, TierRoute>,
    judge: String,
    #[serde(default)]
    unranked_controls: BTreeMap<String, UnrankedControl>,
}

#[derive(Debug, Deserialize)]
struct TierRoute {
    pi: String,
    #[serde(default)]
    fallbacks: Vec<String>,
    thinking: String,
}

#[derive(Debug, Deserialize)]
// TODO(AGNT-0032.T93): Compile capability and control routes outside ranked tier authority.
struct CapabilityRoute {
    pi: String,
    #[serde(default)]
    fallbacks: Vec<String>,
    thinking: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UnrankedControl {
    pi: String,
    maximum_tier: String,
    is_read_only: bool,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ModelKey {
    provider: String,
    model: String,
}

#[derive(Debug)]
struct CatalogModel {
    is_thinking_supported: bool,
}

fn parse_catalog(output: &str) -> Result<BTreeMap<ModelKey, CatalogModel>, SkillEvalError> {
    let mut lines = output.lines().filter(|line| !line.trim().is_empty());
    let header = lines.next().ok_or_else(|| {
        SkillEvalError::InvalidConfiguration("Pi model catalog is empty".to_owned())
    })?;
    let expected_header = [
        "provider", "model", "context", "max-out", "thinking", "images",
    ];
    if !header.split_whitespace().eq(expected_header) {
        return Err(SkillEvalError::InvalidConfiguration(
            "Pi model catalog header is malformed".to_owned(),
        ));
    }

    let mut catalog = BTreeMap::new();
    for (index, line) in lines.enumerate() {
        let columns = line.split_whitespace().collect::<Vec<_>>();
        if columns.len() != expected_header.len() {
            return Err(SkillEvalError::InvalidConfiguration(format!(
                "Pi model catalog line {} is malformed",
                index + 2
            )));
        }
        let key = catalog_key(columns[0], columns[1], index + 2)?;
        let is_thinking_supported = match columns[4] {
            "yes" => true,
            "no" => false,
            _ => {
                return Err(SkillEvalError::InvalidConfiguration(format!(
                    "Pi model catalog line {} has malformed thinking metadata",
                    index + 2
                )));
            }
        };
        if catalog
            .insert(
                key,
                CatalogModel {
                    is_thinking_supported,
                },
            )
            .is_some()
        {
            return Err(SkillEvalError::InvalidConfiguration(format!(
                "Pi model catalog line {} duplicates a model",
                index + 2
            )));
        }
    }
    Ok(catalog)
}

fn catalog_key(provider: &str, model: &str, line: usize) -> Result<ModelKey, SkillEvalError> {
    if provider.is_empty()
        || provider.contains('/')
        || model.is_empty()
        || model.split('/').any(str::is_empty)
    {
        return Err(SkillEvalError::InvalidConfiguration(format!(
            "Pi model catalog line {line} has a malformed model identity"
        )));
    }
    Ok(ModelKey {
        provider: provider.to_owned(),
        model: model.to_owned(),
    })
}

fn parse_identifier(identifier: &str) -> Result<ModelKey, SkillEvalError> {
    let (provider, model) = identifier.split_once('/').ok_or_else(|| {
        SkillEvalError::InvalidConfiguration(format!(
            "model identifier {identifier:?} must use provider/model"
        ))
    })?;
    if identifier.trim() != identifier
        || identifier.chars().any(char::is_whitespace)
        || provider.is_empty()
        || model.is_empty()
        || model.split('/').any(str::is_empty)
    {
        return Err(SkillEvalError::InvalidConfiguration(format!(
            "model identifier {identifier:?} is malformed"
        )));
    }
    Ok(ModelKey {
        provider: provider.to_owned(),
        model: model.to_owned(),
    })
}

fn exact_model_key(requested: &ModelIdentity) -> Result<ModelKey, SkillEvalError> {
    if !is_exact_segment(&requested.provider)
        || requested.provider.contains('/')
        || !requested.model.split('/').all(is_exact_segment)
    {
        return Err(SkillEvalError::InvalidConfiguration(
            "exact model route is malformed".to_owned(),
        ));
    }
    Ok(ModelKey {
        provider: requested.provider.clone(),
        model: requested.model.clone(),
    })
}

fn is_exact_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment != "."
        && segment != ".."
        && !segment
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
}

fn is_moving_alias(model: &str) -> bool {
    model
        .rsplit(['/', '-', ':', '@'])
        .next()
        .is_some_and(|segment| segment == "latest")
}

fn identity(
    tier: Tier,
    route: &TierRoute,
    key: ModelKey,
    catalog_model: &CatalogModel,
) -> ModelIdentity {
    ModelIdentity {
        tier,
        provider: key.provider,
        model: key.model,
        thinking: if catalog_model.is_thinking_supported {
            route.thinking.clone()
        } else {
            "off".to_owned()
        },
    }
}

fn is_same_model(candidate: &ModelIdentity, key: &ModelKey) -> bool {
    candidate.provider == key.provider && candidate.model == key.model
}

fn validate_thinking(thinking: &str) -> Result<(), SkillEvalError> {
    if ["off", "minimal", "low", "medium", "high", "xhigh", "max"].contains(&thinking) {
        Ok(())
    } else {
        Err(SkillEvalError::InvalidConfiguration(format!(
            "thinking level {thinking:?} is malformed"
        )))
    }
}

fn parse_tier(tier: &str) -> Result<Tier, SkillEvalError> {
    match tier {
        "T1" => Ok(Tier::T1),
        "T2" => Ok(Tier::T2),
        "T3" => Ok(Tier::T3),
        "T4" => Ok(Tier::T4),
        "T5" => Ok(Tier::T5),
        _ => Err(SkillEvalError::InvalidConfiguration(format!(
            "unknown model tier {tier:?}"
        ))),
    }
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

#[cfg(test)]
fn resolver(configuration: &str, catalog: &str) -> ConfiguredModelResolver {
    ConfiguredModelResolver::from_text(configuration, catalog).expect("fixture must parse")
}

#[cfg(test)]
#[test]
fn candidates_preserve_primary_and_fallback_order() {
    let resolver = resolver(
        include_str!("../../../config/model-tiers.json"),
        include_str!("../tests/fixtures/models/catalog-all.txt"),
    );

    let candidates = resolver.candidates(Tier::T3).expect("T3 must resolve");

    assert_eq!(
        candidates,
        vec![
            ModelIdentity {
                tier: Tier::T3,
                provider: "openai-codex".to_owned(),
                model: "gpt-5.3-codex-spark".to_owned(),
                thinking: "medium".to_owned(),
            },
            ModelIdentity {
                tier: Tier::T3,
                provider: "anthropic".to_owned(),
                model: "claude-sonnet-5".to_owned(),
                thinking: "medium".to_owned(),
            },
            ModelIdentity {
                tier: Tier::T3,
                provider: "openai-codex".to_owned(),
                model: "gpt-5.6-terra".to_owned(),
                thinking: "medium".to_owned(),
            },
        ]
    );
}

#[cfg(test)]
#[test]
fn candidates_use_off_when_catalog_disables_thinking() {
    let resolver = resolver(
        include_str!("../tests/fixtures/models/routing-non-thinking.json"),
        include_str!("../tests/fixtures/models/catalog-non-thinking.txt"),
    );

    let candidates = resolver.candidates(Tier::T1).expect("T1 must resolve");

    assert_eq!(candidates[0].thinking, "off");
}

#[cfg(test)]
#[test]
fn configured_judge_tier_reads_tracked_value() {
    let resolver = resolver(
        include_str!("../../../config/model-tiers.json"),
        include_str!("../tests/fixtures/models/catalog-all.txt"),
    );

    assert_eq!(resolver.configured_judge_tier(), Ok(Tier::T5));
}

#[cfg(test)]
#[test]
fn candidate_fallback_is_skipped() {
    let resolver = resolver(
        include_str!("../tests/fixtures/models/routing-judge-fallback.json"),
        include_str!("../tests/fixtures/models/catalog-judge-fallback.txt"),
    );
    let candidate = ModelIdentity {
        tier: Tier::T3,
        provider: "openai-codex".to_owned(),
        model: "gpt-5.3-codex-spark".to_owned(),
        thinking: "medium".to_owned(),
    };

    let judge = resolver
        .judge(Tier::T5, Some(&candidate))
        .expect("external fallback must resolve");

    assert_eq!(judge.provider, "anthropic");
    assert_eq!(judge.model, "claude-opus-5");
}

#[cfg(test)]
#[test]
fn candidate_only_fallback_returns_judge_unavailable() {
    let resolver = resolver(
        include_str!("../tests/fixtures/models/routing-judge-fallback.json"),
        include_str!("../tests/fixtures/models/catalog-candidate-only.txt"),
    );
    let candidate = ModelIdentity {
        tier: Tier::T3,
        provider: "openai-codex".to_owned(),
        model: "gpt-5.3-codex-spark".to_owned(),
        thinking: "medium".to_owned(),
    };

    assert_eq!(
        resolver.judge(Tier::T5, Some(&candidate)),
        Err(SkillEvalError::JudgeUnavailable {
            candidate,
            judge_tier: Tier::T5,
        })
    );
}

#[cfg(test)]
#[test]
fn judge_tier_must_be_above_candidate_tier() {
    let resolver = resolver(
        include_str!("../tests/fixtures/models/routing-judge-fallback.json"),
        include_str!("../tests/fixtures/models/catalog-judge-fallback.txt"),
    );
    let candidate = ModelIdentity {
        tier: Tier::T5,
        provider: "openai-codex".to_owned(),
        model: "gpt-5.3-codex-spark".to_owned(),
        thinking: "medium".to_owned(),
    };

    assert!(matches!(
        resolver.judge(Tier::T5, Some(&candidate)),
        Err(SkillEvalError::InvalidConfiguration(message))
            if message.contains("must be above")
    ));
}

#[cfg(test)]
#[test]
fn unknown_tier_fails() {
    let resolver = resolver(
        include_str!("../tests/fixtures/models/routing-unknown-judge.json"),
        include_str!("../tests/fixtures/models/catalog-all.txt"),
    );

    assert!(matches!(
        resolver.configured_judge_tier(),
        Err(SkillEvalError::InvalidConfiguration(message)) if message.contains("T9")
    ));
}

#[cfg(test)]
#[test]
fn malformed_model_identifier_fails_without_guessing() {
    let resolver = resolver(
        include_str!("../tests/fixtures/models/routing-malformed-model.json"),
        include_str!("../tests/fixtures/models/catalog-all.txt"),
    );

    assert!(matches!(
        resolver.candidates(Tier::T1),
        Err(SkillEvalError::InvalidConfiguration(message))
            if message.contains("provider/model")
    ));
}

#[cfg(test)]
#[test]
fn unavailable_candidate_metadata_fails() {
    let resolver = resolver(
        include_str!("../../../config/model-tiers.json"),
        include_str!("../tests/fixtures/models/catalog-candidate-only.txt"),
    );

    assert!(matches!(
        resolver.candidates(Tier::T3),
        Err(SkillEvalError::InvalidConfiguration(message))
            if message.contains("not available")
    ));
}
