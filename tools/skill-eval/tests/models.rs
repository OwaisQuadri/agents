#[path = "../src/model.rs"]
mod model;
#[path = "../src/ports.rs"]
mod ports;

mod models {
    include!("../src/models.rs");

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
                    if message.contains("unsupported thinking")
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

    mod thinking_capabilities {
        use crate::model::{ModelIdentity, SkillEvalError, Tier};
        use crate::ports::ModelResolver;

        use super::ConfiguredModelResolver;

        const CATALOG: &str =
            "provider model context max-out thinking images\nprovider exact 200K 64K yes yes\n";

        fn configuration(thinking: &str, qualification_routes: &str) -> String {
            format!(
                r#"{{
  "tiers": {{
    "T1": {{"pi":"provider/exact","fallbacks":[],"thinking":"{thinking}"}}
  }},
  "qualification_routes": {qualification_routes},
  "judge": "T1"
}}"#
            )
        }

        fn rpc_model(reasoning: bool, level_map: &str) -> String {
            let level_map = if level_map.is_empty() {
                String::new()
            } else {
                format!(r#","thinkingLevelMap":{level_map}"#)
            };
            format!(
                r#"{{"models":[{{"provider":"provider","id":"exact","reasoning":{reasoning}{level_map}}}]}}"#
            )
        }

        fn resolver(reasoning: bool, level_map: &str) -> ConfiguredModelResolver {
            ConfiguredModelResolver::from_text_with_rpc(
                &configuration("off", "{}"),
                CATALOG,
                &rpc_model(reasoning, level_map),
            )
            .expect("capability fixture must load")
        }

        fn requested(level: &str) -> ModelIdentity {
            ModelIdentity {
                tier: Tier::T1,
                provider: "provider".to_owned(),
                model: "exact".to_owned(),
                thinking: level.to_owned(),
            }
        }

        #[test]
        fn non_reasoning_models_support_only_off() {
            let resolver = resolver(false, r#"{"off":null,"high":"high","max":"max"}"#);

            assert_eq!(
                resolver.exact_candidate(&requested("off")),
                Ok(requested("off"))
            );
            for level in ["minimal", "low", "medium", "high", "xhigh", "max"] {
                assert!(matches!(
                    resolver.exact_candidate(&requested(level)),
                    Err(SkillEvalError::InvalidConfiguration(message))
                        if message.contains("provider/exact")
                            && message.contains(level)
                            && message.contains("supported levels: [off]")
                ));
            }
        }

        #[test]
        fn default_reasoning_supports_standard_levels_only() {
            let resolver = resolver(true, "");

            for level in ["off", "minimal", "low", "medium", "high"] {
                assert_eq!(
                    resolver.exact_candidate(&requested(level)),
                    Ok(requested(level))
                );
            }
            for level in ["xhigh", "max"] {
                assert!(resolver.exact_candidate(&requested(level)).is_err());
            }
        }

        #[test]
        fn explicit_maps_preserve_off_high_xhigh_holes() {
            let resolver = resolver(
                true,
                r#"{"off":"none","minimal":null,"low":null,"medium":null,"high":"high","xhigh":"xhigh","max":null}"#,
            );

            for level in ["off", "high", "xhigh"] {
                assert_eq!(
                    resolver.exact_candidate(&requested(level)),
                    Ok(requested(level))
                );
            }
            for level in ["minimal", "low", "medium", "max"] {
                assert!(resolver.exact_candidate(&requested(level)).is_err());
            }
        }

        #[test]
        fn xhigh_and_max_require_explicit_non_null_entries() {
            for (level_map, supported) in [
                (r#"{"xhigh":null,"max":null}"#, Vec::<&str>::new()),
                (r#"{"xhigh":"xhigh"}"#, vec!["xhigh"]),
                (r#"{"max":"max"}"#, vec!["max"]),
                (r#"{"xhigh":"xhigh","max":"max"}"#, vec!["xhigh", "max"]),
            ] {
                let resolver = resolver(true, level_map);
                for level in ["xhigh", "max"] {
                    assert_eq!(
                        resolver.exact_candidate(&requested(level)).is_ok(),
                        supported.contains(&level),
                        "map {level_map} level {level}"
                    );
                }
            }
        }

        #[test]
        fn unsupported_tier_route_is_rejected_before_launch() {
            let resolver = ConfiguredModelResolver::from_text_with_rpc(
                &configuration("medium", "{}"),
                CATALOG,
                &rpc_model(true, r#"{"medium":null}"#),
            )
            .expect("configuration must load before route selection");

            assert!(matches!(
                resolver.candidates(Tier::T1),
                Err(SkillEvalError::InvalidConfiguration(message))
                    if message.contains("provider/exact")
                        && message.contains("medium")
                        && message.contains("supported levels")
            ));
        }

        #[test]
        fn exact_qualification_route_without_rpc_metadata_fails_closed() {
            let routes = r#"{"T1":[{"provider":"provider","model":"exact","thinking":"off"}]}"#;
            let rpc = r#"{"models":[{"provider":"other","id":"model","reasoning":false}]}"#;

            let result = ConfiguredModelResolver::from_text_with_rpc(
                &configuration("off", routes),
                CATALOG,
                rpc,
            );

            assert!(matches!(
                result,
                Err(SkillEvalError::InvalidConfiguration(message))
                    if message.contains("provider/exact")
                        && message.contains("off")
                        && message.contains("metadata is missing")
            ));
        }
    }

    mod qualification_routes {
        use crate::model::{ModelIdentity, SkillEvalError, Tier};
        use crate::ports::ModelResolver;

        use super::{ConfiguredModelResolver, resolver};

        const CATALOG: &str = "provider model context max-out thinking images\nanthropic claude-haiku-4-5 200K 64K yes yes\nopenai-codex luna 200K 64K yes yes\nplain fixed 200K 64K no yes\ncontrol free 200K 64K no yes\n";

        fn configuration(routes: &str) -> String {
            format!(
                r#"{{
  "tiers": {{
    "T1": {{"pi":"openai-codex/luna","fallbacks":[],"thinking":"low"}},
    "T2": {{"pi":"openai-codex/luna","fallbacks":[],"thinking":"low"}}
  }},
  "qualification_routes": {routes},
  "judge": "T2",
  "unranked_controls": {{
    "free": {{"pi":"control/free","maximum_tier":"T1","is_read_only":true}}
  }}
}}"#
            )
        }

        #[test]
        fn same_exact_model_has_a_tier_specific_minimum_thinking_level() {
            let configuration = configuration(
                r#"{
    "T1": [
      {"provider":"anthropic","model":"claude-haiku-4-5","thinking":"off"},
      {"provider":"anthropic","model":"claude-haiku-4-5","thinking":"low"}
    ],
    "T2": [
      {"provider":"anthropic","model":"claude-haiku-4-5","thinking":"medium"},
      {"provider":"anthropic","model":"claude-haiku-4-5","thinking":"high"}
    ]
  }"#,
            );
            let resolver = resolver(&configuration, CATALOG);

            assert_eq!(
                resolver.qualification_routes(Tier::T1).unwrap(),
                vec![
                    ModelIdentity {
                        tier: Tier::T1,
                        provider: "anthropic".to_owned(),
                        model: "claude-haiku-4-5".to_owned(),
                        thinking: "off".to_owned(),
                    },
                    ModelIdentity {
                        tier: Tier::T1,
                        provider: "anthropic".to_owned(),
                        model: "claude-haiku-4-5".to_owned(),
                        thinking: "low".to_owned(),
                    },
                ]
            );
            assert_eq!(
                resolver.qualification_routes(Tier::T2).unwrap()[0].thinking,
                "medium"
            );
        }

        #[test]
        fn same_model_requires_a_stronger_nonoverlapping_range_in_each_higher_tier() {
            for routes in [
                r#"{
                  "T1":[{"provider":"anthropic","model":"claude-haiku-4-5","thinking":"medium"}],
                  "T2":[{"provider":"anthropic","model":"claude-haiku-4-5","thinking":"medium"}]
                }"#,
                r#"{
                  "T1":[
                    {"provider":"anthropic","model":"claude-haiku-4-5","thinking":"off"},
                    {"provider":"anthropic","model":"claude-haiku-4-5","thinking":"medium"}
                  ],
                  "T2":[{"provider":"anthropic","model":"claude-haiku-4-5","thinking":"low"}]
                }"#,
            ] {
                let result = ConfiguredModelResolver::from_text(&configuration(routes), CATALOG);
                assert!(matches!(
                    result,
                    Err(SkillEvalError::InvalidConfiguration(message))
                        if message.contains("strictly stronger thinking")
                ));
            }
        }

        #[test]
        fn absent_qualification_route_order_fails_closed() {
            let resolver = resolver(&configuration("{}"), CATALOG);

            assert!(matches!(
                resolver.qualification_routes(Tier::T1),
                Err(SkillEvalError::InvalidConfiguration(message))
                    if message.contains("absent")
            ));
        }

        #[test]
        fn malformed_qualification_routes_are_rejected() {
            let duplicate = configuration(
                r#"{"T1":[
                  {"provider":"anthropic","model":"claude-haiku-4-5","thinking":"off"},
                  {"provider":"anthropic","model":"claude-haiku-4-5","thinking":"off"}
                ]}"#,
            );
            let descending = configuration(
                r#"{"T1":[
                  {"provider":"anthropic","model":"claude-haiku-4-5","thinking":"high"},
                  {"provider":"anthropic","model":"claude-haiku-4-5","thinking":"medium"}
                ]}"#,
            );
            let alias = configuration(
                r#"{"T1":[{"provider":"anthropic","model":"claude-haiku-latest","thinking":"off"}]}"#,
            );
            let control =
                configuration(r#"{"T1":[{"provider":"control","model":"free","thinking":"off"}]}"#);
            let unavailable = configuration(
                r#"{"T1":[{"provider":"anthropic","model":"missing","thinking":"off"}]}"#,
            );
            let unsupported = configuration(
                r#"{"T1":[{"provider":"plain","model":"fixed","thinking":"medium"}]}"#,
            );

            for (configuration, expected) in [
                (duplicate, "duplicate exact route"),
                (descending, "thinking order"),
                (alias, "moving alias"),
                (control, "control"),
                (unavailable, "not available"),
                (unsupported, "unsupported thinking"),
            ] {
                let result = ConfiguredModelResolver::from_text(&configuration, CATALOG);
                assert!(
                    matches!(result, Err(SkillEvalError::InvalidConfiguration(ref message)) if message.contains(expected)),
                    "expected {expected}, got {result:?}"
                );
            }
        }
    }

    mod pool_judge {
        use crate::model::{ModelIdentity, SkillEvalError, Tier};
        use crate::ports::ModelResolver;

        use super::{ConfiguredModelResolver, resolver};

        const CATALOG_HEADER: &str = "provider model context max-out thinking images\n";

        fn configuration(
            judge: &str,
            primary: &str,
            fallbacks: &[&str],
            thinking: &str,
            controls: &str,
        ) -> String {
            let fallbacks = serde_json::to_string(fallbacks).expect("fallbacks must serialize");
            format!(
                r#"{{
  "tiers": {{
    "{judge}": {{
      "pi": "{primary}",
      "fallbacks": {fallbacks},
      "thinking": "{thinking}"
    }}
  }},
  "judge": "{judge}",
  "unranked_controls": {controls}
}}"#
            )
        }

        fn candidate(tier: Tier, provider: &str, model: &str) -> ModelIdentity {
            ModelIdentity {
                tier,
                provider: provider.to_owned(),
                model: model.to_owned(),
                thinking: "medium".to_owned(),
            }
        }

        fn catalog(routes: &[(&str, &str)]) -> String {
            let mut output = CATALOG_HEADER.to_owned();
            for (provider, model) in routes {
                output.push_str(&format!("{provider} {model} 200K 64K yes yes\n"));
            }
            output
        }

        #[test]
        fn lower_candidate_uses_higher_configured_judge() {
            let configuration = configuration("T4", "anthropic/judge", &[], "high", "{}");
            let resolver = resolver(&configuration, &catalog(&[("anthropic", "judge")]));

            assert_eq!(
                resolver.pool_judge(&candidate(Tier::T3, "openai-codex", "candidate")),
                Ok(ModelIdentity {
                    tier: Tier::T4,
                    provider: "anthropic".to_owned(),
                    model: "judge".to_owned(),
                    thinking: "high".to_owned(),
                })
            );
        }

        #[test]
        fn t5_candidate_uses_distinct_primary() {
            let configuration = configuration("T5", "anthropic/judge", &[], "high", "{}");
            let resolver = resolver(&configuration, &catalog(&[("anthropic", "judge")]));

            assert_eq!(
                resolver.pool_judge(&candidate(Tier::T5, "openai-codex", "candidate")),
                Ok(ModelIdentity {
                    tier: Tier::T5,
                    provider: "anthropic".to_owned(),
                    model: "judge".to_owned(),
                    thinking: "high".to_owned(),
                })
            );
        }

        #[test]
        fn self_primary_is_skipped_for_distinct_fallback() {
            let configuration = configuration(
                "T5",
                "anthropic/candidate",
                &["openai-codex/fallback"],
                "medium",
                "{}",
            );
            let resolver = resolver(
                &configuration,
                &catalog(&[("anthropic", "candidate"), ("openai-codex", "fallback")]),
            );

            assert_eq!(
                resolver.pool_judge(&candidate(Tier::T5, "anthropic", "candidate")),
                Ok(ModelIdentity {
                    tier: Tier::T5,
                    provider: "openai-codex".to_owned(),
                    model: "fallback".to_owned(),
                    thinking: "medium".to_owned(),
                })
            );
        }

        #[test]
        fn first_party_and_proxy_routes_are_distinct() {
            let configuration = configuration(
                "T5",
                "openrouter/anthropic/claude-opus-5",
                &[],
                "xhigh",
                "{}",
            );
            let resolver = ConfiguredModelResolver::from_text_with_rpc(
                &configuration,
                &catalog(&[("openrouter", "anthropic/claude-opus-5")]),
                r#"{"models":[{"provider":"openrouter","id":"anthropic/claude-opus-5","reasoning":true,"thinkingLevelMap":{"xhigh":"xhigh"}}]}"#,
            )
            .expect("explicit xhigh route must load");

            assert_eq!(
                resolver.pool_judge(&candidate(Tier::T5, "anthropic", "claude-opus-5")),
                Ok(ModelIdentity {
                    tier: Tier::T5,
                    provider: "openrouter".to_owned(),
                    model: "anthropic/claude-opus-5".to_owned(),
                    thinking: "xhigh".to_owned(),
                })
            );
        }

        #[test]
        fn unavailable_route_is_skipped() {
            let configuration = configuration(
                "T5",
                "anthropic/unavailable",
                &["openai-codex/available"],
                "medium",
                "{}",
            );
            let resolver = resolver(&configuration, &catalog(&[("openai-codex", "available")]));

            assert_eq!(
                resolver.pool_judge(&candidate(Tier::T5, "anthropic", "candidate")),
                Ok(ModelIdentity {
                    tier: Tier::T5,
                    provider: "openai-codex".to_owned(),
                    model: "available".to_owned(),
                    thinking: "medium".to_owned(),
                })
            );
        }

        #[test]
        fn control_route_is_rejected_as_authority() {
            let controls = r#"{
    "free": {
      "pi": "openrouter/openrouter/free",
      "maximum_tier": "T1",
      "is_read_only": true
    }
  }"#;
            let configuration = configuration(
                "T5",
                "openrouter/openrouter/free",
                &["anthropic/external"],
                "low",
                controls,
            );
            let result = super::ConfiguredModelResolver::from_text(
                &configuration,
                &catalog(&[("openrouter", "openrouter/free"), ("anthropic", "external")]),
            );

            assert!(matches!(
                result,
                Err(SkillEvalError::InvalidConfiguration(message))
                    if message.contains("control") && message.contains("authority")
            ));
        }

        #[test]
        fn malformed_route_is_rejected() {
            let configuration =
                configuration("T5", "malformed", &["anthropic/external"], "medium", "{}");
            let result = super::ConfiguredModelResolver::from_text(
                &configuration,
                &catalog(&[("anthropic", "external")]),
            );

            assert!(matches!(
                result,
                Err(SkillEvalError::InvalidConfiguration(message))
                    if message.contains("provider/model")
            ));
        }

        #[test]
        fn judge_below_candidate_is_rejected() {
            let configuration = configuration("T4", "anthropic/judge", &[], "high", "{}");
            let resolver = resolver(&configuration, &catalog(&[("anthropic", "judge")]));

            assert!(matches!(
                resolver.pool_judge(&candidate(Tier::T5, "openai-codex", "candidate")),
                Err(SkillEvalError::InvalidConfiguration(message))
                    if message.contains("not above")
            ));
        }

        #[test]
        fn self_only_route_returns_judge_unavailable() {
            let configuration = configuration("T5", "anthropic/candidate", &[], "medium", "{}");
            let resolver = resolver(&configuration, &catalog(&[("anthropic", "candidate")]));
            let candidate = candidate(Tier::T5, "anthropic", "candidate");

            assert_eq!(
                resolver.pool_judge(&candidate),
                Err(SkillEvalError::JudgeUnavailable {
                    candidate,
                    judge_tier: Tier::T5,
                })
            );
        }

        #[test]
        fn control_only_route_is_rejected_as_authority() {
            let controls = r#"{
    "free": {
      "pi": "openrouter/openrouter/free",
      "maximum_tier": "T1",
      "is_read_only": true
    }
  }"#;
            let configuration =
                configuration("T5", "openrouter/openrouter/free", &[], "low", controls);
            let result = super::ConfiguredModelResolver::from_text(
                &configuration,
                &catalog(&[("openrouter", "openrouter/free")]),
            );

            assert!(matches!(
                result,
                Err(SkillEvalError::InvalidConfiguration(message))
                    if message.contains("control") && message.contains("authority")
            ));
        }

        #[test]
        fn unavailable_only_route_returns_judge_unavailable() {
            let configuration = configuration(
                "T5",
                "anthropic/unavailable",
                &["openai-codex/also-unavailable"],
                "medium",
                "{}",
            );
            let resolver = resolver(&configuration, CATALOG_HEADER);
            let candidate = candidate(Tier::T5, "anthropic", "candidate");

            assert_eq!(
                resolver.pool_judge(&candidate),
                Err(SkillEvalError::JudgeUnavailable {
                    candidate,
                    judge_tier: Tier::T5,
                })
            );
        }

        #[test]
        fn ordinary_judge_behavior_is_unchanged() {
            let configuration = configuration(
                "T5",
                "anthropic/primary",
                &["openai-codex/fallback"],
                "medium",
                "{}",
            );
            let resolver = resolver(
                &configuration,
                &catalog(&[("anthropic", "primary"), ("openai-codex", "fallback")]),
            );
            let ordinary_candidate = candidate(Tier::T4, "openai-codex", "ordinary");
            let before = resolver
                .judge(Tier::T5, Some(&ordinary_candidate))
                .expect("ordinary judge must resolve");

            assert_eq!(
                resolver.pool_judge(&candidate(Tier::T5, "anthropic", "primary")),
                Ok(ModelIdentity {
                    tier: Tier::T5,
                    provider: "openai-codex".to_owned(),
                    model: "fallback".to_owned(),
                    thinking: "medium".to_owned(),
                })
            );
            assert_eq!(
                resolver.judge(Tier::T5, Some(&ordinary_candidate)),
                Ok(before)
            );
        }
    }

    mod routing_controls {
        use std::fs;
        use std::path::{Path, PathBuf};
        use std::process::Command;
        use std::sync::atomic::{AtomicU64, Ordering};

        use crate::model::{ModelIdentity, SkillEvalError, Tier};
        use crate::ports::ModelResolver;

        use super::ConfiguredModelResolver;

        const CATALOG: &str = "provider model context max-out thinking images\nprovider ranked-primary 200K 64K yes yes\nprovider ranked-fallback 200K 64K yes yes\nprovider judge-primary 200K 64K yes yes\nprovider capability-primary 200K 64K yes yes\nprovider capability-fallback 200K 64K yes yes\ncontrol free 200K 64K no yes\n";
        static SCRATCH_SEQUENCE: AtomicU64 = AtomicU64::new(0);

        fn configuration(capabilities: &str, controls: &str) -> String {
            format!(
                r#"{{
  "tiers": {{
    "T1": {{
      "pi": "provider/ranked-primary",
      "fallbacks": ["provider/ranked-fallback"],
      "thinking": "low"
    }},
    "T5": {{
      "pi": "provider/judge-primary",
      "fallbacks": [],
      "thinking": "high"
    }}
  }},
  "orchestrator": "T1",
  "judge": "T5",
  "capabilities": {capabilities},
  "unranked_controls": {controls},
  "agents": {{"worker": "T1"}},
  "untiered": {{"delegate": "inherits"}}
}}"#
            )
        }

        fn valid_capability() -> &'static str {
            r#"{
    "summarize": {
      "pi": "provider/capability-primary",
      "fallbacks": ["provider/capability-fallback"]
    }
  }"#
        }

        fn valid_control() -> &'static str {
            r#"{
    "free": {
      "pi": "control/free",
      "maximum_tier": "T1",
      "is_read_only": true
    }
  }"#
        }

        fn assert_invalid(configuration: &str, expected: &str) {
            let result = ConfiguredModelResolver::from_text(configuration, CATALOG);
            assert!(matches!(
                result,
                Err(SkillEvalError::InvalidConfiguration(message))
                    if message.contains(expected)
            ));
        }

        #[test]
        fn routing_controls_empty_sections_preserve_ranked_t1() {
            let resolver = ConfiguredModelResolver::from_text(
                include_str!("../../../config/model-tiers.json"),
                include_str!("fixtures/models/catalog-all.txt"),
            )
            .expect("transitional routing must load");

            let candidates = resolver.candidates(Tier::T1).expect("T1 must resolve");

            assert_eq!(candidates[0].provider, "openrouter");
            assert_eq!(candidates[0].model, "openrouter/free");
        }

        #[test]
        fn routing_controls_tracked_authority_prefers_authenticated_first_party_hosts() {
            let resolver = ConfiguredModelResolver::from_text(
                include_str!("../../../config/model-tiers.json"),
                include_str!("fixtures/models/catalog-all.txt"),
            )
            .expect("tracked routing must load");

            for tier in [Tier::T2, Tier::T3, Tier::T4, Tier::T5] {
                for candidate in resolver.candidates(tier).expect("tier must resolve") {
                    assert!(
                        ["anthropic", "openai-codex"].contains(&candidate.provider.as_str()),
                        "{} must use an authenticated first-party host",
                        candidate.model
                    );
                }
            }
        }

        #[test]
        fn routing_controls_valid_separate_routes_do_not_change_ranked_candidates() {
            let resolver = ConfiguredModelResolver::from_text(
                &configuration(valid_capability(), valid_control()),
                CATALOG,
            )
            .expect("separate capability and control routes must load");

            let candidates = resolver.candidates(Tier::T1).expect("T1 must resolve");

            assert_eq!(candidates.len(), 2);
            assert_eq!(candidates[0].model, "ranked-primary");
            assert_eq!(candidates[1].model, "ranked-fallback");
        }

        #[test]
        fn routing_controls_reject_control_in_ranked_primary_fallback_or_judge_route() {
            let primary = configuration(valid_capability(), valid_control())
                .replace("provider/ranked-primary", "control/free");
            let fallback = configuration(valid_capability(), valid_control())
                .replace("provider/ranked-fallback", "control/free");
            let judge = configuration(valid_capability(), valid_control())
                .replace("provider/judge-primary", "control/free");

            for configuration in [primary, fallback, judge] {
                assert_invalid(&configuration, "authority");
            }
        }

        #[test]
        fn routing_controls_reject_control_as_exact_decision_or_publication_evidence() {
            let resolver = ConfiguredModelResolver::from_text(
                &configuration(valid_capability(), valid_control()),
                CATALOG,
            )
            .expect("separate control must load");
            let control = ModelIdentity {
                tier: Tier::T1,
                provider: "control".to_owned(),
                model: "free".to_owned(),
                thinking: "off".to_owned(),
            };

            assert!(matches!(
                resolver.exact_candidate(&control),
                Err(SkillEvalError::InvalidConfiguration(message))
                    if message.contains("unranked control")
            ));
        }

        #[test]
        fn routing_controls_reject_unknown_malformed_and_duplicate_configuration() {
            let base = configuration(valid_capability(), valid_control());
            let cases = [
                (
                    base.replace("\"agents\":", "\"unknown\": 1,\n  \"agents\":"),
                    "unknown field",
                ),
                (
                    base.replace("\"summarize\"", "\"Bad Key\""),
                    "capability key",
                ),
                (
                    base.replace(
                        "\"fallbacks\": [\"provider/capability-fallback\"]",
                        "\"fallbacks\": [\"provider/capability-fallback\"], \"thinking\": null",
                    ),
                    "malformed",
                ),
                (base.replace("control/free", "control/../free"), "malformed"),
                (
                    base.replace("\"maximum_tier\": \"T1\"", "\"maximum_tier\": \"T2\""),
                    "maximum tier",
                ),
                (
                    base.replace("\"is_read_only\": true", "\"is_read_only\": false"),
                    "read-only",
                ),
                (
                    base.replace(
                        "\"provider/ranked-fallback\"",
                        "\"provider/ranked-primary\"",
                    ),
                    "duplicate model",
                ),
                (base.replace("\"judge\": \"T5\"", "\"judge\": \"T9\""), "T9"),
                (
                    base.replace(
                        "\"judge\": \"T5\"",
                        "\"judge\": \"T5\",\n  \"judge\": \"T5\"",
                    ),
                    "duplicate configuration key",
                ),
            ];

            for (configuration, expected) in cases {
                assert_invalid(&configuration, expected);
            }
        }

        struct ScratchDirectory(PathBuf);

        impl ScratchDirectory {
            fn new() -> Self {
                let sequence = SCRATCH_SEQUENCE.fetch_add(1, Ordering::Relaxed);
                let path = std::env::temp_dir().join(format!(
                    "skill-eval-routing-controls-{}-{sequence}",
                    std::process::id()
                ));
                fs::create_dir_all(&path).expect("scratch directory must be created");
                Self(path)
            }

            fn path(&self) -> &Path {
                &self.0
            }
        }

        impl Drop for ScratchDirectory {
            fn drop(&mut self) {
                fs::remove_dir_all(&self.0).expect("scratch directory must be removed");
            }
        }

        #[test]
        fn routing_controls_installer_compiles_owned_fields_and_preserves_local_settings() {
            let scratch = ScratchDirectory::new();
            let repository = scratch.path().join("repository");
            let home = scratch.path().join("home");
            fs::create_dir_all(repository.join("config")).expect("config directory must exist");
            fs::create_dir_all(repository.join("skills")).expect("skills directory must exist");
            fs::create_dir_all(repository.join("agents")).expect("agents directory must exist");
            fs::create_dir_all(home.join(".pi/agent")).expect("Pi directory must exist");

            let project_root = Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .and_then(Path::parent)
                .expect("project root must exist");
            let installer = repository.join("install.sh");
            fs::copy(project_root.join("install.sh"), &installer)
                .expect("installer must be copied");
            fs::write(
                repository.join("config/model-tiers.json"),
                installer_configuration(),
            )
            .expect("routing configuration must be written");
            let settings = home.join(".pi/agent/settings.json");
            fs::write(
                &settings,
                r#"{"localSetting":{"keep":true},"capabilityRoutes":{"stale":true},"unrankedControls":{"stale":true}}"#,
            )
            .expect("local settings must be written");

            let output = Command::new("/bin/bash")
                .arg(&installer)
                .env("REPO_TARGET", &repository)
                .env("HOME_TARGET", &home)
                .env("HOME", &home)
                .output()
                .expect("installer must run");
            assert!(
                output.status.success(),
                "installer failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );

            let installed: serde_json::Value = serde_json::from_slice(
                &fs::read(&settings).expect("installed settings must be read"),
            )
            .expect("installed settings must be valid JSON");
            assert_eq!(installed["localSetting"]["keep"], true);
            assert_eq!(
                installed["capabilityRoutes"]["summarize"]["model"],
                "provider/capability-primary"
            );
            assert_eq!(
                installed["capabilityRoutes"]["summarize"]["fallbackModels"],
                serde_json::json!(["provider/capability-fallback"])
            );
            assert!(
                installed["capabilityRoutes"]["summarize"]
                    .get("thinking")
                    .is_none()
            );
            assert_eq!(
                installed["unrankedControls"]["free"],
                serde_json::json!({
                    "model": "control/free",
                    "maximumTier": "T1",
                    "isReadOnly": true
                })
            );
            assert_eq!(
                installed["modelTierFallbacks"]["provider/t1-primary"],
                "provider/t1-fallback"
            );

            let before_dry_run = fs::read(&settings).expect("settings must be readable");
            let dry_run = Command::new("/bin/bash")
                .arg(&installer)
                .arg("--dry-run")
                .env("REPO_TARGET", &repository)
                .env("HOME_TARGET", &home)
                .env("HOME", &home)
                .output()
                .expect("dry-run installer must run");
            assert!(dry_run.status.success());
            assert_eq!(
                fs::read(&settings).expect("settings must remain readable"),
                before_dry_run
            );

            fs::write(
                repository.join("config/model-tiers.json"),
                installer_configuration().replace("provider/t1-primary", "control/free"),
            )
            .expect("invalid routing configuration must be written");
            let rejected = Command::new("/bin/bash")
                .arg(&installer)
                .env("REPO_TARGET", &repository)
                .env("HOME_TARGET", &home)
                .env("HOME", &home)
                .output()
                .expect("invalid installer run must finish");
            assert!(!rejected.status.success());
            assert!(String::from_utf8_lossy(&rejected.stderr).contains("unsafe routing"));
            assert_eq!(
                fs::read(&settings).expect("settings must remain readable"),
                before_dry_run
            );
        }

        fn installer_configuration() -> &'static str {
            r#"{
  "tiers": {
    "T1": {"pi":"provider/t1-primary","fallbacks":["provider/t1-fallback"],"thinking":"low"},
    "T2": {"pi":"provider/t2-primary","fallbacks":["provider/t2-fallback"],"thinking":"low"},
    "T3": {"pi":"provider/t3-primary","fallbacks":["provider/t3-fallback"],"thinking":"medium"},
    "T4": {"pi":"provider/t4-primary","fallbacks":["provider/t4-fallback"],"thinking":"high"},
    "T5": {"pi":"provider/t5-primary","fallbacks":["provider/t5-fallback"],"thinking":"high"}
  },
  "orchestrator": "T3",
  "judge": "T5",
  "capabilities": {
    "summarize": {
      "pi": "provider/capability-primary",
      "fallbacks": ["provider/capability-fallback"]
    }
  },
  "unranked_controls": {
    "free": {
      "pi": "control/free",
      "maximum_tier": "T1",
      "is_read_only": true
    }
  },
  "agents": {},
  "untiered": {}
}"#
        }
    }
}
