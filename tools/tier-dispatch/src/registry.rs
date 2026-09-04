use crate::config::TiersFile;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub tier: String,
    pub slot: String,
    pub model: String,
}

#[derive(Debug, Clone, Default)]
pub struct Registry {
    models: BTreeMap<String, BTreeSet<String>>,
}

#[derive(Debug, Clone, Default)]
pub struct ModelOverrides {
    models: BTreeMap<String, BTreeSet<String>>,
}

impl Registry {
    pub fn load(path: &Path) -> Result<Self, String> {
        let raw = std::fs::read_to_string(path)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        let root: serde_json::Value =
            serde_json::from_str(&raw).map_err(|error| format!("{}: {error}", path.display()))?;
        let providers = root
            .as_object()
            .ok_or_else(|| format!("{}: registry root must be an object", path.display()))?;
        let models = providers
            .iter()
            .filter_map(|(provider, value)| {
                provider_models(value).map(|models| (provider.clone(), models))
            })
            .collect();
        Ok(Self { models })
    }

    fn contains(&self, provider: &str, model: &str) -> bool {
        self.models
            .get(provider)
            .is_some_and(|models| models.contains(model))
    }

    fn model_ids(&self) -> impl Iterator<Item = (&str, &str)> {
        self.models.iter().flat_map(|(provider, models)| {
            models
                .iter()
                .map(move |model| (provider.as_str(), model.as_str()))
        })
    }
}

impl ModelOverrides {
    pub fn load(path: &Path) -> Result<Self, String> {
        let raw = std::fs::read_to_string(path)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        let root: serde_json::Value =
            serde_json::from_str(&raw).map_err(|error| format!("{}: {error}", path.display()))?;
        let providers = root
            .get("providers")
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| format!("{}: providers must be an object", path.display()))?;
        let models = providers
            .iter()
            .filter_map(|(provider, value)| {
                let overrides = value.get("modelOverrides")?.as_object()?;
                Some((provider.clone(), overrides.keys().cloned().collect()))
            })
            .collect();
        Ok(Self { models })
    }

    fn model_ids(&self) -> impl Iterator<Item = (&str, &str)> {
        self.models.iter().flat_map(|(provider, models)| {
            models
                .iter()
                .map(move |model| (provider.as_str(), model.as_str()))
        })
    }
}

pub fn unknown_models(tiers: &TiersFile, registry: &Registry) -> Vec<Finding> {
    let mut findings = Vec::new();
    for (tier_name, tier) in &tiers.tiers {
        let entries = std::iter::once((&tier.pi, "pi".to_string())).chain(
            tier.fallbacks
                .iter()
                .enumerate()
                .map(|(index, entry)| (entry, format!("fallbacks[{index}]"))),
        );
        for (entry, slot) in entries {
            let Some((provider, model)) = entry.model.split_once('/') else {
                findings.push(Finding {
                    tier: tier_name.clone(),
                    slot,
                    model: entry.model.clone(),
                });
                continue;
            };
            if !registry.contains(provider, model) {
                findings.push(Finding {
                    tier: tier_name.clone(),
                    slot,
                    model: entry.model.clone(),
                });
            }
        }
    }
    findings
}

pub fn unknown_model_overrides(overrides: &ModelOverrides, registry: &Registry) -> Vec<String> {
    overrides
        .model_ids()
        .filter(|(provider, model)| !registry.contains(provider, model))
        .map(|(provider, model)| format!("{provider}/{model}"))
        .collect()
}

pub fn unreferenced_newer(tiers: &TiersFile, registry: &Registry) -> Vec<String> {
    let tiered = tiers
        .tiers
        .values()
        .flat_map(|tier| std::iter::once(&tier.pi).chain(tier.fallbacks.iter()))
        .filter_map(|entry| entry.model.split_once('/'))
        .collect::<BTreeSet<_>>();

    registry
        .model_ids()
        .filter(|(provider, model)| {
            let candidate = (*provider, *model);
            !tiered.contains(&candidate)
                && is_newer_model(
                    model,
                    tiered.iter().filter_map(|(tiered_provider, tiered_model)| {
                        (provider == tiered_provider).then_some(*tiered_model)
                    }),
                )
        })
        .map(|(provider, model)| format!("{provider}/{model}"))
        .collect()
}

fn provider_models(value: &serde_json::Value) -> Option<BTreeSet<String>> {
    let models = value.as_object()?.get("models")?.as_array()?;
    Some(
        models
            .iter()
            .filter_map(|model| model.get("id")?.as_str().map(str::to_string))
            .collect(),
    )
}

fn is_newer_model<'a>(candidate: &str, tiered: impl Iterator<Item = &'a str>) -> bool {
    let candidate_family = model_family(candidate);
    let candidate_variant = model_variant(candidate);
    let candidate_version = version(candidate);
    let comparable = tiered
        .filter(|model| model_family(model) == candidate_family)
        .map(|model| (model_variant(model), version(model)))
        .collect::<Vec<_>>();
    let is_new_major = comparable
        .iter()
        .filter_map(|(_, version)| version.first())
        .max()
        .zip(candidate_version.first())
        .is_some_and(|(latest, candidate)| candidate > latest);
    candidate_variant != "mini"
        && (is_new_major
            || comparable
                .iter()
                .map(|(_, version)| version)
                .max()
                .is_some_and(|latest| &candidate_version > latest))
}

fn model_family(model: &str) -> String {
    without_snapshot_date(model)
        .split(|character: char| character.is_ascii_digit())
        .next()
        .unwrap_or(model)
        .trim_matches(|character: char| !character.is_ascii_alphabetic())
        .to_owned()
}

fn model_variant(model: &str) -> String {
    without_snapshot_date(model)
        .rsplit(|character: char| character.is_ascii_digit())
        .next()
        .unwrap_or_default()
        .trim_matches(|character: char| !character.is_ascii_alphabetic())
        .to_owned()
}

fn version(model: &str) -> Vec<u64> {
    without_snapshot_date(model)
        .split(|character: char| !character.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse().ok())
        .collect()
}

fn without_snapshot_date(model: &str) -> &str {
    let Some((prefix, last)) = model.rsplit_once('-') else {
        return model;
    };
    if last.len() == 8 && last.bytes().all(|byte| byte.is_ascii_digit()) {
        return prefix;
    }
    let Some((prefix, month)) = prefix.rsplit_once('-') else {
        return model;
    };
    let Some((prefix, year)) = prefix.rsplit_once('-') else {
        return model;
    };
    if year.len() == 4
        && month.len() == 2
        && last.len() == 2
        && [year, month, last]
            .iter()
            .all(|part| part.bytes().all(|byte| byte.is_ascii_digit()))
    {
        prefix
    } else {
        model
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_dir(name: &str) -> std::path::PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "tier-dispatch-registry-{name}-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    fn write_tiers(directory: &Path, model: &str) -> std::path::PathBuf {
        let path = directory.join("model-tiers.json");
        std::fs::write(
            &path,
            format!(
                r#"{{"tiers":{{"T4":{{"pi":{{"model":"anthropic/claude-fable-5","thinking":"medium"}},"fallbacks":[{{"model":"openai-codex/gpt-5.6-sol","thinking":"low"}},{{"model":"{model}","thinking":"low"}}]}}}}}}"#
            ),
        )
        .unwrap();
        path
    }

    fn write_registry(directory: &Path, body: &str) -> std::path::PathBuf {
        let path = directory.join("models-store.json");
        std::fs::write(&path, body).unwrap();
        path
    }

    fn write_overrides(directory: &Path, body: &str) -> std::path::PathBuf {
        let path = directory.join("models.json");
        std::fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn all_tier_models_present_yields_no_findings() {
        let directory = test_dir("all-present");
        let tiers =
            TiersFile::load(&write_tiers(&directory, "anthropic/claude-haiku-4-5")).unwrap();
        let registry = Registry::load(&write_registry(&directory, r#"{"anthropic":{"models":[{"id":"claude-fable-5"},{"id":"claude-haiku-4-5"}]},"openai-codex":{"models":[{"id":"gpt-5.6-sol"}]}}"#)).unwrap();
        assert!(unknown_models(&tiers, &registry).is_empty());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn missing_id_is_reported_with_tier_and_slot() {
        let directory = test_dir("missing-id");
        let tiers =
            TiersFile::load(&write_tiers(&directory, "anthropic/claude-does-not-exist")).unwrap();
        let registry = Registry::load(&write_registry(&directory, r#"{"anthropic":{"models":[{"id":"claude-fable-5"}]},"openai-codex":{"models":[{"id":"gpt-5.6-sol"}]}}"#)).unwrap();
        assert_eq!(
            unknown_models(&tiers, &registry),
            vec![Finding {
                tier: "T4".into(),
                slot: "fallbacks[1]".into(),
                model: "anthropic/claude-does-not-exist".into()
            }]
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn right_id_wrong_provider_is_reported() {
        let directory = test_dir("wrong-provider");
        let tiers = TiersFile::load(&write_tiers(&directory, "openai/gpt-5.6-sol")).unwrap();
        let registry = Registry::load(&write_registry(&directory, r#"{"anthropic":{"models":[{"id":"claude-fable-5"}]},"openai-codex":{"models":[{"id":"gpt-5.6-sol"}]}}"#)).unwrap();
        assert_eq!(
            unknown_models(&tiers, &registry)[0].model,
            "openai/gpt-5.6-sol"
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn unknown_override_is_reported() {
        let directory = test_dir("unknown-override");
        let registry = Registry::load(&write_registry(
            &directory,
            r#"{"anthropic":{"models":[{"id":"claude-fable-5"}]}}"#,
        ))
        .unwrap();
        let overrides = ModelOverrides::load(&write_overrides(
            &directory,
            r#"{"providers":{"anthropic":{"modelOverrides":{"claude-fable-5":{},"claude-does-not-exist":{}}}}}"#,
        ))
        .unwrap();
        assert_eq!(
            unknown_model_overrides(&overrides, &registry),
            vec!["anthropic/claude-does-not-exist"]
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn registry_with_unknown_provider_block_still_parses() {
        let directory = test_dir("unknown-provider");
        let registry = Registry::load(&write_registry(&directory, r#"{"checkedAt":1,"etag":"x","someprovider":{"models":[{"id":"new-model-1"}]},"anthropic":{"models":[{"id":"claude-fable-5"}]}}"#)).unwrap();
        assert!(registry.contains("anthropic", "claude-fable-5"));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn malformed_registry_json_is_an_error_naming_the_path() {
        let directory = test_dir("malformed");
        let path = write_registry(&directory, "{");
        let error = Registry::load(&path).unwrap_err();
        assert!(error.contains(&path.display().to_string()));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn missing_registry_file_is_an_error_not_a_pass() {
        let directory = test_dir("missing-file");
        assert!(Registry::load(&directory.join("missing.json")).is_err());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn unreferenced_newer_lists_the_new_family_member_only() {
        let directory = test_dir("newer");
        let tiers = TiersFile::load(&write_tiers(&directory, "anthropic/claude-fable-5")).unwrap();
        let registry = Registry::load(&write_registry(&directory, r#"{"anthropic":{"models":[{"id":"claude-fable-5"},{"id":"claude-fable-5-1"}]},"openai-codex":{"models":[{"id":"gpt-5.6-sol"}]}}"#)).unwrap();
        assert_eq!(
            unreferenced_newer(&tiers, &registry),
            vec!["anthropic/claude-fable-5-1"]
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn unreferenced_newer_ignores_older_versions_and_dated_snapshots() {
        let directory = test_dir("not-newer");
        let tiers =
            TiersFile::load(&write_tiers(&directory, "anthropic/claude-haiku-4-5")).unwrap();
        let registry = Registry::load(&write_registry(&directory, r#"{"anthropic":{"models":[{"id":"claude-fable-5"},{"id":"claude-haiku-4-5"},{"id":"claude-haiku-4-5-20251001"}]},"openai-codex":{"models":[{"id":"gpt-5.6-sol"},{"id":"gpt-5.6-sol-2026-03-01"},{"id":"gpt-5.4"},{"id":"gpt-5.5"},{"id":"gpt-5.7-mini"},{"id":"gpt-6-mini"}]}}"#)).unwrap();
        assert!(unreferenced_newer(&tiers, &registry).is_empty());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn unreferenced_newer_includes_a_new_minor_variant() {
        let directory = test_dir("new-minor-variant");
        let tiers =
            TiersFile::load(&write_tiers(&directory, "anthropic/claude-haiku-4-5")).unwrap();
        let registry = Registry::load(&write_registry(&directory, r#"{"anthropic":{"models":[{"id":"claude-fable-5"},{"id":"claude-haiku-4-5"}]},"openai-codex":{"models":[{"id":"gpt-5.6-sol"},{"id":"gpt-5.7-nova"}]}}"#)).unwrap();
        assert_eq!(
            unreferenced_newer(&tiers, &registry),
            vec!["openai-codex/gpt-5.7-nova"]
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn unreferenced_newer_includes_a_new_major_with_a_new_variant() {
        let directory = test_dir("new-major");
        let tiers =
            TiersFile::load(&write_tiers(&directory, "anthropic/claude-haiku-4-5")).unwrap();
        let registry = Registry::load(&write_registry(&directory, r#"{"anthropic":{"models":[{"id":"claude-fable-5"},{"id":"claude-haiku-4-5"}]},"openai-codex":{"models":[{"id":"gpt-5.6-sol"},{"id":"gpt-6-astra"}]}}"#)).unwrap();
        assert_eq!(
            unreferenced_newer(&tiers, &registry),
            vec!["openai-codex/gpt-6-astra"]
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn unreferenced_newer_is_empty_when_every_family_member_is_tiered() {
        let directory = test_dir("all-tiered");
        let tiers =
            TiersFile::load(&write_tiers(&directory, "anthropic/claude-fable-5-1")).unwrap();
        let registry = Registry::load(&write_registry(&directory, r#"{"anthropic":{"models":[{"id":"claude-fable-5"},{"id":"claude-fable-5-1"}]},"openai-codex":{"models":[{"id":"gpt-5.6-sol"}]}}"#)).unwrap();
        assert!(unreferenced_newer(&tiers, &registry).is_empty());
        std::fs::remove_dir_all(directory).unwrap();
    }
}
