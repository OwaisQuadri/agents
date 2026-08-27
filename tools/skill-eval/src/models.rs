use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::Path;

use serde::Deserialize;
use serde::de::{self, DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor};

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
        rpc_output: &str,
    ) -> Result<Self, SkillEvalError> {
        let configuration_text =
            fs::read_to_string(configuration_path).map_err(|error| SkillEvalError::Io {
                path: configuration_path.to_path_buf(),
                message: error.to_string(),
            })?;
        Self::from_inputs(&configuration_text, catalog_output, rpc_output)
    }

    fn from_inputs(
        configuration_text: &str,
        catalog_output: &str,
        rpc_output: &str,
    ) -> Result<Self, SkillEvalError> {
        reject_duplicate_keys(configuration_text)?;
        let configuration: RoutingConfiguration = serde_json::from_str(configuration_text)
            .map_err(|error| {
                SkillEvalError::InvalidConfiguration(format!(
                    "model tier configuration is malformed: {error}"
                ))
            })?;
        configuration.validate()?;
        let mut catalog = parse_catalog(catalog_output)?;
        merge_rpc_capabilities(&mut catalog, rpc_output)?;
        let resolver = Self {
            configuration,
            catalog,
        };
        resolver.validate_qualification_routes()?;
        Ok(resolver)
    }

    #[cfg(test)]
    fn from_text(configuration_text: &str, catalog_output: &str) -> Result<Self, SkillEvalError> {
        let rpc_output = test_rpc_output(catalog_output)?;
        Self::from_inputs(configuration_text, catalog_output, &rpc_output)
    }

    #[cfg(test)]
    fn from_text_with_rpc(
        configuration_text: &str,
        catalog_output: &str,
        rpc_output: &str,
    ) -> Result<Self, SkillEvalError> {
        Self::from_inputs(configuration_text, catalog_output, rpc_output)
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
        identity(tier, route, key, model)
    }

    fn is_control(&self, requested: &ModelKey) -> Result<bool, SkillEvalError> {
        for control in self.configuration.unranked_controls.values() {
            if parse_identifier(&control.pi)? == *requested {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn validate_qualification_routes(&self) -> Result<(), SkillEvalError> {
        let mut model_tier_ranges = BTreeMap::<(String, String), BTreeMap<Tier, (u8, u8)>>::new();
        for (tier_name, routes) in &self.configuration.qualification_routes {
            let tier = parse_tier(tier_name)?;
            if !self.configuration.tiers.contains_key(tier_name) {
                return Err(invalid_configuration(format!(
                    "qualification route tier {tier_name} is not configured"
                )));
            }
            if routes.is_empty() {
                return Err(invalid_configuration(format!(
                    "qualification route tier {tier_name} has an empty route order"
                )));
            }

            let mut exact_routes = BTreeSet::new();
            let mut last_thinking = BTreeMap::new();
            for route in routes {
                let requested = ModelIdentity {
                    tier,
                    provider: route.provider.clone(),
                    model: route.model.clone(),
                    thinking: route.thinking.clone(),
                };
                let exact = self.exact_candidate(&requested)?;
                let key = (
                    exact.provider.clone(),
                    exact.model.clone(),
                    exact.thinking.clone(),
                );
                if !exact_routes.insert(key) {
                    return Err(invalid_configuration(format!(
                        "qualification route tier {tier_name} contains a duplicate exact route"
                    )));
                }
                let model = (exact.provider.clone(), exact.model.clone());
                let thinking = thinking_rank(&exact.thinking);
                if last_thinking
                    .insert(model.clone(), thinking)
                    .is_some_and(|previous| previous >= thinking)
                {
                    return Err(invalid_configuration(format!(
                        "qualification route tier {tier_name} has malformed thinking order"
                    )));
                }
                model_tier_ranges
                    .entry(model)
                    .or_default()
                    .entry(tier)
                    .and_modify(|range| {
                        range.0 = range.0.min(thinking);
                        range.1 = range.1.max(thinking);
                    })
                    .or_insert((thinking, thinking));
            }
            u16::try_from(routes.len() - 1).map_err(|_| {
                invalid_configuration(format!(
                    "qualification route tier {tier_name} has too many exact routes"
                ))
            })?;
        }
        for ((provider, model), tiers) in model_tier_ranges {
            let mut previous = None;
            for (tier, (minimum, maximum)) in tiers {
                if previous.is_some_and(|(_, lower_maximum)| minimum <= lower_maximum) {
                    return Err(invalid_configuration(format!(
                        "qualification route {provider}/{model} must use strictly stronger thinking in higher tier {tier:?}"
                    )));
                }
                previous = Some((tier, maximum));
            }
        }
        Ok(())
    }
}

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

    fn qualification_routes(&self, tier: Tier) -> Result<Vec<ModelIdentity>, SkillEvalError> {
        let tier_name = tier_name(tier);
        let routes = self
            .configuration
            .qualification_routes
            .get(tier_name)
            .ok_or_else(|| {
                SkillEvalError::InvalidConfiguration(format!(
                    "artifact qualification route order for tier {tier_name} is absent"
                ))
            })?;
        if routes.is_empty() {
            return Err(SkillEvalError::InvalidConfiguration(format!(
                "artifact qualification route order for tier {tier_name} is absent"
            )));
        }
        routes
            .iter()
            .map(|route| {
                self.exact_candidate(&ModelIdentity {
                    tier,
                    provider: route.provider.clone(),
                    model: route.model.clone(),
                    thinking: route.thinking.clone(),
                })
            })
            .collect()
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
        validate_supported_thinking(&key, catalog_model, &requested.thinking)?;

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

    fn pool_judge(&self, candidate: &ModelIdentity) -> Result<ModelIdentity, SkillEvalError> {
        let judge_tier = self.configured_judge_tier()?;
        let is_eligible_tier = if candidate.tier == Tier::T5 {
            judge_tier == Tier::T5
        } else {
            judge_tier > candidate.tier
        };
        if !is_eligible_tier {
            return Err(SkillEvalError::InvalidConfiguration(format!(
                "pool judge tier {} is not above candidate tier {}",
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
            if is_same_model(candidate, &key) || self.is_control(&key)? {
                continue;
            }
            if let Some(model) = self.catalog.get(&key) {
                return identity(judge_tier, route, key, model);
            }
        }

        Err(SkillEvalError::JudgeUnavailable {
            candidate: candidate.clone(),
            judge_tier,
        })
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
                return identity(judge_tier, route, key, model);
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
#[serde(deny_unknown_fields)]
struct RoutingConfiguration {
    tiers: BTreeMap<String, TierRoute>,
    #[serde(default)]
    qualification_routes: BTreeMap<String, Vec<QualificationRoute>>,
    #[serde(default)]
    orchestrator: Option<String>,
    judge: String,
    #[serde(default)]
    capabilities: BTreeMap<String, CapabilityRoute>,
    #[serde(default)]
    unranked_controls: BTreeMap<String, UnrankedControl>,
    #[serde(default)]
    agents: BTreeMap<String, String>,
    #[serde(default)]
    untiered: BTreeMap<String, String>,
}

impl RoutingConfiguration {
    fn validate(&self) -> Result<(), SkillEvalError> {
        if self.tiers.is_empty() {
            return Err(invalid_configuration("model tiers are empty"));
        }

        let mut authority_routes = BTreeSet::new();
        for (tier_name, route) in &self.tiers {
            parse_tier(tier_name)?;
            validate_route(
                tier_name,
                &route.pi,
                &route.fallbacks,
                Some(&route.thinking),
            )?;
            authority_routes.extend(route_keys(&route.pi, &route.fallbacks)?);
        }

        self.validate_tier_reference("judge", &self.judge)?;
        if let Some(orchestrator) = &self.orchestrator {
            self.validate_tier_reference("orchestrator", orchestrator)?;
        }

        for (name, route) in &self.capabilities {
            validate_key("capability", name)?;
            validate_route(name, &route.pi, &route.fallbacks, route.thinking.as_deref())?;
        }

        let mut control_routes = BTreeSet::new();
        for (name, control) in &self.unranked_controls {
            validate_key("control", name)?;
            let route = parse_identifier(&control.pi)?;
            if !control_routes.insert(route.clone()) {
                return Err(invalid_configuration(format!(
                    "unranked control {name:?} duplicates another control route"
                )));
            }
            if parse_tier(&control.maximum_tier)? != Tier::T1 {
                return Err(invalid_configuration(format!(
                    "unranked control {name:?} maximum tier must be T1"
                )));
            }
            if !control.is_read_only {
                return Err(invalid_configuration(format!(
                    "unranked control {name:?} must be read-only"
                )));
            }
        }

        if !control_routes.is_empty()
            && let Some(route) = control_routes.intersection(&authority_routes).next()
        {
            return Err(invalid_configuration(format!(
                "unranked control route {}/{} cannot appear in ranked tier or judge authority",
                route.provider, route.model
            )));
        }

        for (name, tier) in &self.agents {
            validate_key("agent", name)?;
            self.validate_tier_reference("agent", tier)?;
        }
        for (name, reason) in &self.untiered {
            validate_key("untiered agent", name)?;
            if reason.trim().is_empty() {
                return Err(invalid_configuration(format!(
                    "untiered agent {name:?} has an empty reason"
                )));
            }
        }

        Ok(())
    }

    fn validate_tier_reference(&self, field: &str, tier: &str) -> Result<(), SkillEvalError> {
        parse_tier(tier)?;
        if !self.tiers.contains_key(tier) {
            return Err(invalid_configuration(format!(
                "{field} tier {tier} is not configured"
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TierRoute {
    pi: String,
    #[serde(default)]
    fallbacks: Vec<String>,
    thinking: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct QualificationRoute {
    provider: String,
    model: String,
    thinking: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CapabilityRoute {
    pi: String,
    #[serde(default)]
    fallbacks: Vec<String>,
    #[serde(default, deserialize_with = "deserialize_optional_thinking")]
    thinking: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
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
    supported_thinking_levels: Option<BTreeSet<String>>,
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
    #[serde(rename = "reasoning")]
    is_reasoning_supported: bool,
    #[serde(default)]
    thinking_level_map: Option<BTreeMap<String, Option<String>>>,
}

fn deserialize_optional_thinking<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    String::deserialize(deserializer).map(Some)
}

fn reject_duplicate_keys(configuration_text: &str) -> Result<(), SkillEvalError> {
    let mut deserializer = serde_json::Deserializer::from_str(configuration_text);
    UniqueValue
        .deserialize(&mut deserializer)
        .map_err(|error| {
            SkillEvalError::InvalidConfiguration(format!(
                "model tier configuration is malformed: {error}"
            ))
        })?;
    deserializer.end().map_err(|error| {
        SkillEvalError::InvalidConfiguration(format!(
            "model tier configuration is malformed: {error}"
        ))
    })
}

struct UniqueValue;

impl<'de> DeserializeSeed<'de> for UniqueValue {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for UniqueValue {
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value with unique object keys")
    }

    fn visit_bool<E>(self, _value: bool) -> Result<Self::Value, E> {
        Ok(())
    }

    fn visit_i64<E>(self, _value: i64) -> Result<Self::Value, E> {
        Ok(())
    }

    fn visit_u64<E>(self, _value: u64) -> Result<Self::Value, E> {
        Ok(())
    }

    fn visit_f64<E>(self, _value: f64) -> Result<Self::Value, E> {
        Ok(())
    }

    fn visit_str<E>(self, _value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(())
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(())
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(())
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        while sequence.next_element_seed(UniqueValue)?.is_some() {}
        Ok(())
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut keys = BTreeSet::new();
        while let Some(key) = map.next_key::<String>()? {
            if !keys.insert(key.clone()) {
                return Err(de::Error::custom(format!(
                    "duplicate configuration key {key:?}"
                )));
            }
            map.next_value_seed(UniqueValue)?;
        }
        Ok(())
    }
}

fn validate_route(
    name: &str,
    primary: &str,
    fallbacks: &[String],
    thinking: Option<&str>,
) -> Result<(), SkillEvalError> {
    let routes = route_keys(primary, fallbacks)?;
    if routes.len() != fallbacks.len() + 1 {
        return Err(invalid_configuration(format!(
            "route {name:?} contains a duplicate model"
        )));
    }
    if let Some(thinking) = thinking {
        validate_thinking(thinking)?;
    }
    Ok(())
}

fn route_keys(primary: &str, fallbacks: &[String]) -> Result<BTreeSet<ModelKey>, SkillEvalError> {
    std::iter::once(primary)
        .chain(fallbacks.iter().map(String::as_str))
        .map(parse_identifier)
        .collect()
}

fn validate_key(kind: &str, key: &str) -> Result<(), SkillEvalError> {
    let is_valid = !key.is_empty()
        && key.split('-').all(|segment| {
            !segment.is_empty()
                && segment
                    .chars()
                    .all(|value| value.is_ascii_alphanumeric() && !value.is_ascii_uppercase())
        });
    if is_valid {
        Ok(())
    } else {
        Err(invalid_configuration(format!(
            "{kind} key {key:?} is malformed"
        )))
    }
}

fn invalid_configuration(message: impl Into<String>) -> SkillEvalError {
    SkillEvalError::InvalidConfiguration(message.into())
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
        if is_catalog_alias(columns[0], columns[1]) {
            continue;
        }
        let key = catalog_key(columns[0], columns[1], index + 2)?;
        if !matches!(columns[4], "yes" | "no") {
            return Err(SkillEvalError::InvalidConfiguration(format!(
                "Pi model catalog line {} has malformed thinking metadata",
                index + 2
            )));
        }
        if catalog
            .insert(
                key,
                CatalogModel {
                    supported_thinking_levels: None,
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

pub(crate) fn validate_rpc_models_data(output: &str) -> Result<(), SkillEvalError> {
    parse_rpc_catalog(output).map(|_| ())
}

fn merge_rpc_capabilities(
    catalog: &mut BTreeMap<ModelKey, CatalogModel>,
    output: &str,
) -> Result<(), SkillEvalError> {
    for (key, levels) in parse_rpc_catalog(output)? {
        if let Some(model) = catalog.get_mut(&key) {
            model.supported_thinking_levels = Some(levels);
        }
    }
    Ok(())
}

fn parse_rpc_catalog(output: &str) -> Result<BTreeMap<ModelKey, BTreeSet<String>>, SkillEvalError> {
    let rpc_catalog: RpcCatalog = serde_json::from_str(output).map_err(|error| {
        invalid_configuration(format!("Pi RPC model capability data is invalid: {error}"))
    })?;
    let mut capabilities = BTreeMap::new();
    for model in rpc_catalog.models {
        let key = rpc_model_key(&model.provider, &model.id)?;
        let levels = supported_thinking_levels(&model)?;
        if capabilities.insert(key, levels).is_some() {
            return Err(invalid_configuration(format!(
                "Pi RPC model capability data duplicates {}/{}",
                model.provider, model.id
            )));
        }
    }
    Ok(capabilities)
}

fn rpc_model_key(provider: &str, model: &str) -> Result<ModelKey, SkillEvalError> {
    let (normalized_model, _) = normalize_rpc_model_id(model).ok_or_else(|| {
        invalid_configuration(format!(
            "Pi RPC model capability data has malformed model identity {provider}/{model}"
        ))
    })?;
    if provider.contains('/')
        || !is_exact_segment(provider)
        || !normalized_model.split('/').all(is_exact_segment)
    {
        return Err(invalid_configuration(format!(
            "Pi RPC model capability data has malformed model identity {provider}/{model}"
        )));
    }
    Ok(ModelKey {
        provider: provider.to_owned(),
        model: normalized_model.to_owned(),
    })
}

fn supported_thinking_levels(model: &RpcModel) -> Result<BTreeSet<String>, SkillEvalError> {
    derive_supported_thinking_levels(
        &model.provider,
        &model.id,
        model.is_reasoning_supported,
        model.thinking_level_map.as_ref(),
    )
    .map(|levels| levels.into_iter().collect())
}

pub(crate) fn derive_supported_thinking_levels(
    provider: &str,
    model: &str,
    is_reasoning_supported: bool,
    thinking_level_map: Option<&BTreeMap<String, Option<String>>>,
) -> Result<Vec<String>, SkillEvalError> {
    if let Some(level_map) = thinking_level_map {
        for level in level_map.keys() {
            validate_thinking(level).map_err(|_| {
                invalid_configuration(format!(
                    "Pi RPC model capability data for {provider}/{model} has unknown thinking level {level:?}"
                ))
            })?;
        }
    }

    if !is_reasoning_supported {
        return Ok(vec!["off".to_owned()]);
    }

    let mut supported = Vec::new();
    for level in ["off", "minimal", "low", "medium", "high"] {
        if !matches!(
            thinking_level_map.and_then(|level_map| level_map.get(level)),
            Some(None)
        ) {
            supported.push(level.to_owned());
        }
    }
    for level in ["xhigh", "max"] {
        if matches!(
            thinking_level_map.and_then(|level_map| level_map.get(level)),
            Some(Some(_))
        ) {
            supported.push(level.to_owned());
        }
    }
    Ok(supported)
}

fn is_catalog_alias(provider: &str, model: &str) -> bool {
    !provider.contains('/')
        && is_exact_segment(provider)
        && model
            .strip_prefix('~')
            .is_some_and(|alias| alias.contains('/') && alias.split('/').all(is_exact_segment))
}

fn catalog_key(provider: &str, model: &str, line: usize) -> Result<ModelKey, SkillEvalError> {
    if provider.contains('/')
        || !is_exact_segment(provider)
        || !model.split('/').all(is_exact_segment)
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
    if provider.contains('/')
        || !is_exact_segment(provider)
        || !model.split('/').all(is_exact_segment)
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
        && segment.chars().all(|character| {
            character.is_ascii_alphanumeric()
                || matches!(character, '-' | '.' | '_' | ':' | '+' | '@')
        })
}

pub(crate) fn normalize_rpc_model_id(model: &str) -> Option<(&str, bool)> {
    let (model, is_moving_alias) = match model.strip_prefix('~') {
        Some(model) => (model, true),
        None => (model, false),
    };
    (!model.is_empty() && !model.contains('~')).then_some((model, is_moving_alias))
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
) -> Result<ModelIdentity, SkillEvalError> {
    validate_supported_thinking(&key, catalog_model, &route.thinking)?;
    Ok(ModelIdentity {
        tier,
        provider: key.provider,
        model: key.model,
        thinking: route.thinking.clone(),
    })
}

fn validate_supported_thinking(
    key: &ModelKey,
    model: &CatalogModel,
    requested: &str,
) -> Result<(), SkillEvalError> {
    let Some(supported) = &model.supported_thinking_levels else {
        return Err(invalid_configuration(format!(
            "model {}/{} requests thinking level {requested:?}, but Pi RPC capability metadata is missing; supported levels: unavailable",
            key.provider, key.model
        )));
    };
    if supported.contains(requested) {
        return Ok(());
    }
    let levels = ["off", "minimal", "low", "medium", "high", "xhigh", "max"]
        .into_iter()
        .filter(|level| supported.contains(*level))
        .collect::<Vec<_>>()
        .join(", ");
    Err(invalid_configuration(format!(
        "model {}/{} requests unsupported thinking level {requested:?}; supported levels: [{levels}]",
        key.provider, key.model
    )))
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

fn thinking_rank(thinking: &str) -> u8 {
    match thinking {
        "off" => 0,
        "minimal" => 1,
        "low" => 2,
        "medium" => 3,
        "high" => 4,
        "xhigh" => 5,
        "max" => 6,
        _ => unreachable!("thinking is validated before ranking"),
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
fn test_rpc_output(catalog: &str) -> Result<String, SkillEvalError> {
    let mut models = Vec::new();
    for line in catalog
        .lines()
        .skip(1)
        .filter(|line| !line.trim().is_empty())
    {
        let columns = line.split_whitespace().collect::<Vec<_>>();
        if columns.len() != 6 || is_catalog_alias(columns[0], columns[1]) {
            continue;
        }
        models.push(serde_json::json!({
            "provider": columns[0],
            "id": columns[1],
            "reasoning": columns[4] == "yes"
        }));
    }
    serde_json::to_string(&serde_json::json!({"models": models})).map_err(|error| {
        invalid_configuration(format!("test Pi RPC fixture cannot be serialized: {error}"))
    })
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
    let result = ConfiguredModelResolver::from_text(
        include_str!("../tests/fixtures/models/routing-unknown-judge.json"),
        include_str!("../tests/fixtures/models/catalog-all.txt"),
    );

    assert!(matches!(
        result,
        Err(SkillEvalError::InvalidConfiguration(message)) if message.contains("T9")
    ));
}

#[cfg(test)]
#[test]
fn malformed_model_identifier_fails_without_guessing() {
    let result = ConfiguredModelResolver::from_text(
        include_str!("../tests/fixtures/models/routing-malformed-model.json"),
        include_str!("../tests/fixtures/models/catalog-all.txt"),
    );

    assert!(matches!(
        result,
        Err(SkillEvalError::InvalidConfiguration(message))
            if message.contains("provider/model")
    ));
}

#[cfg(test)]
#[test]
fn catalog_moving_alias_rows_are_skipped() {
    let configuration = r#"{
  "tiers": {
    "T1": {
      "pi": "anthropic/claude-fable-5",
      "fallbacks": [],
      "thinking": "medium"
    }
  },
  "judge": "T1"
}"#;
    let catalog = "provider model context max-out thinking images\nopenrouter ~anthropic/claude-fable-latest 1M 128K yes yes\nanthropic claude-fable-5 1M 128K yes yes\n";
    let resolver = resolver(configuration, catalog);

    let candidates = resolver
        .candidates(Tier::T1)
        .expect("exact route must resolve");

    assert_eq!(candidates[0].provider, "anthropic");
    assert_eq!(candidates[0].model, "claude-fable-5");
}

#[cfg(test)]
#[test]
fn malformed_non_alias_catalog_row_still_fails() {
    let configuration = r#"{
  "tiers": {
    "T1": {
      "pi": "anthropic/claude-fable-5",
      "fallbacks": [],
      "thinking": "medium"
    }
  },
  "judge": "T1"
}"#;
    let catalog =
        "provider model context max-out thinking images\nopenrouter bad~identity 1M 128K yes yes\n";

    let result = ConfiguredModelResolver::from_text(configuration, catalog);

    assert!(matches!(
        result,
        Err(SkillEvalError::InvalidConfiguration(message))
            if message.contains("catalog line 2") && message.contains("malformed")
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
