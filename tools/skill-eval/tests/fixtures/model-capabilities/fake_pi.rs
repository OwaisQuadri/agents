use std::env;
use std::fs::OpenOptions;
use std::io::{self, Read, Write};

const REQUEST: &str = "{\"id\":\"skill-eval-models\",\"type\":\"get_available_models\"}\n";
const BASE_COST: &str = r#"{"input":1.0,"output":2.0,"cacheRead":0.1,"cacheWrite":0.2}"#;
const VALID_TIERED_COST: &str = r#"{"input":0.0,"output":0.0,"cacheRead":0.1,"cacheWrite":0.2,"tiers":[{"inputTokensAbove":100000,"input":0.0,"output":0.25,"cacheRead":0.3,"cacheWrite":0.4},{"inputTokensAbove":200000,"input":0.5,"output":0.0,"cacheRead":0.6,"cacheWrite":0.7}]}"#;
const FREE_TIERED_COST: &str = r#"{"input":0.0,"output":0.0,"cacheRead":0.1,"cacheWrite":0.2,"tiers":[{"inputTokensAbove":100000,"input":0.0,"output":0.0,"cacheRead":0.3,"cacheWrite":0.4},{"inputTokensAbove":200000,"input":0.0,"output":0.0,"cacheRead":0.6,"cacheWrite":0.7}]}"#;

fn main() {
    let arguments = env::args().skip(1).collect::<Vec<_>>();
    record(&format!("args:{}", arguments.join(" ")));
    match arguments.as_slice() {
        [argument] if argument == "--list-models" => list_models(),
        [argument] if argument == "--version" => println!("synthetic-pi-1.2.3"),
        [mode, rpc, no_session, no_context, no_extensions]
            if [
                mode.as_str(),
                rpc.as_str(),
                no_session.as_str(),
                no_context.as_str(),
                no_extensions.as_str(),
            ] == [
                "--mode",
                "rpc",
                "--no-session",
                "--no-context-files",
                "--no-extensions",
            ] =>
        {
            rpc_models()
        }
        _ => fail("fake Pi rejects model or provider command"),
    }
}

fn list_models() {
    println!("provider model context max-out thinking images");
    match scenario().as_str() {
        "duplicate-list" => {
            println!("provider exact 100K 10K yes no");
            println!("provider exact 100K 10K yes no");
        }
        "conflicting-list" => {
            println!("openrouter vendor/moving 100K 10K yes no");
            println!("openrouter ~vendor/moving 100K 10K yes no");
        }
        "malformed-list" => println!("provider missing-columns"),
        _ => {
            println!("provider both 100K 10K yes no");
            println!("openrouter ~vendor/moving 100K 10K yes yes");
            println!("extension list-only 80K 8K no no");
            println!("anthropic core-holey 200K 64K yes yes");
            println!("anthropic core-default 200K 64K yes yes");
            if scenario().starts_with("auto-") {
                println!("openrouter openrouter/auto 100K 10K no no");
                println!("openrouter openrouter/auto-beta 100K 10K no no");
            }
        }
    }
}

fn rpc_models() {
    let mut request = String::new();
    io::stdin().read_to_string(&mut request).unwrap();
    record(&format!("stdin:{request:?}"));
    if request != REQUEST {
        fail("fake Pi rejects unexpected RPC input");
    }
    let valid_response = response(valid_data());
    match scenario().as_str() {
        "malformed-rpc" => println!("not-json"),
        "failed-rpc" => println!(
            "{{\"id\":\"skill-eval-models\",\"type\":\"response\",\"command\":\"get_available_models\",\"success\":false}}"
        ),
        "duplicate-response" => {
            println!("{valid_response}");
            println!("{valid_response}");
        }
        "duplicate-rpc" => println!("{}", response(duplicate_data())),
        "tiered-valid" => println!("{}", response(&priced_data(VALID_TIERED_COST))),
        "tiered-free-cache" => println!("{}", response(&priced_data(FREE_TIERED_COST))),
        "auto-valid" => println!(
            "{}",
            response(&auto_data(
                r#"{"input":-1000000.0,"output":-1000000.0,"cacheRead":0.0,"cacheWrite":0.0}"#,
                r#"{"input":-1.0,"output":-2.0,"cacheRead":0.1,"cacheWrite":0.2}"#
            ))
        ),
        "auto-mixed-sign" => println!(
            "{}",
            response(&auto_data(
                r#"{"input":-1.0,"output":1.0,"cacheRead":0.0,"cacheWrite":0.0}"#,
                BASE_COST
            ))
        ),
        "auto-negative-cache" => println!(
            "{}",
            response(&auto_data(
                r#"{"input":-1.0,"output":-1.0,"cacheRead":-0.1,"cacheWrite":0.0}"#,
                BASE_COST
            ))
        ),
        "auto-tiered-sentinel" => println!(
            "{}",
            response(&auto_data(
                r#"{"input":-1.0,"output":-1.0,"cacheRead":0.0,"cacheWrite":0.0,"tiers":[{"inputTokensAbove":100000,"input":0.0,"output":0.0,"cacheRead":0.0,"cacheWrite":0.0}]}"#,
                BASE_COST
            ))
        ),
        "tier-zero-threshold" => println!(
            "{}",
            response(&priced_data(&tiered_cost(0, 200_000, "0.0", "0.0")))
        ),
        "tier-duplicate-threshold" => println!(
            "{}",
            response(&priced_data(&tiered_cost(100_000, 100_000, "0.0", "0.0")))
        ),
        "tier-descending-threshold" => println!(
            "{}",
            response(&priced_data(&tiered_cost(200_000, 100_000, "0.0", "0.0")))
        ),
        "tier-missing-field" => println!(
            "{}",
            response(&priced_data(
                r#"{"input":0.0,"output":0.0,"cacheRead":0.0,"cacheWrite":0.0,"tiers":[{"inputTokensAbove":100000,"input":0.0,"output":0.0,"cacheRead":0.0}]}"#
            ))
        ),
        "tier-unknown-field" => println!(
            "{}",
            response(&priced_data(
                r#"{"input":0.0,"output":0.0,"cacheRead":0.0,"cacheWrite":0.0,"tiers":[{"inputTokensAbove":100000,"input":0.0,"output":0.0,"cacheRead":0.0,"cacheWrite":0.0,"currency":"USD"}]}"#
            ))
        ),
        "tier-negative-price" => println!(
            "{}",
            response(&priced_data(&tiered_cost(100_000, 200_000, "0.0", "-0.1")))
        ),
        "tier-malformed-price" => println!(
            "{}",
            response(&priced_data(&tiered_cost(
                100_000, 200_000, "\"free\"", "0.0"
            )))
        ),
        "tier-nonfinite-price" => println!(
            "{}",
            response(&priced_data(&tiered_cost(100_000, 200_000, "0.0", "1e400")))
        ),
        "base-negative-price" => println!(
            "{}",
            response(&priced_data(
                r#"{"input":-1.0,"output":0.0,"cacheRead":0.0,"cacheWrite":0.0}"#
            ))
        ),
        "non-control-negative" => println!(
            "{}",
            response(&priced_data(
                r#"{"input":-1.0,"output":-2.0,"cacheRead":0.0,"cacheWrite":0.0}"#
            ))
        ),
        "base-malformed-price" => println!(
            "{}",
            response(&priced_data(
                r#"{"input":0.0,"output":"paid","cacheRead":0.0,"cacheWrite":0.0}"#
            ))
        ),
        "base-nonfinite-price" => println!(
            "{}",
            response(&priced_data(
                r#"{"input":0.0,"output":0.0,"cacheRead":1e400,"cacheWrite":0.0}"#
            ))
        ),
        _ => println!("{valid_response}"),
    }
}

fn valid_data() -> &'static str {
    r#"{"models":[
{"provider":"provider","id":"both","name":"Both","reasoning":false,"input":["text"],"cost":{"input":1.0,"output":2.0,"cacheRead":0.1,"cacheWrite":0.2},"contextWindow":100000,"maxTokens":10000,"baseUrl":"https://secret.invalid","headers":{"authorization":"secret"}},
{"provider":"core","id":"rpc-only","name":"RPC only","reasoning":true,"input":["image","text"],"cost":{"input":3.0,"output":4.0,"cacheRead":0.3,"cacheWrite":0.4},"contextWindow":120000,"maxTokens":12000},
{"provider":"openrouter","id":"vendor/moving","name":"Moving","reasoning":true,"input":["text"],"contextWindow":100000,"maxTokens":10000},
{"provider":"anthropic","id":"core-holey","name":"Holey","reasoning":true,"thinkingLevelMap":{"off":"none","minimal":null,"low":null,"medium":null,"high":"high","xhigh":"xhigh","max":null},"input":["text","image"],"contextWindow":200000,"maxTokens":64000},
{"provider":"anthropic","id":"core-default","name":"Default","reasoning":true,"input":["text"],"contextWindow":200000,"maxTokens":64000}
]}"#
}

fn priced_data(cost: &str) -> String {
    valid_data().replacen(BASE_COST, cost, 1)
}

fn auto_data(auto_cost: &str, beta_cost: &str) -> String {
    let models = valid_data().strip_suffix("]}").unwrap();
    format!(
        r#"{models},
{{"provider":"openrouter","id":"openrouter/auto","name":"Auto","reasoning":false,"input":["text"],"cost":{auto_cost},"contextWindow":100000,"maxTokens":10000}},
{{"provider":"openrouter","id":"openrouter/auto-beta","name":"Auto beta","reasoning":false,"input":["text"],"cost":{beta_cost},"contextWindow":100000,"maxTokens":10000}}]}}"#
    )
}

fn tiered_cost(
    first_threshold: u64,
    second_threshold: u64,
    second_input: &str,
    second_cache_write: &str,
) -> String {
    format!(
        r#"{{"input":0.0,"output":0.0,"cacheRead":0.0,"cacheWrite":0.0,"tiers":[{{"inputTokensAbove":{first_threshold},"input":0.0,"output":0.0,"cacheRead":0.0,"cacheWrite":0.0}},{{"inputTokensAbove":{second_threshold},"input":{second_input},"output":0.0,"cacheRead":0.0,"cacheWrite":{second_cache_write}}}]}}"#
    )
}

fn duplicate_data() -> &'static str {
    r#"{"models":[
{"provider":"provider","id":"same","reasoning":true},
{"provider":"provider","id":"same","reasoning":false}
]}"#
}

fn response(data: &str) -> String {
    let data = data.replace('\n', "");
    format!(
        "{{\"id\":\"skill-eval-models\",\"type\":\"response\",\"command\":\"get_available_models\",\"success\":true,\"data\":{data}}}"
    )
}

fn scenario() -> String {
    env::var("FAKE_PI_SCENARIO").unwrap_or_else(|_| "valid".to_owned())
}

fn record(line: &str) {
    let Ok(path) = env::var("FAKE_PI_LOG") else {
        return;
    };
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    writeln!(file, "{line}").unwrap();
}

fn fail(message: &str) -> ! {
    eprintln!("{message}");
    std::process::exit(64)
}
