use std::env;
use std::io::{self, Read};

const MODELS: [(&str, &str); 9] = [
    ("openrouter", "openrouter/free"),
    ("openai-codex", "gpt-5.6-luna"),
    ("openai-codex", "gpt-5.3-codex-spark"),
    ("anthropic", "claude-haiku-4-5"),
    ("anthropic", "claude-sonnet-5"),
    ("openai-codex", "gpt-5.6-terra"),
    ("anthropic", "claude-opus-5"),
    ("openai-codex", "gpt-5.6-sol"),
    ("anthropic", "claude-fable-5"),
];

fn main() {
    let arguments = env::args().skip(1).collect::<Vec<_>>();
    match arguments.as_slice() {
        [argument] if argument == "--list-models" => list_models(),
        [argument] if argument == "--version" => println!("synthetic-pi-h6"),
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
        _ => {
            eprintln!("H6 forbids live model execution");
            std::process::exit(64);
        }
    }
}

fn list_models() {
    println!("provider model context max-out thinking images");
    for (provider, model) in MODELS {
        println!("{provider} {model} 100000 10000 yes no");
    }
}

fn rpc_models() {
    let mut request = String::new();
    io::stdin().read_to_string(&mut request).unwrap();
    if request != "{\"id\":\"skill-eval-models\",\"type\":\"get_available_models\"}\n" {
        eprintln!("H6 rejects unexpected RPC input");
        std::process::exit(65);
    }
    let models = MODELS
        .map(|(provider, model)| {
            format!(r#"{{"provider":"{provider}","id":"{model}","reasoning":true}}"#)
        })
        .join(",");
    println!(
        r#"{{"id":"skill-eval-models","type":"response","command":"get_available_models","success":true,"data":{{"models":[{models}]}}}}"#
    );
}
