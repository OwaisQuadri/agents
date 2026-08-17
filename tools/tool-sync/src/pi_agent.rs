use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Eq, PartialEq)]
struct AgentSource {
    path: PathBuf,
    name: String,
    description: String,
    tools: Vec<String>,
    model: String,
    prompt: String,
}

#[derive(Debug, Eq, PartialEq, Serialize)]
struct PiAgent {
    name: String,
    description: String,
    tools: Vec<String>,
    model: String,
    prompt: String,
}

#[derive(Debug, Eq, PartialEq)]
struct AgentAdapterReport {
    rendered: Vec<PathBuf>,
    overlapping_builtins: Vec<String>,
}
