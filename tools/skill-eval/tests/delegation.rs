use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

#[test]
fn parent_retains_control() {
    let fixture = Fixture::load();
    let route = fixture.route(&fixture.skill);

    assert_eq!(route.target_tier.as_deref(), Some("T2"));
    assert!(tier_rank(route.target_tier.as_deref().unwrap()) < tier_rank("T3"));
    assert_eq!(request_ids(&route.child), ["classification"]);
    assert_eq!(
        request_ids(&route.parent),
        ["direction", "publish", "accept"]
    );
    assert_eq!(child_output(route.child[0]), "classified: sample");
    assert_eq!(
        responsibilities(&route.parent),
        [
            "human_decision",
            "irreversible_action",
            "final_verification"
        ]
    );
}

#[test]
fn retained_actions_never_dispatch() {
    let fixture = Fixture::load();
    let route = fixture.route(&fixture.skill);

    assert!(
        route
            .child
            .iter()
            .all(|request| request.responsibility == "bounded_mechanical")
    );
    for request in &route.parent {
        assert_eq!(child_output(request), "FAIL retained responsibility");
    }
    assert!(fixture.skill.contains("does no work"));
}

#[test]
fn missing_target_metadata_preserves_the_parent_path() {
    let fixture = Fixture::load();
    let skill = fixture.skill.replace("  target-tier: T2\n", "");
    let route = fixture.route(&skill);

    assert_eq!(route.target_tier, None);
    assert!(route.child.is_empty());
    assert_eq!(
        request_ids(&route.parent),
        ["classification", "direction", "publish", "accept"]
    );
}

#[derive(Deserialize)]
struct Request {
    id: String,
    input: String,
    responsibility: String,
}

struct Route<'a> {
    target_tier: Option<String>,
    child: Vec<&'a Request>,
    parent: Vec<&'a Request>,
}

struct Fixture {
    skill: String,
    requests: Vec<Request>,
}

impl Fixture {
    fn load() -> Self {
        let root = fixture_root();
        let skill = fs::read_to_string(root.join("SKILL.md")).unwrap();
        let requests = fs::read_to_string(root.join("requests.jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert!(skill.contains("FAIL retained responsibility"));
        Self { skill, requests }
    }

    fn route<'a>(&'a self, skill: &str) -> Route<'a> {
        let target_tier = metadata_value(skill, "target-tier");
        if target_tier.is_none() {
            return Route {
                target_tier,
                child: Vec::new(),
                parent: self.requests.iter().collect(),
            };
        }
        let (child, parent) = self
            .requests
            .iter()
            .partition(|request| request.responsibility == "bounded_mechanical");
        Route {
            target_tier,
            child,
            parent,
        }
    }
}

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/delegation")
}

fn metadata_value(skill: &str, key: &str) -> Option<String> {
    skill.lines().find_map(|line| {
        let (candidate, value) = line.trim().split_once(':')?;
        (candidate == key).then(|| value.trim().to_owned())
    })
}

fn child_output(request: &Request) -> &'static str {
    if request.responsibility == "bounded_mechanical"
        && request.input == "mechanical: classify the sample"
    {
        "classified: sample"
    } else {
        "FAIL retained responsibility"
    }
}

fn request_ids<'a>(requests: &[&'a Request]) -> Vec<&'a str> {
    requests.iter().map(|request| request.id.as_str()).collect()
}

fn responsibilities<'a>(requests: &[&'a Request]) -> Vec<&'a str> {
    requests
        .iter()
        .map(|request| request.responsibility.as_str())
        .collect()
}

fn tier_rank(tier: &str) -> u8 {
    tier.strip_prefix('T').unwrap().parse().unwrap()
}
