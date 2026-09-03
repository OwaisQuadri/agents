use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const MAX_LEASES: usize = 3;
const LOCK_RETRY_DELAY: Duration = Duration::from_millis(20);
const WATCH_FIELDS: &str =
    "number,isDraft,mergeStateStatus,reviewDecision,statusCheckRollup,updatedAt";

type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Deserialize, Serialize)]
struct Lease {
    run: String,
    repo: String,
    kind: String,
    task: Option<String>,
    stage: Option<String>,
    pr: Option<u64>,
    pid: u32,
    heartbeat: u64,
    #[serde(rename = "priorStatus")]
    prior_status: Option<String>,
}

#[derive(Default, Deserialize, Serialize)]
struct Registry {
    leases: Vec<Lease>,
}

#[derive(Serialize)]
struct LeaseOutput<'a> {
    status: &'a str,
    siblings: &'a [Lease],
}

struct Acquire {
    run: String,
    repo: String,
    kind: String,
    task: Option<String>,
    prior_status: Option<String>,
    pr: Option<u64>,
    pid: u32,
}

struct Heartbeat {
    run: String,
    stage: Option<String>,
    pr: Option<u64>,
}

fn main() -> ExitCode {
    match run(env::args().skip(1)) {
        Ok(output) => {
            if !output.is_empty() {
                println!("{output}");
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(1)
        }
    }
}

fn run(args: impl Iterator<Item = String>) -> Result<String> {
    let arguments: Vec<String> = args.collect();
    let command = arguments.first().ok_or_else(usage)?;
    match command.as_str() {
        "acquire" => acquire_command(&arguments[1..]),
        "heartbeat" => heartbeat_command(&arguments[1..]),
        "list" => list_command(&arguments[1..]),
        "release" => release_command(&arguments[1..]),
        "watch-prs" => watch_prs_command(&arguments[1..]),
        "repair-worktree" => repair_worktree_command(&arguments[1..]),
        other => Err(format!("unknown command {other}; {}", usage())),
    }
}

fn usage() -> String {
    "usage: autonomous-engineer-state <acquire|heartbeat|list|release|watch-prs|repair-worktree> ..."
        .to_owned()
}

fn acquire_command(arguments: &[String]) -> Result<String> {
    let acquire = parse_acquire(arguments)?;
    let directory = state_directory()?;
    with_registry(&directory, |registry| {
        cleanup_stale(registry);
        registry.leases.retain(|lease| lease.run != acquire.run);
        let is_globally_full = registry.leases.len() >= MAX_LEASES;
        let is_task_already_running = acquire.kind == "task"
            && registry
                .leases
                .iter()
                .any(|lease| lease.kind == "task" && lease.repo == acquire.repo);
        let status = if is_globally_full || is_task_already_running {
            "denied"
        } else {
            registry.leases.push(Lease {
                run: acquire.run,
                repo: acquire.repo,
                kind: acquire.kind,
                task: acquire.task,
                stage: None,
                pr: acquire.pr,
                pid: acquire.pid,
                heartbeat: now_seconds()?,
                prior_status: acquire.prior_status,
            });
            "acquired"
        };
        render_lease_output(status, registry)
    })
}

fn heartbeat_command(arguments: &[String]) -> Result<String> {
    let heartbeat = parse_heartbeat(arguments)?;
    let directory = state_directory()?;
    with_registry(&directory, |registry| {
        cleanup_stale(registry);
        let lease = registry
            .leases
            .iter_mut()
            .find(|lease| lease.run == heartbeat.run)
            .ok_or_else(|| format!("no live run named {}", heartbeat.run))?;
        if let Some(stage) = heartbeat.stage {
            lease.stage = Some(stage);
        }
        if let Some(pr) = heartbeat.pr {
            lease.pr = Some(pr);
        }
        lease.heartbeat = now_seconds()?;
        render_lease_output("updated", registry)
    })
}

fn list_command(arguments: &[String]) -> Result<String> {
    reject_arguments(arguments)?;
    let directory = state_directory()?;
    with_registry(&directory, |registry| {
        cleanup_stale(registry);
        render_lease_output("listed", registry)
    })
}

fn release_command(arguments: &[String]) -> Result<String> {
    let run = required_option(arguments, "--run")?;
    reject_unknown_options(arguments, &["--run"])?;
    let directory = state_directory()?;
    with_registry(&directory, |registry| {
        cleanup_stale(registry);
        registry.leases.retain(|lease| lease.run != run);
        render_lease_output("released", registry)
    })
}

fn render_lease_output(status: &str, registry: &Registry) -> Result<String> {
    serde_json::to_string(&LeaseOutput {
        status,
        siblings: &registry.leases,
    })
    .map_err(|error| format!("cannot render JSON: {error}"))
}

fn parse_acquire(arguments: &[String]) -> Result<Acquire> {
    let run = required_option(arguments, "--run")?;
    let repo = required_option(arguments, "--repo")?;
    let kind = required_option(arguments, "--kind")?;
    if !matches!(kind.as_str(), "task" | "pr-care") {
        return Err(format!("--kind must be task or pr-care, got {kind}"));
    }
    reject_unknown_options(
        arguments,
        &[
            "--run",
            "--repo",
            "--kind",
            "--task",
            "--prior-status",
            "--pr",
            "--pid",
        ],
    )?;
    Ok(Acquire {
        run,
        repo,
        kind,
        task: optional_option(arguments, "--task")?,
        prior_status: optional_option(arguments, "--prior-status")?,
        pr: optional_number(arguments, "--pr")?,
        pid: optional_pid(arguments)?.unwrap_or_else(controller_pid),
    })
}

fn parse_heartbeat(arguments: &[String]) -> Result<Heartbeat> {
    let run = required_option(arguments, "--run")?;
    reject_unknown_options(arguments, &["--run", "--stage", "--pr"])?;
    Ok(Heartbeat {
        run,
        stage: optional_option(arguments, "--stage")?,
        pr: optional_number(arguments, "--pr")?,
    })
}

fn required_option(arguments: &[String], option: &str) -> Result<String> {
    optional_option(arguments, option)?.ok_or_else(|| format!("{option} requires a value"))
}

fn optional_option(arguments: &[String], option: &str) -> Result<Option<String>> {
    let mut value = None;
    let mut index = 0;
    while index < arguments.len() {
        if arguments[index] == option {
            let candidate = arguments
                .get(index + 1)
                .ok_or_else(|| format!("{option} requires a value"))?;
            if candidate.starts_with("--") {
                return Err(format!("{option} requires a value"));
            }
            if value.replace(candidate.clone()).is_some() {
                return Err(format!("{option} appears more than once"));
            }
            index += 2;
        } else {
            index += 1;
        }
    }
    Ok(value)
}

fn optional_number(arguments: &[String], option: &str) -> Result<Option<u64>> {
    optional_option(arguments, option)?.map_or(Ok(None), |value| {
        value
            .parse()
            .map(Some)
            .map_err(|_| format!("{option} needs a number, got {value}"))
    })
}

fn optional_pid(arguments: &[String]) -> Result<Option<u32>> {
    optional_option(arguments, "--pid")?.map_or(Ok(None), |value| {
        value
            .parse()
            .map(Some)
            .map_err(|_| format!("--pid needs a process identifier, got {value}"))
    })
}

fn reject_arguments(arguments: &[String]) -> Result<()> {
    if arguments.is_empty() {
        Ok(())
    } else {
        Err(format!("unexpected argument {}", arguments[0]))
    }
}

fn reject_unknown_options(arguments: &[String], allowed: &[&str]) -> Result<()> {
    let mut index = 0;
    while index < arguments.len() {
        let option = &arguments[index];
        if !allowed.contains(&option.as_str()) {
            return Err(format!("unknown argument {option}"));
        }
        if arguments
            .get(index + 1)
            .is_none_or(|value| value.starts_with("--"))
        {
            return Err(format!("{option} requires a value"));
        }
        index += 2;
    }
    Ok(())
}

fn state_directory() -> Result<PathBuf> {
    state_directory_from(None)
}

fn state_directory_from(override_path: Option<PathBuf>) -> Result<PathBuf> {
    override_path
        .or_else(|| env::var_os("AUTONOMOUS_ENGINEER_STATE_DIR").map(PathBuf::from))
        .map_or_else(
            || {
                env::var_os("HOME")
                    .map(PathBuf::from)
                    .map(|home| home.join(".local/state/autonomous-engineer"))
                    .ok_or_else(|| "HOME is not set".to_owned())
            },
            Ok,
        )
}

fn with_registry<T>(
    directory: &Path,
    operation: impl FnOnce(&mut Registry) -> Result<T>,
) -> Result<T> {
    fs::create_dir_all(directory)
        .map_err(|error| format!("cannot create {}: {error}", directory.display()))?;
    let _lock = acquire_lock(directory)?;
    let mut registry = read_registry(directory)?;
    let result = operation(&mut registry)?;
    write_registry(directory, &registry)?;
    Ok(result)
}

struct Lock {
    path: PathBuf,
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn acquire_lock(directory: &Path) -> Result<Lock> {
    let path = directory.join("lock");
    loop {
        match fs::create_dir(&path) {
            Ok(()) => {
                fs::write(path.join("pid"), std::process::id().to_string()).map_err(|error| {
                    format!("cannot write lock owner in {}: {error}", path.display())
                })?;
                return Ok(Lock { path });
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                if stale_lock_owner(&path)? {
                    retire_stale_lock(&path)?;
                } else {
                    thread::sleep(LOCK_RETRY_DELAY);
                }
            }
            Err(error) => return Err(format!("cannot lock {}: {error}", path.display())),
        }
    }
}

fn stale_lock_owner(path: &Path) -> Result<bool> {
    let pid = match fs::read_to_string(path.join("pid")) {
        Ok(value) => value.trim().parse::<u32>().ok(),
        Err(_) => None,
    };
    Ok(pid.is_some_and(is_process_definitely_dead))
}

fn retire_stale_lock(path: &Path) -> Result<()> {
    let retired = path.with_file_name(format!("lock.stale-{}", now_seconds()?));
    fs::rename(path, &retired).map_err(|error| format!("cannot retire stale lock: {error}"))?;
    fs::remove_dir_all(&retired)
        .map_err(|error| format!("cannot remove retired stale lock: {error}"))
}

fn read_registry(directory: &Path) -> Result<Registry> {
    let path = directory.join("leases.json");
    match fs::read_to_string(&path) {
        Ok(contents) => serde_json::from_str(&contents)
            .map_err(|error| format!("cannot parse {}: {error}", path.display())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Registry::default()),
        Err(error) => Err(format!("cannot read {}: {error}", path.display())),
    }
}

fn write_registry(directory: &Path, registry: &Registry) -> Result<()> {
    let path = directory.join("leases.json");
    let temporary = directory.join(format!("leases.{}.tmp", std::process::id()));
    let contents =
        serde_json::to_vec(registry).map_err(|error| format!("cannot encode state: {error}"))?;
    fs::write(&temporary, contents)
        .map_err(|error| format!("cannot write {}: {error}", temporary.display()))?;
    fs::rename(&temporary, &path)
        .map_err(|error| format!("cannot replace {}: {error}", path.display()))
}

fn cleanup_stale(registry: &mut Registry) {
    registry
        .leases
        .retain(|lease| !is_process_definitely_dead(lease.pid));
}

fn controller_pid() -> u32 {
    let pid = unsafe { libc::getppid() };
    if pid > 0 {
        pid as u32
    } else {
        std::process::id()
    }
}

fn is_process_definitely_dead(pid: u32) -> bool {
    if pid == 0 || pid > i32::MAX as u32 {
        return false;
    }
    // `kill` does not dereference memory; pid is checked above, and ESRCH is the only removal proof.
    let result = unsafe { libc::kill(pid as i32, 0) };
    result != 0 && io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
}

fn now_seconds() -> Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|error| format!("clock is before UNIX epoch: {error}"))
}

fn watch_prs_command(arguments: &[String]) -> Result<String> {
    let repo = required_option(arguments, "--repo")?;
    let interval = required_option(arguments, "--interval-seconds")?
        .parse::<u64>()
        .map_err(|_| "--interval-seconds needs a positive number".to_owned())?;
    if interval == 0 {
        return Err("--interval-seconds needs a positive number".to_owned());
    }
    reject_unknown_options(arguments, &["--repo", "--interval-seconds"])?;
    let mut previous = None;
    loop {
        let snapshot = fetch_pr_snapshot(&repo)?;
        if snapshot.is_empty() {
            return Ok(String::new());
        }
        if previous.as_ref() != Some(&snapshot) {
            println!(
                "{}",
                serde_json::to_string(&snapshot)
                    .map_err(|error| format!("cannot render JSON: {error}"))?
            );
            previous = Some(snapshot);
        }
        thread::sleep(Duration::from_secs(interval));
    }
}

#[derive(Clone, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct PrSnapshot {
    number: u64,
    #[serde(default)]
    is_draft: bool,
    #[serde(default)]
    merge_state_status: String,
    #[serde(default)]
    review_decision: Option<String>,
    #[serde(default)]
    status_check_rollup: serde_json::Value,
    #[serde(default)]
    updated_at: String,
}

fn fetch_pr_snapshot(repo: &str) -> Result<Vec<PrSnapshot>> {
    let output = Command::new("gh")
        .args([
            "pr",
            "list",
            "--repo",
            repo,
            "--state",
            "open",
            "--label",
            "autonomous-engineer",
            "--json",
            WATCH_FIELDS,
        ])
        .output()
        .map_err(|error| format!("cannot run gh: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "gh pr list failed for {repo}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    parse_pr_snapshot(&output.stdout)
}

fn parse_pr_snapshot(bytes: &[u8]) -> Result<Vec<PrSnapshot>> {
    let mut snapshots: Vec<PrSnapshot> = serde_json::from_slice(bytes)
        .map_err(|error| format!("cannot parse gh pr list output: {error}"))?;
    snapshots.sort_by_key(|snapshot| snapshot.number);
    Ok(snapshots)
}

fn repair_worktree_command(arguments: &[String]) -> Result<String> {
    let repo = PathBuf::from(required_option(arguments, "--repo")?);
    reject_unknown_options(arguments, &["--repo"])?;
    repair_worktree(&repo)?;
    Ok("{\"status\":\"ready\"}".to_owned())
}

fn repair_worktree(repo: &Path) -> Result<()> {
    let probe = git_probe(repo)?;
    if probe.status.success() && is_inside_worktree(&probe.stdout) {
        return Ok(());
    }
    if !is_bare_worktree_failure(&probe.stderr) && !is_outside_worktree(&probe.stdout) {
        return Err(git_failure(repo, &probe));
    }
    let gitdir = resolve_worktree_gitdir(repo)?;
    let mut config_command = Command::new("git");
    config_command
        .arg(format!("--git-dir={}", gitdir.display()))
        .arg(format!("--work-tree={}", repo.display()));
    let extension = config_command
        .args(["config", "extensions.worktreeConfig", "true"])
        .output()
        .map_err(|error| format!("cannot run git config: {error}"))?;
    if !extension.status.success() {
        return Err(format!(
            "git config failed for {}: {}",
            repo.display(),
            String::from_utf8_lossy(&extension.stderr).trim()
        ));
    }
    let configured = Command::new("git")
        .arg(format!("--git-dir={}", gitdir.display()))
        .arg(format!("--work-tree={}", repo.display()))
        .args(["config", "--worktree", "core.bare", "false"])
        .output()
        .map_err(|error| format!("cannot run git config: {error}"))?;
    if !configured.status.success() {
        return Err(format!(
            "git config failed for {}: {}",
            repo.display(),
            String::from_utf8_lossy(&configured.stderr).trim()
        ));
    }
    let retried = git_probe(repo)?;
    if retried.status.success() && is_inside_worktree(&retried.stdout) {
        Ok(())
    } else {
        Err(git_failure(repo, &retried))
    }
}

fn is_inside_worktree(stdout: &[u8]) -> bool {
    String::from_utf8_lossy(stdout).trim() == "true"
}

fn is_outside_worktree(stdout: &[u8]) -> bool {
    String::from_utf8_lossy(stdout).trim() == "false"
}

fn git_probe(repo: &Path) -> Result<std::process::Output> {
    Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .map_err(|error| format!("cannot run git for {}: {error}", repo.display()))
}

fn is_bare_worktree_failure(stderr: &[u8]) -> bool {
    String::from_utf8_lossy(stderr)
        .trim()
        .contains("fatal: this operation must be run in a work tree")
}

fn git_failure(repo: &Path, output: &std::process::Output) -> String {
    format!(
        "git probe failed for {}: {}",
        repo.display(),
        String::from_utf8_lossy(&output.stderr).trim()
    )
}

fn resolve_worktree_gitdir(repo: &Path) -> Result<PathBuf> {
    let git_path = repo.join(".git");
    let metadata = fs::metadata(&git_path)
        .map_err(|_| format!("{} is not a valid Git worktree", repo.display()))?;
    if metadata.is_dir() {
        return validate_gitdir(&git_path);
    }
    let contents = fs::read_to_string(&git_path)
        .map_err(|_| format!("{} is not a valid linked Git worktree", repo.display()))?;
    let reference = contents
        .trim()
        .strip_prefix("gitdir: ")
        .ok_or_else(|| format!("{} has an invalid .git file", repo.display()))?;
    let gitdir = PathBuf::from(reference);
    let gitdir = if gitdir.is_absolute() {
        gitdir
    } else {
        repo.join(gitdir)
    };
    validate_gitdir(&gitdir)
}

fn validate_gitdir(gitdir: &Path) -> Result<PathBuf> {
    let gitdir = fs::canonicalize(gitdir).map_err(|_| {
        format!(
            "{} does not point to a valid Git directory",
            gitdir.display()
        )
    })?;
    if (gitdir.join("HEAD").is_file() && gitdir.join("commondir").is_file())
        || gitdir.join("config").is_file()
    {
        Ok(gitdir)
    } else {
        Err(format!(
            "{} does not point to a valid Git directory",
            gitdir.display()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_directory(name: &str) -> PathBuf {
        let path = env::temp_dir().join(format!(
            "autonomous-engineer-state-{name}-{}-{}",
            std::process::id(),
            now_seconds().expect("clock")
        ));
        fs::create_dir_all(&path).expect("create fixture directory");
        path
    }

    fn git_success(arguments: &[&str]) {
        assert!(
            Command::new("git")
                .args(arguments)
                .status()
                .expect("run git")
                .success(),
            "git {arguments:?}"
        );
    }

    fn git_success_at(directory: &Path, arguments: &[&str]) {
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(directory)
                .args(arguments)
                .status()
                .expect("run git")
                .success(),
            "git -C {} {arguments:?}",
            directory.display()
        );
    }

    fn acquire(directory: &Path, run: &str, repo: &str, kind: &str) -> String {
        let request = Acquire {
            run: run.to_owned(),
            repo: repo.to_owned(),
            kind: kind.to_owned(),
            task: None,
            prior_status: None,
            pr: None,
            pid: std::process::id(),
        };
        let status = with_registry(directory, |registry| {
            cleanup_stale(registry);
            registry.leases.retain(|lease| lease.run != request.run);
            if registry.leases.len() >= MAX_LEASES
                || request.kind == "task"
                    && registry
                        .leases
                        .iter()
                        .any(|lease| lease.kind == "task" && lease.repo == request.repo)
            {
                return Ok("denied".to_owned());
            }
            registry.leases.push(Lease {
                run: request.run,
                repo: request.repo,
                kind: request.kind,
                task: None,
                stage: None,
                pr: None,
                pid: request.pid,
                heartbeat: now_seconds()?,
                prior_status: None,
            });
            Ok("acquired".to_owned())
        })
        .expect("admission");
        status
    }

    #[test]
    fn atomically_admits_only_three_runs() {
        let directory = fixture_directory("atomic");
        let mut workers = Vec::new();
        for index in 0..8 {
            let directory = directory.clone();
            workers.push(thread::spawn(move || {
                acquire(
                    &directory,
                    &format!("run-{index}"),
                    &format!("/repo-{index}"),
                    "task",
                )
            }));
        }
        let acquired = workers
            .into_iter()
            .filter_map(|worker| {
                worker
                    .join()
                    .expect("worker")
                    .strip_prefix("acquired")
                    .map(str::to_owned)
            })
            .count();
        assert_eq!(acquired, MAX_LEASES);
    }

    #[test]
    fn refuses_a_second_task_for_the_same_repo_but_allows_pr_care() {
        let directory = fixture_directory("same-repo");
        assert_eq!(acquire(&directory, "task-a", "/repo", "task"), "acquired");
        assert_eq!(acquire(&directory, "task-b", "/repo", "task"), "denied");
        assert_eq!(acquire(&directory, "care", "/repo", "pr-care"), "acquired");
    }

    #[test]
    fn heartbeat_and_release_change_only_the_named_run() {
        let directory = fixture_directory("heartbeat");
        assert_eq!(acquire(&directory, "one", "/one", "task"), "acquired");
        assert_eq!(acquire(&directory, "two", "/two", "task"), "acquired");
        with_registry(&directory, |registry| {
            let lease = registry
                .leases
                .iter_mut()
                .find(|lease| lease.run == "one")
                .expect("one");
            lease.stage = Some("test".to_owned());
            lease.pr = Some(42);
            Ok(())
        })
        .expect("heartbeat");
        with_registry(&directory, |registry| {
            registry.leases.retain(|lease| lease.run != "one");
            Ok(())
        })
        .expect("release");
        let registry = read_registry(&directory).expect("registry");
        assert_eq!(registry.leases.len(), 1);
        assert_eq!(registry.leases[0].run, "two");
    }

    #[test]
    fn removes_only_a_definitely_dead_process() {
        let mut registry = Registry {
            leases: vec![
                Lease {
                    run: "dead".to_owned(),
                    repo: "/dead".to_owned(),
                    kind: "task".to_owned(),
                    task: None,
                    stage: None,
                    pr: None,
                    pid: u32::MAX,
                    heartbeat: 0,
                    prior_status: None,
                },
                Lease {
                    run: "live".to_owned(),
                    repo: "/live".to_owned(),
                    kind: "task".to_owned(),
                    task: None,
                    stage: None,
                    pr: None,
                    pid: std::process::id(),
                    heartbeat: 0,
                    prior_status: None,
                },
            ],
        };
        cleanup_stale(&mut registry);
        assert_eq!(registry.leases.len(), 2);
        registry.leases[0].pid = 999_999_999;
        cleanup_stale(&mut registry);
        assert_eq!(registry.leases.len(), 1);
        assert_eq!(registry.leases[0].run, "live");
    }

    #[test]
    fn parses_and_orders_watcher_snapshots() {
        let snapshots = parse_pr_snapshot(
            br#"[{"number":2,"isDraft":false,"mergeStateStatus":"CLEAN","reviewDecision":null,"statusCheckRollup":[],"updatedAt":"2026-09-01T00:00:00Z"},{"number":1,"isDraft":true,"mergeStateStatus":"BLOCKED","reviewDecision":"REVIEW_REQUIRED","statusCheckRollup":[],"updatedAt":"2026-09-02T00:00:00Z"}]"#,
        )
        .expect("snapshot");
        assert_eq!(
            snapshots
                .iter()
                .map(|snapshot| snapshot.number)
                .collect::<Vec<_>>(),
            [1, 2]
        );
    }

    #[test]
    fn accepts_an_environment_state_directory() {
        let path = fixture_directory("environment");
        assert_eq!(
            state_directory_from(Some(path.clone())).expect("state directory"),
            path
        );
    }

    #[test]
    fn repair_ignores_a_healthy_primary_worktree() {
        let directory = fixture_directory("repair");
        git_success(&["init", "--quiet", directory.to_str().expect("path")]);
        repair_worktree(&directory).expect("healthy worktree");
    }

    #[test]
    fn repair_recovers_a_linked_worktree_with_a_bare_override() {
        let root = fixture_directory("linked-repair");
        let primary = root.join("primary");
        let linked = root.join("linked");
        git_success(&["init", "--quiet", primary.to_str().expect("primary path")]);
        git_success_at(&primary, &["config", "user.email", "test@example.invalid"]);
        git_success_at(&primary, &["config", "user.name", "Test"]);
        fs::write(primary.join("file"), "fixture").expect("fixture file");
        git_success_at(&primary, &["add", "file"]);
        git_success_at(&primary, &["commit", "--quiet", "-m", "fixture"]);
        git_success_at(
            &primary,
            &[
                "worktree",
                "add",
                "--quiet",
                "-b",
                "linked-branch",
                linked.to_str().expect("linked path"),
            ],
        );
        git_success_at(&primary, &["config", "extensions.worktreeConfig", "true"]);
        git_success_at(&linked, &["config", "--worktree", "core.bare", "true"]);
        let before = git_probe(&linked).expect("probe before repair");
        assert!(before.status.success());
        assert!(is_outside_worktree(&before.stdout));
        repair_worktree(&linked).expect("repair linked worktree");
        let after = git_probe(&linked).expect("probe after repair");
        assert!(after.status.success());
        assert!(is_inside_worktree(&after.stdout));
        let configured = Command::new("git")
            .arg("-C")
            .arg(&linked)
            .args(["config", "--worktree", "--get", "core.bare"])
            .output()
            .expect("read config");
        assert!(configured.status.success());
        assert_eq!(String::from_utf8_lossy(&configured.stdout).trim(), "false");
    }

    #[test]
    fn repair_rejects_invalid_paths_and_other_git_errors() {
        let directory = fixture_directory("invalid");
        assert!(repair_worktree(&directory.join("missing")).is_err());
        assert!(!is_bare_worktree_failure(b"fatal: not a git repository"));
    }
}
