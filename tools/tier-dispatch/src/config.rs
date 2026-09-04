//! Reads `config/model-tiers.json` — the repo's single source of truth for
//! tier-to-model mapping (see `docs/routing.md`, "Model ids live in ONE file"). This
//! module only resolves a tier name to its ordered chain of candidate models; it never
//! decides which one actually runs — that's `dispatch.rs`'s job.

use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ModelEntry {
    pub model: String,
    pub thinking: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tier {
    pub pi: ModelEntry,
    #[serde(default)]
    pub fallbacks: Vec<ModelEntry>,
    #[serde(default, rename = "climbOnExhaustion")]
    pub climb_on_exhaustion: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TiersFile {
    pub tiers: BTreeMap<String, Tier>,
    #[serde(default)]
    pub orchestrator: Option<String>,
    #[serde(default)]
    pub agents: BTreeMap<String, String>,
    #[serde(default, rename = "untiered")]
    _untiered: BTreeMap<String, String>,
}

impl TiersFile {
    pub fn load(path: &Path) -> Result<Self, String> {
        let raw = std::fs::read_to_string(path)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        let tiers: Self =
            serde_json::from_str(&raw).map_err(|error| format!("{}: {error}", path.display()))?;
        if tiers.tiers.is_empty() {
            return Err(format!("{}: tiers must not be empty", path.display()));
        }
        if let Some(orchestrator) = &tiers.orchestrator
            && !tiers.tiers.contains_key(orchestrator)
        {
            return Err(format!(
                "{}: orchestrator names unknown tier {orchestrator}",
                path.display()
            ));
        }
        for (agent, tier) in &tiers.agents {
            if !tiers.tiers.contains_key(tier) {
                return Err(format!(
                    "{}: agent {agent} names unknown tier {tier}",
                    path.display()
                ));
            }
        }
        for (name, tier) in &tiers.tiers {
            for entry in std::iter::once(&tier.pi).chain(tier.fallbacks.iter()) {
                if !matches!(
                    entry.thinking.as_str(),
                    "off" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max"
                ) {
                    return Err(format!(
                        "{}: tier {name} has invalid thinking level {}",
                        path.display(),
                        entry.thinking
                    ));
                }
            }
        }
        for (name, tier) in &tiers.tiers {
            if let Some(target) = &tier.climb_on_exhaustion
                && (target == name || !tiers.tiers.contains_key(target))
            {
                return Err(format!(
                    "{}: tier {name} has invalid climbOnExhaustion {target}",
                    path.display()
                ));
            }
        }
        for start in tiers.tiers.keys() {
            let mut seen = BTreeSet::new();
            let mut current = start;
            while let Some(target) = tiers
                .tiers
                .get(current)
                .and_then(|tier| tier.climb_on_exhaustion.as_ref())
            {
                if !seen.insert(current) {
                    return Err(format!(
                        "{}: climbOnExhaustion cycle includes {current}",
                        path.display()
                    ));
                }
                current = target;
            }
        }
        Ok(tiers)
    }

    /// The ordered candidate chain for one tier: its own primary model first, then its
    /// own fallbacks, in the order `config/model-tiers.json` lists them. Never crosses
    /// into another tier's chain — `tools/tier-dispatch` reports the whole tier
    /// unavailable rather than silently substituting a different tier's model, per the
    /// plan's own decision.
    pub fn chain(&self, tier_name: &str) -> Result<Vec<ModelEntry>, String> {
        let tier = self.tiers.get(tier_name).ok_or_else(|| {
            format!(
                "unknown tier {tier_name:?}; known tiers: {:?}",
                self.tier_names()
            )
        })?;
        let mut chain = Vec::with_capacity(1 + tier.fallbacks.len());
        chain.push(tier.pi.clone());
        chain.extend(tier.fallbacks.iter().cloned());
        Ok(chain)
    }

    pub fn tier_names(&self) -> Vec<&str> {
        self.tiers.keys().map(String::as_str).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_fixture(dir: &Path) -> std::path::PathBuf {
        let path = dir.join("model-tiers.json");
        std::fs::write(
            &path,
            r#"{
              "tiers": {
                "T1": {
                  "pi": { "model": "openai-codex/gpt-5.6-luna", "thinking": "low" },
                  "fallbacks": [
                    { "model": "anthropic/claude-haiku-4-5", "thinking": "low" }
                  ]
                },
                "T5": {
                  "pi": { "model": "anthropic/claude-fable-5", "thinking": "medium" },
                  "fallbacks": []
                }
              }
            }"#,
        )
        .unwrap();
        path
    }

    #[test]
    fn chain_puts_primary_first_then_fallbacks_in_file_order() {
        let dir = std::env::temp_dir().join(format!("tier-dispatch-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = write_fixture(&dir);
        let tiers = TiersFile::load(&path).unwrap();
        let chain = tiers.chain("T1").unwrap();
        assert_eq!(chain.len(), 2);
        assert_eq!(chain[0].model, "openai-codex/gpt-5.6-luna");
        assert_eq!(chain[1].model, "anthropic/claude-haiku-4-5");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rejects_an_unknown_tier_field() {
        let dir = std::env::temp_dir().join(format!(
            "tier-dispatch-test-unknown-field-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("model-tiers.json");
        std::fs::write(
            &path,
            r#"{"tiers":{"T1":{"pi":{"model":"openai-codex/gpt-5.6-luna","thinking":"low"},"falbacks":[]}}}"#,
        )
        .unwrap();
        assert!(TiersFile::load(&path).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rejects_an_unknown_top_level_field() {
        let dir = std::env::temp_dir().join(format!(
            "tier-dispatch-test-top-level-field-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("model-tiers.json");
        std::fs::write(
            &path,
            r#"{"tiers":{"T1":{"pi":{"model":"openai-codex/gpt-5.6-luna","thinking":"low"}}},"orchestratorr":"T1"}"#,
        )
        .unwrap();
        assert!(TiersFile::load(&path).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rejects_an_unknown_climb_target() {
        let dir = std::env::temp_dir().join(format!(
            "tier-dispatch-test-unknown-climb-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("model-tiers.json");
        std::fs::write(
            &path,
            r#"{"tiers":{"T1":{"pi":{"model":"openai-codex/gpt-5.6-luna","thinking":"low"},"climbOnExhaustion":"T9"}}}"#,
        )
        .unwrap();
        assert!(TiersFile::load(&path).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rejects_a_climb_cycle() {
        let dir = std::env::temp_dir().join(format!(
            "tier-dispatch-test-climb-cycle-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("model-tiers.json");
        std::fs::write(
            &path,
            r#"{"tiers":{"T1":{"pi":{"model":"openai-codex/gpt-5.6-luna","thinking":"low"},"climbOnExhaustion":"T2"},"T2":{"pi":{"model":"anthropic/claude-haiku-4-5","thinking":"low"},"climbOnExhaustion":"T1"}}}"#,
        )
        .unwrap();
        assert!(TiersFile::load(&path).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rejects_unknown_orchestrator_and_agent_tiers() {
        let dir = std::env::temp_dir().join(format!(
            "tier-dispatch-test-tier-references-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("model-tiers.json");
        std::fs::write(
            &path,
            r#"{"tiers":{"T1":{"pi":{"model":"openai-codex/gpt-5.6-luna","thinking":"low"}}},"orchestrator":"T9","agents":{"reviewer":"T8"}}"#,
        )
        .unwrap();
        assert!(TiersFile::load(&path).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rejects_an_unknown_thinking_level() {
        let dir = std::env::temp_dir().join(format!(
            "tier-dispatch-test-thinking-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("model-tiers.json");
        std::fs::write(
            &path,
            r#"{"tiers":{"T1":{"pi":{"model":"openai-codex/gpt-5.6-luna","thinking":"maximum"}}}}"#,
        )
        .unwrap();
        assert!(TiersFile::load(&path).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn chain_with_no_fallbacks_is_just_the_primary() {
        let dir =
            std::env::temp_dir().join(format!("tier-dispatch-test-{}", std::process::id() + 1));
        std::fs::create_dir_all(&dir).unwrap();
        let path = write_fixture(&dir);
        let tiers = TiersFile::load(&path).unwrap();
        let chain = tiers.chain("T5").unwrap();
        assert_eq!(
            chain,
            vec![ModelEntry {
                model: "anthropic/claude-fable-5".into(),
                thinking: "medium".into()
            }]
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn unknown_tier_is_an_error_naming_the_known_tiers() {
        let dir =
            std::env::temp_dir().join(format!("tier-dispatch-test-{}", std::process::id() + 2));
        std::fs::create_dir_all(&dir).unwrap();
        let path = write_fixture(&dir);
        let tiers = TiersFile::load(&path).unwrap();
        let error = tiers.chain("T99").unwrap_err();
        assert!(
            error.contains("T99"),
            "error should name the bad tier: {error}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
