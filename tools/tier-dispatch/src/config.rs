//! Reads `config/model-tiers.json` — the repo's single source of truth for
//! tier-to-model mapping (see `docs/routing.md`, "Model ids live in ONE file"). This
//! module only resolves a tier name to its ordered chain of candidate models; it never
//! decides which one actually runs — that's `dispatch.rs`'s job.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ModelEntry {
    pub model: String,
    pub thinking: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Tier {
    pub pi: ModelEntry,
    #[serde(default)]
    pub fallbacks: Vec<ModelEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TiersFile {
    pub tiers: BTreeMap<String, Tier>,
}

impl TiersFile {
    pub fn load(path: &Path) -> Result<Self, String> {
        let raw = std::fs::read_to_string(path)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        serde_json::from_str(&raw).map_err(|error| format!("{}: {error}", path.display()))
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
