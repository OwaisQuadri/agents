use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, ExitCode};

use serde::{Deserialize, Serialize};
use toml_edit::{DocumentMut, Item, Table};

type Result<T> = std::result::Result<T, String>;

const CONFIG_KEYS: &str =
    "platform, exclude_bots, exclude_authors, require_mergeable, require_checks_pass, include_drafts, max";

const FETCH_LIMIT: &str = "100";
const PR_FIELDS: &str = "number,title,url,author,isDraft,mergeable,reviewDecision,reviewRequests,reviews,statusCheckRollup,baseRefName,headRefName";
const REVIEWED_FETCH_LIMIT: &str = "30";

struct Cli {
    command: Option<String>,
    assignments: Vec<String>,
    repo: Option<String>,
    config: Option<PathBuf>,
    max: Option<usize>,
    is_all: bool,
    is_json: bool,
}

#[derive(Clone, Deserialize)]
struct ConfigLayer {
    platform: Option<String>,
    #[serde(rename = "exclude_bots")]
    is_bots_excluded: Option<bool>,
    exclude_authors: Option<Vec<String>>,
    #[serde(rename = "require_mergeable")]
    is_mergeable_required: Option<bool>,
    #[serde(rename = "require_checks_pass")]
    is_checks_pass_required: Option<bool>,
    #[serde(rename = "include_drafts")]
    is_drafts_included: Option<bool>,
    max: Option<usize>,
}

#[derive(Deserialize)]
struct ConfigFile {
    defaults: Option<ConfigLayer>,
    #[serde(default)]
    repos: HashMap<String, ConfigLayer>,
}

struct Config {
    platform: String,
    is_bots_excluded: bool,
    exclude_authors: Vec<String>,
    is_mergeable_required: bool,
    is_checks_pass_required: bool,
    is_drafts_included: bool,
    max: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            platform: "github".to_owned(),
            is_bots_excluded: true,
            exclude_authors: Vec::new(),
            is_mergeable_required: true,
            is_checks_pass_required: false,
            is_drafts_included: false,
            max: 10,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Pr {
    number: u64,
    title: String,
    url: String,
    author: Author,
    #[serde(default)]
    is_draft: bool,
    #[serde(default)]
    mergeable: String,
    #[serde(default)]
    review_decision: Option<String>,
    #[serde(default)]
    review_requests: Vec<serde_json::Value>,
    #[serde(default)]
    reviews: Vec<Review>,
    #[serde(default)]
    status_check_rollup: Vec<CheckNode>,
    #[serde(default)]
    base_ref_name: String,
    #[serde(default)]
    head_ref_name: String,
}

#[derive(Deserialize)]
struct SearchPr {
    number: u64,
    #[serde(default)]
    commits: Vec<Commit>,
}

#[derive(Deserialize)]
struct Author {
    #[serde(default)]
    login: String,
    #[serde(default)]
    is_bot: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Review {
    author: Option<Author>,
    #[serde(default)]
    submitted_at: String,
}

#[derive(Deserialize)]
struct CheckNode {
    #[serde(default)]
    conclusion: String,
    #[serde(default)]
    state: String,
    #[serde(default)]
    status: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Commit {
    #[serde(default)]
    committed_date: String,
}

#[derive(Serialize)]
struct Report {
    repo: String,
    platform: String,
    inbox: Vec<Row>,
    unclaimed: Vec<Row>,
}

#[derive(Serialize)]
struct Row {
    number: u64,
    title: String,
    author: String,
    checks: String,
    stack_position: usize,
    stack_size: usize,
    url: String,
    review_url: String,
}

fn main() -> ExitCode {
    match run(env::args().skip(1)) {
        Ok(report) => {
            println!("{report}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(1)
        }
    }
}

fn run(args: impl Iterator<Item = String>) -> Result<String> {
    let cli = parse_arguments(args)?;
    let repo = match &cli.repo {
        Some(repo) => repo.clone(),
        None => repo_from_origin()?,
    };
    let config_path = match &cli.config {
        Some(path) => path.clone(),
        None => home()?.join("Documents/agents/config/pr-review.toml"),
    };
    match cli.command.as_deref() {
        Some("set") => {
            set_values(&config_path, &repo, &cli.assignments)?;
            let config = load_config(&config_path, &repo)?;
            return Ok(render_config(&repo, &config));
        }
        Some("show") => {
            let config = load_config(&config_path, &repo)?;
            return Ok(render_config(&repo, &config));
        }
        _ => {}
    }

    let mut config = load_config(&config_path, &repo)?;
    if let Some(max) = cli.max {
        config.max = max;
    }
    if cli.is_all {
        config.max = usize::MAX;
    }

    let login = current_login()?;
    let prs = fetch_prs(&repo)?;
    let is_reviewed_by_me = prs.iter().any(|pr| {
        pr.reviews.iter().any(|review| {
            review
                .author
                .as_ref()
                .is_some_and(|author| author.login == login)
        })
    });
    let pushes = if is_reviewed_by_me {
        fetch_reviewed_pushes(&repo, &login)?
    } else {
        HashMap::new()
    };
    let report = build_report(&repo, &config, &login, prs, &pushes);
    if cli.is_json {
        serde_json::to_string_pretty(&report)
            .map_err(|error| format!("cannot render JSON: {error}"))
    } else {
        Ok(render_table(&report))
    }
}

fn parse_arguments(mut args: impl Iterator<Item = String>) -> Result<Cli> {
    let mut cli = Cli {
        command: None,
        assignments: Vec::new(),
        repo: None,
        config: None,
        max: None,
        is_all: false,
        is_json: false,
    };
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--repo" => cli.repo = Some(required_value("--repo", &mut args)?),
            "--config" => cli.config = Some(PathBuf::from(required_value("--config", &mut args)?)),
            "--max" => {
                let value = required_value("--max", &mut args)?;
                cli.max = Some(
                    value
                        .parse()
                        .map_err(|_| format!("--max needs a number, got {value}"))?,
                );
            }
            "--all" => cli.is_all = true,
            "--json" => cli.is_json = true,
            "set" | "show" if cli.command.is_none() => cli.command = Some(argument),
            other if cli.command.as_deref() == Some("set") && other.contains('=') => {
                cli.assignments.push(argument)
            }
            other => return Err(usage(other)),
        }
    }
    if cli.command.as_deref() == Some("set") && cli.assignments.is_empty() {
        return Err(format!("set needs key=value pairs; keys: {CONFIG_KEYS}"));
    }
    Ok(cli)
}

fn usage(argument: &str) -> String {
    format!(
        "unknown argument {argument}; usage: pr-review-filter [set key=value ... | show] [--repo owner/name] [--config path] [--max N] [--all] [--json]"
    )
}

fn required_value(flag: &str, args: &mut impl Iterator<Item = String>) -> Result<String> {
    args.next()
        .ok_or_else(|| format!("{flag} requires a value"))
}

fn home() -> Result<PathBuf> {
    env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "HOME is not set".to_owned())
}

fn repo_from_origin() -> Result<String> {
    let output = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .output()
        .map_err(|error| format!("cannot run git: {error}"))?;
    if !output.status.success() {
        return Err("no origin remote here; pass --repo owner/name".to_owned());
    }
    let url = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    parse_remote(&url).ok_or_else(|| format!("cannot read owner/name from {url}; pass --repo"))
}

fn parse_remote(url: &str) -> Option<String> {
    let path = url
        .strip_prefix("git@")
        .and_then(|rest| rest.split_once(':'))
        .map(|(_, path)| path)
        .or_else(|| {
            url.split_once("://")
                .map(|(_, rest)| rest)
                .and_then(|rest| rest.split_once('/'))
                .map(|(_, path)| path)
        })?;
    let path = path.trim_end_matches('/').trim_end_matches(".git");
    let mut parts = path.rsplitn(2, '/');
    let name = parts.next()?;
    let owner = parts.next()?.rsplit('/').next()?;
    if owner.is_empty() || name.is_empty() {
        return None;
    }
    Some(format!("{owner}/{name}"))
}

fn load_config(path: &PathBuf, repo: &str) -> Result<Config> {
    let mut config = Config::default();
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(config),
        Err(error) => return Err(format!("cannot read {}: {error}", path.display())),
    };
    let file: ConfigFile = toml::from_str(&text)
        .map_err(|error| format!("{} is not valid TOML: {error}", path.display()))?;
    if let Some(layer) = &file.defaults {
        apply_layer(&mut config, layer);
    }
    if let Some(layer) = file.repos.get(repo) {
        apply_layer(&mut config, layer);
    }
    Ok(config)
}

fn apply_layer(config: &mut Config, layer: &ConfigLayer) {
    if let Some(platform) = &layer.platform {
        config.platform = platform.clone();
    }
    if let Some(value) = layer.is_bots_excluded {
        config.is_bots_excluded = value;
    }
    if let Some(authors) = &layer.exclude_authors {
        config.exclude_authors = authors.clone();
    }
    if let Some(value) = layer.is_mergeable_required {
        config.is_mergeable_required = value;
    }
    if let Some(value) = layer.is_checks_pass_required {
        config.is_checks_pass_required = value;
    }
    if let Some(value) = layer.is_drafts_included {
        config.is_drafts_included = value;
    }
    if let Some(value) = layer.max {
        config.max = value;
    }
}

fn set_values(path: &PathBuf, repo: &str, assignments: &[String]) -> Result<()> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(format!("cannot read {}: {error}", path.display())),
    };
    let mut document: DocumentMut = text
        .parse()
        .map_err(|error| format!("{} is not valid TOML: {error}", path.display()))?;

    let repos = document.entry("repos").or_insert_with(|| {
        let mut table = Table::new();
        table.set_implicit(true);
        Item::Table(table)
    });
    let repo_table = repos
        .as_table_mut()
        .ok_or_else(|| "the config key repos is not a table".to_owned())?
        .entry(repo)
        .or_insert(Item::Table(Table::new()));
    let repo_table = repo_table
        .as_table_mut()
        .ok_or_else(|| format!("the config entry for {repo} is not a table"))?;

    for assignment in assignments {
        let (key, value) = assignment
            .split_once('=')
            .ok_or_else(|| format!("{assignment} is not key=value"))?;
        repo_table.insert(key, config_item(key, value)?);
    }

    fs::write(path, document.to_string())
        .map_err(|error| format!("cannot write {}: {error}", path.display()))
}

fn config_item(key: &str, value: &str) -> Result<Item> {
    match key {
        "platform" => {
            if matches!(value, "github" | "graphite") {
                Ok(toml_edit::value(value))
            } else {
                Err(format!("platform must be github or graphite, got {value}"))
            }
        }
        "exclude_bots" | "require_mergeable" | "require_checks_pass" | "include_drafts" => value
            .parse::<bool>()
            .map(toml_edit::value)
            .map_err(|_| format!("{key} must be true or false, got {value}")),
        "max" => value
            .parse::<i64>()
            .ok()
            .filter(|max| *max >= 0)
            .map(toml_edit::value)
            .ok_or_else(|| format!("max must be a number, got {value}")),
        "exclude_authors" => {
            let mut array = toml_edit::Array::new();
            for author in value
                .split(',')
                .map(str::trim)
                .filter(|author| !author.is_empty())
            {
                array.push(author);
            }
            Ok(toml_edit::value(array))
        }
        other => Err(format!("unknown key {other}; keys: {CONFIG_KEYS}")),
    }
}

fn render_config(repo: &str, config: &Config) -> String {
    format!(
        "repo = {repo}\nplatform = {}\nexclude_bots = {}\nexclude_authors = {:?}\nrequire_mergeable = {}\nrequire_checks_pass = {}\ninclude_drafts = {}\nmax = {}",
        config.platform,
        config.is_bots_excluded,
        config.exclude_authors,
        config.is_mergeable_required,
        config.is_checks_pass_required,
        config.is_drafts_included,
        config.max
    )
}

fn current_login() -> Result<String> {
    let output = Command::new("gh")
        .args(["api", "user", "--jq", ".login"])
        .output()
        .map_err(|error| format!("cannot run gh: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "gh api user failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn fetch_prs(repo: &str) -> Result<Vec<Pr>> {
    let output = Command::new("gh")
        .args([
            "pr",
            "list",
            "--repo",
            repo,
            "--state",
            "open",
            "--limit",
            FETCH_LIMIT,
            "--json",
            PR_FIELDS,
        ])
        .output()
        .map_err(|error| format!("cannot run gh: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "gh pr list failed for {repo}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("cannot parse gh pr list output: {error}"))
}

fn fetch_reviewed_pushes(repo: &str, login: &str) -> Result<HashMap<u64, String>> {
    let output = Command::new("gh")
        .args([
            "pr",
            "list",
            "--repo",
            repo,
            "--state",
            "open",
            "--limit",
            REVIEWED_FETCH_LIMIT,
            "--search",
            &format!("reviewed-by:{login}"),
            "--json",
            "number,commits",
        ])
        .output()
        .map_err(|error| format!("cannot run gh: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "gh pr list --search reviewed-by failed for {repo}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let prs: Vec<SearchPr> = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("cannot parse gh reviewed-by output: {error}"))?;
    Ok(prs
        .into_iter()
        .filter_map(|pr| {
            pr.commits
                .iter()
                .map(|commit| commit.committed_date.clone())
                .max()
                .map(|date| (pr.number, date))
        })
        .collect())
}

fn build_report(
    repo: &str,
    config: &Config,
    login: &str,
    prs: Vec<Pr>,
    pushes: &HashMap<u64, String>,
) -> Report {
    let stacks = stack_positions(&prs);
    let mut inbox = Vec::new();
    let mut unclaimed = Vec::new();

    for pr in &prs {
        if pr.author.login == login
            || (config.is_bots_excluded && pr.author.is_bot)
            || config.exclude_authors.contains(&pr.author.login)
            || (!config.is_drafts_included && pr.is_draft)
            || (config.is_mergeable_required && pr.mergeable == "CONFLICTING")
        {
            continue;
        }
        let checks = checks_summary(&pr.status_check_rollup);
        if config.is_checks_pass_required && !matches!(checks, "pass" | "none") {
            continue;
        }

        let row = row_for(pr, checks, &stacks, config, repo);
        if is_inbox(pr, login, pushes) {
            inbox.push(row);
        } else if is_unclaimed(pr) {
            unclaimed.push(row);
        }
    }

    sort_rows(&mut inbox, &stacks);
    sort_rows(&mut unclaimed, &stacks);
    inbox.truncate(config.max);
    unclaimed.truncate(config.max);
    Report {
        repo: repo.to_owned(),
        platform: config.platform.clone(),
        inbox,
        unclaimed,
    }
}

fn is_inbox(pr: &Pr, login: &str, pushes: &HashMap<u64, String>) -> bool {
    let is_requested = pr
        .review_requests
        .iter()
        .any(|request| request.get("login").and_then(|value| value.as_str()) == Some(login));
    if is_requested {
        return true;
    }
    let my_last_review = pr
        .reviews
        .iter()
        .filter(|review| {
            review
                .author
                .as_ref()
                .is_some_and(|author| author.login == login)
        })
        .map(|review| review.submitted_at.as_str())
        .max()
        .unwrap_or("");
    if my_last_review.is_empty() {
        return false;
    }
    let last_push = pushes.get(&pr.number).map_or("", String::as_str);
    last_push > my_last_review
}

fn is_unclaimed(pr: &Pr) -> bool {
    pr.reviews.is_empty()
        && pr.review_requests.is_empty()
        && matches!(
            pr.review_decision.as_deref().unwrap_or(""),
            "" | "REVIEW_REQUIRED"
        )
}

fn checks_summary(nodes: &[CheckNode]) -> &'static str {
    if nodes.is_empty() {
        return "none";
    }
    let mut is_pending_seen = false;
    for node in nodes {
        let verdict = if node.conclusion.is_empty() {
            if node.state.is_empty() {
                &node.status
            } else {
                &node.state
            }
        } else {
            &node.conclusion
        };
        match verdict.as_str() {
            "SUCCESS" | "NEUTRAL" | "SKIPPED" => {}
            "" | "PENDING" | "EXPECTED" | "QUEUED" | "IN_PROGRESS" => is_pending_seen = true,
            _ => return "fail",
        }
    }
    if is_pending_seen {
        "pending"
    } else {
        "pass"
    }
}

struct StackPlace {
    root: u64,
    position: usize,
    size: usize,
}

fn stack_positions(prs: &[Pr]) -> HashMap<u64, StackPlace> {
    let by_head: HashMap<&str, &Pr> = prs
        .iter()
        .filter(|pr| !pr.head_ref_name.is_empty())
        .map(|pr| (pr.head_ref_name.as_str(), pr))
        .collect();

    let mut places = HashMap::new();
    let mut root_sizes: HashMap<u64, usize> = HashMap::new();
    for pr in prs {
        let mut depth = 0;
        let mut cursor = pr;
        let mut visited = vec![pr.number];
        while let Some(parent) = by_head.get(cursor.base_ref_name.as_str()) {
            if visited.contains(&parent.number) {
                break;
            }
            visited.push(parent.number);
            depth += 1;
            cursor = parent;
        }
        let root = cursor.number;
        *root_sizes.entry(root).or_default() += 1;
        places.insert(
            pr.number,
            StackPlace {
                root,
                position: depth + 1,
                size: 0,
            },
        );
    }
    for place in places.values_mut() {
        place.size = root_sizes.get(&place.root).copied().unwrap_or(1);
    }
    places
}

fn row_for(
    pr: &Pr,
    checks: &str,
    stacks: &HashMap<u64, StackPlace>,
    config: &Config,
    repo: &str,
) -> Row {
    let place = stacks.get(&pr.number);
    let review_url = if config.platform == "graphite" {
        format!("https://app.graphite.dev/github/pr/{repo}/{}", pr.number)
    } else {
        pr.url.clone()
    };
    Row {
        number: pr.number,
        title: pr.title.clone(),
        author: pr.author.login.clone(),
        checks: checks.to_owned(),
        stack_position: place.map_or(1, |place| place.position),
        stack_size: place.map_or(1, |place| place.size),
        url: pr.url.clone(),
        review_url,
    }
}

fn sort_rows(rows: &mut [Row], stacks: &HashMap<u64, StackPlace>) {
    rows.sort_by_key(|row| {
        let root = stacks
            .get(&row.number)
            .map_or(row.number, |place| place.root);
        (root, row.stack_position, row.number)
    });
}

fn render_table(report: &Report) -> String {
    let mut text = String::new();
    render_bucket(&mut text, "Review inbox", &report.inbox);
    text.push('\n');
    render_bucket(&mut text, "Unclaimed", &report.unclaimed);
    text
}

fn render_bucket(text: &mut String, heading: &str, rows: &[Row]) {
    text.push_str(&format!("## {heading} ({})\n", rows.len()));
    if rows.is_empty() {
        text.push_str("none\n");
        return;
    }
    text.push_str("\n| PR | title | author | checks | stack | link |\n");
    text.push_str("|---|---|---|---|---|---|\n");
    for row in rows {
        let stack = if row.stack_size > 1 {
            format!("{}/{}", row.stack_position, row.stack_size)
        } else {
            String::new()
        };
        text.push_str(&format!(
            "| #{} | {} | {} | {} | {} | {} |\n",
            row.number,
            row.title.replace('|', "\\|"),
            row.author,
            row.checks,
            stack,
            row.review_url
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pr(number: u64, head: &str, base: &str) -> Pr {
        Pr {
            number,
            title: format!("change {number}"),
            url: format!("https://github.com/o/r/pull/{number}"),
            author: Author {
                login: "alice".to_owned(),
                is_bot: false,
            },
            is_draft: false,
            mergeable: "MERGEABLE".to_owned(),
            review_decision: None,
            review_requests: Vec::new(),
            reviews: Vec::new(),
            status_check_rollup: Vec::new(),
            base_ref_name: base.to_owned(),
            head_ref_name: head.to_owned(),
        }
    }

    fn config() -> Config {
        Config::default()
    }

    #[test]
    fn parses_remote_urls() {
        for url in [
            "https://github.com/OwaisQuadri/agents.git",
            "https://github.com/OwaisQuadri/agents",
            "git@github.com:OwaisQuadri/agents.git",
            "ssh://git@github.com/OwaisQuadri/agents.git",
        ] {
            assert_eq!(
                parse_remote(url).as_deref(),
                Some("OwaisQuadri/agents"),
                "{url}"
            );
        }
        assert_eq!(parse_remote("nonsense"), None);
    }

    #[test]
    fn merges_config_layers_with_repo_overrides_winning() {
        let root = env::temp_dir().join(format!("pr-review-filter-config-{}", std::process::id()));
        fs::create_dir_all(&root).expect("create fixture root");
        let path = root.join("pr-review.toml");
        fs::write(
            &path,
            r#"[defaults]
require_checks_pass = true
max = 3

[repos."o/graphite-repo"]
platform = "graphite"
require_checks_pass = false
"#,
        )
        .expect("write config");

        let plain = load_config(&path, "o/plain-repo").expect("plain config");
        assert_eq!(plain.platform, "github");
        assert!(plain.is_checks_pass_required);
        assert_eq!(plain.max, 3);

        let graphite = load_config(&path, "o/graphite-repo").expect("graphite config");
        assert_eq!(graphite.platform, "graphite");
        assert!(!graphite.is_checks_pass_required);
        assert_eq!(graphite.max, 3);

        let missing = load_config(&root.join("absent.toml"), "o/plain-repo").expect("defaults");
        assert_eq!(missing.max, 10);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn classifies_requested_rereview_unclaimed_and_exclusions() {
        let mut requested = pr(1, "feat-1", "main");
        requested.review_requests = vec![serde_json::json!({"login": "me"})];

        let mut rereview = pr(2, "feat-2", "main");
        rereview.reviews = vec![Review {
            author: Some(Author {
                login: "me".to_owned(),
                is_bot: false,
            }),
            submitted_at: "2026-08-18T10:00:00Z".to_owned(),
        }];
        let mut pushes = HashMap::new();
        pushes.insert(2, "2026-08-19T09:00:00Z".to_owned());
        pushes.insert(3, "2026-08-18T09:00:00Z".to_owned());

        let mut reviewed_current = pr(3, "feat-3", "main");
        reviewed_current.reviews = vec![Review {
            author: Some(Author {
                login: "me".to_owned(),
                is_bot: false,
            }),
            submitted_at: "2026-08-19T10:00:00Z".to_owned(),
        }];

        let untouched = pr(4, "feat-4", "main");

        let mut own = pr(5, "feat-5", "main");
        own.author.login = "me".to_owned();

        let mut bot = pr(6, "feat-6", "main");
        bot.author.is_bot = true;

        let mut draft = pr(7, "feat-7", "main");
        draft.is_draft = true;

        let mut conflicting = pr(8, "feat-8", "main");
        conflicting.mergeable = "CONFLICTING".to_owned();

        let report = build_report(
            "o/r",
            &config(),
            "me",
            vec![
                requested,
                rereview,
                reviewed_current,
                untouched,
                own,
                bot,
                draft,
                conflicting,
            ],
            &pushes,
        );

        let inbox: Vec<u64> = report.inbox.iter().map(|row| row.number).collect();
        let unclaimed: Vec<u64> = report.unclaimed.iter().map(|row| row.number).collect();
        assert_eq!(inbox, [1, 2]);
        assert_eq!(unclaimed, [4]);
    }

    #[test]
    fn orders_stacks_bottom_first_and_marks_positions() {
        let bottom = pr(11, "stack-a", "main");
        let middle = pr(12, "stack-b", "stack-a");
        let top = pr(13, "stack-c", "stack-b");
        let single = pr(9, "solo", "main");

        let report = build_report(
            "o/r",
            &config(),
            "me",
            vec![top, single, middle, bottom],
            &HashMap::new(),
        );
        let unclaimed: Vec<(u64, usize, usize)> = report
            .unclaimed
            .iter()
            .map(|row| (row.number, row.stack_position, row.stack_size))
            .collect();
        assert_eq!(unclaimed, [(9, 1, 1), (11, 1, 3), (12, 2, 3), (13, 3, 3)]);
    }

    #[test]
    fn graphite_platform_swaps_the_review_link_and_keeps_the_github_url() {
        let mut config = config();
        config.platform = "graphite".to_owned();
        let report = build_report(
            "o/r",
            &config,
            "me",
            vec![pr(21, "feat", "main")],
            &HashMap::new(),
        );
        assert_eq!(
            report.unclaimed[0].review_url,
            "https://app.graphite.dev/github/pr/o/r/21"
        );
        assert_eq!(report.unclaimed[0].url, "https://github.com/o/r/pull/21");
    }

    #[test]
    fn set_writes_a_repo_table_and_keeps_the_defaults_table() {
        let root = env::temp_dir().join(format!("pr-review-filter-set-{}", std::process::id()));
        fs::create_dir_all(&root).expect("create fixture root");
        let path = root.join("pr-review.toml");
        fs::write(&path, "[defaults]\nmax = 3\n").expect("seed config");

        set_values(
            &path,
            "o/r",
            &[
                "platform=graphite".to_owned(),
                "exclude_authors=alice, bob".to_owned(),
                "max=5".to_owned(),
            ],
        )
        .expect("set succeeds");

        let text = fs::read_to_string(&path).expect("read config");
        assert!(text.starts_with("[defaults]\nmax = 3\n"), "{text}");
        assert!(text.contains("[repos.\"o/r\"]"), "{text}");

        let config = load_config(&path, "o/r").expect("config loads");
        assert_eq!(config.platform, "graphite");
        assert_eq!(config.exclude_authors, ["alice", "bob"]);
        assert_eq!(config.max, 5);
        assert_eq!(load_config(&path, "o/other").expect("other repo").max, 3);

        set_values(&path, "o/r", &["platform=github".to_owned()]).expect("second set");
        let config = load_config(&path, "o/r").expect("config reloads");
        assert_eq!(config.platform, "github");
        assert_eq!(config.max, 5);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn set_creates_a_missing_config_file() {
        let root = env::temp_dir().join(format!(
            "pr-review-filter-set-missing-{}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("create fixture root");
        let path = root.join("absent.toml");

        set_values(&path, "o/r", &["require_checks_pass=true".to_owned()]).expect("set succeeds");
        assert!(
            load_config(&path, "o/r")
                .expect("config loads")
                .is_checks_pass_required
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn set_rejects_unknown_keys_and_bad_values() {
        let root = env::temp_dir().join(format!(
            "pr-review-filter-set-invalid-{}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("create fixture root");
        let path = root.join("pr-review.toml");
        for assignment in [
            "colour=blue",
            "platform=gitlab",
            "max=many",
            "exclude_bots=1",
        ] {
            assert!(
                set_values(&path, "o/r", &[assignment.to_owned()]).is_err(),
                "{assignment}"
            );
        }
        assert!(!path.exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn parses_set_and_show_commands() {
        let cli = parse_arguments(
            ["set", "platform=graphite", "--repo", "o/r"]
                .into_iter()
                .map(str::to_owned),
        )
        .expect("set parses");
        assert_eq!(cli.command.as_deref(), Some("set"));
        assert_eq!(cli.assignments, ["platform=graphite"]);
        assert_eq!(cli.repo.as_deref(), Some("o/r"));

        let cli = parse_arguments(["show"].into_iter().map(str::to_owned)).expect("show parses");
        assert_eq!(cli.command.as_deref(), Some("show"));

        assert!(parse_arguments(["set"].into_iter().map(str::to_owned)).is_err());
        assert!(parse_arguments(["nonsense"].into_iter().map(str::to_owned)).is_err());
    }

    #[test]
    fn summarizes_checks_across_node_shapes() {
        let node = |conclusion: &str, state: &str, status: &str| CheckNode {
            conclusion: conclusion.to_owned(),
            state: state.to_owned(),
            status: status.to_owned(),
        };
        assert_eq!(checks_summary(&[]), "none");
        assert_eq!(checks_summary(&[node("SUCCESS", "", "")]), "pass");
        assert_eq!(checks_summary(&[node("", "SUCCESS", "")]), "pass");
        assert_eq!(
            checks_summary(&[node("SUCCESS", "", ""), node("", "", "IN_PROGRESS")]),
            "pending"
        );
        assert_eq!(
            checks_summary(&[node("SUCCESS", "", ""), node("FAILURE", "", "")]),
            "fail"
        );
    }
}
