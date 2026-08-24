#[path = "../src/model.rs"]
mod model;
#[path = "../src/ports.rs"]
mod ports;

mod models {
    include!("../src/models.rs");

    // TODO(AGNT-0032.T102): Prove pool judge selection across self, fallback, control, and host routes.
    mod exact_candidate {
        use crate::model::{ModelIdentity, SkillEvalError, Tier};
        use crate::ports::ModelResolver;

        use super::resolver;

        const CATALOG_HEADER: &str = "provider model context max-out thinking images\n";

        fn configuration(controls: &str) -> String {
            format!(
                r#"{{
  "tiers": {{
    "T3": {{
      "pi": "first-party/configured-primary",
      "fallbacks": ["fallback/configured-fallback"],
      "thinking": "medium"
    }}
  }},
  "judge": "T3",
  "unranked_controls": {controls}
}}"#
            )
        }

        fn requested(provider: &str, model: &str, thinking: &str) -> ModelIdentity {
            ModelIdentity {
                tier: Tier::T3,
                provider: provider.to_owned(),
                model: model.to_owned(),
                thinking: thinking.to_owned(),
            }
        }

        #[test]
        fn first_party_and_openrouter_copies_remain_distinct() {
            let catalog = format!(
                "{CATALOG_HEADER}anthropic claude-sonnet-4-5 200K 64K yes yes\nopenrouter anthropic/claude-sonnet-4-5 200K 64K yes yes\n"
            );
            let resolver = resolver(&configuration("{}"), &catalog);
            let first_party = requested("anthropic", "claude-sonnet-4-5", "high");
            let openrouter = requested("openrouter", "anthropic/claude-sonnet-4-5", "high");

            assert_eq!(resolver.exact_candidate(&first_party), Ok(first_party));
            assert_eq!(resolver.exact_candidate(&openrouter), Ok(openrouter));
        }

        #[test]
        fn missing_exact_route_does_not_use_configured_fallback() {
            let catalog = format!(
                "{CATALOG_HEADER}first-party configured-primary 200K 64K yes yes\nfallback configured-fallback 200K 64K yes yes\n"
            );
            let resolver = resolver(&configuration("{}"), &catalog);
            let missing = requested("anthropic", "requested-exact-version", "medium");

            assert!(matches!(
                resolver.exact_candidate(&missing),
                Err(SkillEvalError::InvalidConfiguration(message))
                    if message.contains("requested-exact-version")
                        && !message.contains("configured-fallback")
            ));
        }

        #[test]
        fn openrouter_copy_cannot_replace_requested_first_party_route() {
            let catalog = format!(
                "{CATALOG_HEADER}openrouter anthropic/claude-sonnet-4-5 200K 64K yes yes\n"
            );
            let resolver = resolver(&configuration("{}"), &catalog);
            let first_party = requested("anthropic", "claude-sonnet-4-5", "high");

            assert!(matches!(
                resolver.exact_candidate(&first_party),
                Err(SkillEvalError::InvalidConfiguration(message))
                    if message.contains("not available")
            ));
        }

        #[test]
        fn unranked_control_is_rejected_even_when_catalogued() {
            let controls = r#"{
    "free": {
      "pi": "openrouter/openrouter/free",
      "maximum_tier": "T1",
      "is_read_only": true
    }
  }"#;
            let catalog = format!("{CATALOG_HEADER}openrouter openrouter/free 200K 4K yes yes\n");
            let resolver = resolver(&configuration(controls), &catalog);
            let control = requested("openrouter", "openrouter/free", "low");

            assert!(matches!(
                resolver.exact_candidate(&control),
                Err(SkillEvalError::InvalidConfiguration(message))
                    if message.contains("control")
            ));
        }

        #[test]
        fn unsupported_thinking_is_rejected_instead_of_downgraded() {
            let catalog = format!("{CATALOG_HEADER}anthropic exact-non-thinking 200K 64K no yes\n");
            let resolver = resolver(&configuration("{}"), &catalog);
            let unsupported = requested("anthropic", "exact-non-thinking", "low");
            let off = requested("anthropic", "exact-non-thinking", "off");

            assert!(matches!(
                resolver.exact_candidate(&unsupported),
                Err(SkillEvalError::InvalidConfiguration(message))
                    if message.contains("does not support")
            ));
            assert_eq!(resolver.exact_candidate(&off), Ok(off));
        }

        #[test]
        fn explicit_latest_alias_is_rejected_without_rejecting_exact_names() {
            let catalog = format!(
                "{CATALOG_HEADER}anthropic claude-sonnet-latest 200K 64K yes yes\nanthropic claude-latest-preview-20250801 200K 64K yes yes\n"
            );
            let resolver = resolver(&configuration("{}"), &catalog);
            let alias = requested("anthropic", "claude-sonnet-latest", "medium");
            let exact = requested("anthropic", "claude-latest-preview-20250801", "medium");

            assert!(matches!(
                resolver.exact_candidate(&alias),
                Err(SkillEvalError::InvalidConfiguration(message))
                    if message.contains("moving alias")
            ));
            assert_eq!(resolver.exact_candidate(&exact), Ok(exact));
        }

        #[test]
        fn malformed_exact_route_is_rejected() {
            let catalog = format!("{CATALOG_HEADER}anthropic exact-model 200K 64K yes yes\n");
            let resolver = resolver(&configuration("{}"), &catalog);

            for malformed in [
                requested(".", "exact-model", "medium"),
                requested("anthropic", "../exact-model", "medium"),
                requested("anthropic", "exact\u{7}model", "medium"),
            ] {
                assert!(matches!(
                    resolver.exact_candidate(&malformed),
                    Err(SkillEvalError::InvalidConfiguration(message))
                        if message.contains("malformed")
                ));
            }
        }
    }
}
