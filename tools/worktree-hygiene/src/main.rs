//! Keeps a development checkout free of the debris that agent worktrees leave behind:
//! a local `main` that drifts behind `origin/main`, so every new worktree starts from a
//! stale base, and worktrees plus branches whose work already landed upstream, which
//! pile up until `git worktree list` is unreadable and branch names collide.
//!
//! The root cause is that nothing owns the cleanup. A worktree is created by whichever
//! agent needs one and abandoned the moment its pull request merges, and this repository
//! squash-merges, so the abandoned branch is never an ancestor of `origin/main` and no
//! ordinary `git branch -d` sweep reaches it. This binary decides, from git state alone,
//! which of them are provably finished, and applies the fix rather than reporting drift.
//!
//! A landed branch alone never justifies deleting a worktree: an agent working in one
//! sits on a landed, clean branch for most of its life, so the worktree also has to have
//! gone untouched for `STALE_AFTER_SECS` before removal. Branch deletion carries no such
//! wait, since a merged local branch is recoverable from the reflog.
//!
//! Run by the `com.owaisquadri.worktree-hygiene` launchd job every five minutes.
//! `--dry-run` prints the plan and touches nothing.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, PartialEq, Eq)]
struct Worktree {
    path: String,
    head: String,
    branch: Option<String>,
    is_locked: bool,
    is_primary: bool,
    idle_secs: u64,
}

#[derive(Clone, Debug)]
struct BranchFacts {
    name: String,
    tip: String,
    is_ancestor_of_upstream: bool,
    is_patch_equivalent_upstream: bool,
    worktree: Option<Worktree>,
    is_worktree_dirty: bool,
}

#[derive(Clone, Debug)]
struct RepoFacts {
    upstream_tip: String,
    main_tip: Option<String>,
    is_main_ancestor_of_upstream: bool,
    main_worktree: Option<Worktree>,
    is_main_worktree_dirty: bool,
    branches: Vec<BranchFacts>,
    invoked_from: String,
}

#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    Keep(&'static str),
    PruneWorktreeAndBranch,
    PruneBranchOnly,
}

#[derive(Debug, PartialEq, Eq)]
enum Action {
    FastForwardMainInWorktree { path: String },
    FastForwardMainRef,
    RemoveWorktree { path: String },
    DeleteBranch { name: String, is_force: bool },
}

const MAIN: &str = "main";
const UPSTREAM: &str = "origin/main";
const STALE_AFTER_SECS: u64 = 7 * 24 * 60 * 60;

fn parse_worktree_list(porcelain: &str) -> Vec<Worktree> {
    let mut out: Vec<Worktree> = Vec::new();
    let mut current: Option<Worktree> = None;
    for line in porcelain.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            if let Some(worktree) = current.take() {
                out.push(worktree);
            }
            continue;
        }
        let (key, value) = line.split_once(' ').unwrap_or((line, ""));
        match key {
            "worktree" => {
                if let Some(worktree) = current.take() {
                    out.push(worktree);
                }
                current = Some(Worktree {
                    path: value.to_string(),
                    head: String::new(),
                    branch: None,
                    is_locked: false,
                    is_primary: out.is_empty(),
                    idle_secs: 0,
                });
            }
            "HEAD" => {
                if let Some(worktree) = current.as_mut() {
                    worktree.head = value.to_string();
                }
            }
            "branch" => {
                if let Some(worktree) = current.as_mut() {
                    worktree.branch = Some(value.trim_start_matches("refs/heads/").to_string());
                }
            }
            "locked" => {
                if let Some(worktree) = current.as_mut() {
                    worktree.is_locked = true;
                }
            }
            _ => {}
        }
    }
    if let Some(worktree) = current.take() {
        out.push(worktree);
    }
    out
}

fn is_cherry_fully_landed(cherry_output: &str) -> bool {
    let mut is_any = false;
    for line in cherry_output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        is_any = true;
        if !line.starts_with('-') {
            return false;
        }
    }
    is_any
}

fn is_path_within(inner: &str, outer: &str) -> bool {
    let outer = outer.trim_end_matches('/');
    inner.trim_end_matches('/') == outer || inner.starts_with(&format!("{outer}/"))
}

fn classify_branch(facts: &BranchFacts, repo: &RepoFacts) -> Verdict {
    if facts.name == MAIN {
        return Verdict::Keep("the main branch");
    }
    if facts.tip == repo.upstream_tip {
        return Verdict::Keep("tip is origin/main's tip");
    }
    if let Some(worktree) = &facts.worktree {
        if worktree.is_locked {
            return Verdict::Keep("worktree is locked");
        }
        if worktree.is_primary {
            return Verdict::Keep("worktree is the primary checkout");
        }
        if is_path_within(&repo.invoked_from, &worktree.path) {
            return Verdict::Keep("this run was invoked from that worktree");
        }
        if worktree.idle_secs < STALE_AFTER_SECS {
            return Verdict::Keep("worktree was touched recently");
        }
        if facts.is_worktree_dirty {
            return Verdict::Keep("worktree has uncommitted work");
        }
    }
    if !facts.is_ancestor_of_upstream && !facts.is_patch_equivalent_upstream {
        return Verdict::Keep("no sign the work landed on origin/main");
    }
    if facts.worktree.is_some() {
        Verdict::PruneWorktreeAndBranch
    } else {
        Verdict::PruneBranchOnly
    }
}

fn plan_fast_forward(repo: &RepoFacts) -> Option<Action> {
    let main_tip = repo.main_tip.as_ref()?;
    if *main_tip == repo.upstream_tip || !repo.is_main_ancestor_of_upstream {
        return None;
    }
    match &repo.main_worktree {
        None => Some(Action::FastForwardMainRef),
        Some(_) if repo.is_main_worktree_dirty => None,
        Some(worktree) => Some(Action::FastForwardMainInWorktree {
            path: worktree.path.clone(),
        }),
    }
}

fn plan_actions(repo: &RepoFacts) -> Vec<Action> {
    let mut actions: Vec<Action> = Vec::new();
    if let Some(action) = plan_fast_forward(repo) {
        actions.push(action);
    }
    for branch in &repo.branches {
        let verdict = classify_branch(branch, repo);
        if verdict == Verdict::PruneWorktreeAndBranch {
            if let Some(worktree) = &branch.worktree {
                actions.push(Action::RemoveWorktree {
                    path: worktree.path.clone(),
                });
            }
        }
        if matches!(
            verdict,
            Verdict::PruneWorktreeAndBranch | Verdict::PruneBranchOnly
        ) {
            actions.push(Action::DeleteBranch {
                name: branch.name.clone(),
                is_force: !branch.is_ancestor_of_upstream,
            });
        }
    }
    actions
}

fn describe(action: &Action) -> String {
    match action {
        Action::FastForwardMainInWorktree { path } => format!("fast-forward main in {path}"),
        Action::FastForwardMainRef => "fast-forward the main ref".to_string(),
        Action::RemoveWorktree { path } => format!("remove worktree {path}"),
        Action::DeleteBranch { name, is_force } => {
            let flag = if *is_force { " (force)" } else { "" };
            format!("delete branch {name}{flag}")
        }
    }
}

fn git(repo: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .map_err(|error| format!("git {}: {error}", args.join(" ")))?;
    if !output.status.success() {
        return Err(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn git_ok(repo: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn rev_parse(repo: &Path, rev: &str) -> Option<String> {
    git(repo, &["rev-parse", "--verify", "--quiet", rev])
        .ok()
        .map(|out| out.trim().to_string())
        .filter(|out| !out.is_empty())
}

fn idle_secs(path: &Path) -> u64 {
    let modified = std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|since| since.as_secs());
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0);
    match modified {
        Some(modified) => now.saturating_sub(modified),
        None => 0,
    }
}

fn is_dirty(path: &Path) -> bool {
    match git(path, &["status", "--porcelain"]) {
        Ok(out) => !out.trim().is_empty(),
        Err(_) => true,
    }
}

fn gather(repo: &Path, invoked_from: &str) -> Result<RepoFacts, String> {
    let upstream_tip =
        rev_parse(repo, UPSTREAM).ok_or_else(|| format!("{UPSTREAM} does not resolve"))?;
    let main_tip = rev_parse(repo, MAIN);
    let is_main_ancestor_of_upstream = match &main_tip {
        Some(tip) => git_ok(repo, &["merge-base", "--is-ancestor", tip, &upstream_tip]),
        None => false,
    };

    let mut worktrees = parse_worktree_list(&git(repo, &["worktree", "list", "--porcelain"])?);
    for worktree in &mut worktrees {
        worktree.idle_secs = idle_secs(Path::new(&worktree.path));
    }
    let main_worktree = worktrees
        .iter()
        .find(|worktree| worktree.branch.as_deref() == Some(MAIN))
        .cloned();
    let is_main_worktree_dirty = main_worktree
        .as_ref()
        .is_some_and(|worktree| is_dirty(Path::new(&worktree.path)));

    let listing = git(
        repo,
        &[
            "for-each-ref",
            "--format=%(refname:short) %(objectname)",
            "refs/heads/",
        ],
    )?;
    let mut branches = Vec::new();
    for line in listing.lines() {
        let Some((name, tip)) = line.trim().split_once(' ') else {
            continue;
        };
        let worktree = worktrees
            .iter()
            .find(|worktree| worktree.branch.as_deref() == Some(name))
            .cloned();
        let is_worktree_dirty = worktree
            .as_ref()
            .is_some_and(|worktree| is_dirty(Path::new(&worktree.path)));
        branches.push(BranchFacts {
            name: name.to_string(),
            tip: tip.to_string(),
            is_ancestor_of_upstream: git_ok(
                repo,
                &["merge-base", "--is-ancestor", tip, &upstream_tip],
            ),
            is_patch_equivalent_upstream: git(repo, &["cherry", UPSTREAM, name])
                .map(|out| is_cherry_fully_landed(&out))
                .unwrap_or(false),
            worktree,
            is_worktree_dirty,
        });
    }

    Ok(RepoFacts {
        upstream_tip,
        main_tip,
        is_main_ancestor_of_upstream,
        main_worktree,
        is_main_worktree_dirty,
        branches,
        invoked_from: invoked_from.to_string(),
    })
}

fn apply(repo: &Path, action: &Action, upstream_tip: &str) -> Result<(), String> {
    match action {
        Action::FastForwardMainInWorktree { path } => {
            git(Path::new(path), &["merge", "--ff-only", UPSTREAM])?;
        }
        Action::FastForwardMainRef => {
            git(repo, &["update-ref", "refs/heads/main", upstream_tip, "--"])?;
        }
        Action::RemoveWorktree { path } => {
            git(repo, &["worktree", "remove", path])?;
        }
        Action::DeleteBranch { name, is_force } => {
            let flag = if *is_force { "-D" } else { "-d" };
            git(repo, &["branch", flag, name])?;
        }
    }
    verify(repo, action, upstream_tip)
}

fn verify(repo: &Path, action: &Action, upstream_tip: &str) -> Result<(), String> {
    match action {
        Action::FastForwardMainInWorktree { .. } | Action::FastForwardMainRef => {
            match rev_parse(repo, MAIN) {
                Some(tip) if tip == upstream_tip => Ok(()),
                other => Err(format!(
                    "main is {} after the fast-forward, expected {upstream_tip}",
                    other.unwrap_or_else(|| "missing".to_string())
                )),
            }
        }
        Action::RemoveWorktree { path } => {
            let listing = git(repo, &["worktree", "list", "--porcelain"])?;
            if parse_worktree_list(&listing)
                .iter()
                .any(|worktree| worktree.path == *path)
            {
                return Err(format!("worktree {path} still registered"));
            }
            Ok(())
        }
        Action::DeleteBranch { name, .. } => match rev_parse(repo, &format!("refs/heads/{name}")) {
            Some(_) => Err(format!("branch {name} still exists")),
            None => Ok(()),
        },
    }
}

fn run_repo(repo: &Path, is_dry: bool) -> bool {
    if let Err(error) = git(repo, &["fetch", "origin", "main", "--prune"]) {
        println!("FAIL {}: {error}", repo.display());
        return false;
    }
    let invoked_from = std::env::current_dir()
        .and_then(|dir| dir.canonicalize())
        .map(|dir| dir.display().to_string())
        .unwrap_or_default();
    let facts = match gather(repo, &invoked_from) {
        Ok(facts) => facts,
        Err(error) => {
            println!("FAIL {}: {error}", repo.display());
            return false;
        }
    };
    let mut is_ok = true;
    for action in plan_actions(&facts) {
        if is_dry {
            println!("would {}", describe(&action));
            continue;
        }
        match apply(repo, &action, &facts.upstream_tip) {
            Ok(()) => println!("{}", describe(&action)),
            Err(error) => {
                println!("FAIL {}: {error}", describe(&action));
                is_ok = false;
            }
        }
    }
    is_ok
}

fn default_repo() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/".to_string()))
        .join("Documents")
        .join("agents")
}

fn main() -> ExitCode {
    let mut is_dry = false;
    let mut repos: Vec<PathBuf> = Vec::new();
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--dry-run" => is_dry = true,
            other if other.starts_with('-') => {
                eprintln!("usage: worktree-hygiene [--dry-run] [<repo-path>...]");
                return ExitCode::FAILURE;
            }
            other => repos.push(PathBuf::from(other)),
        }
    }
    if repos.is_empty() {
        repos.push(default_repo());
    }
    let mut is_ok = true;
    for repo in &repos {
        if !run_repo(repo, is_dry) {
            is_ok = false;
        }
    }
    if is_ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn scratch_root() -> PathBuf {
        let index = COUNTER.fetch_add(1, Ordering::SeqCst);
        let root = std::env::temp_dir().join(format!(
            "worktree-hygiene-test-{}-{index}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("scratch root");
        root.canonicalize().expect("canonical scratch root")
    }

    fn run(repo: &Path, args: &[&str]) -> String {
        git(repo, args).unwrap_or_else(|error| panic!("{error}"))
    }

    fn commit(repo: &Path, file: &str, body: &str, message: &str) {
        fs::write(repo.join(file), body).expect("write");
        run(repo, &["add", "-A"]);
        run(repo, &["commit", "-q", "-m", message]);
    }

    fn facts(branches: Vec<BranchFacts>) -> RepoFacts {
        RepoFacts {
            upstream_tip: "upstreamsha".to_string(),
            main_tip: Some("upstreamsha".to_string()),
            is_main_ancestor_of_upstream: true,
            main_worktree: None,
            is_main_worktree_dirty: false,
            branches,
            invoked_from: "/somewhere/else".to_string(),
        }
    }

    fn branch(name: &str, tip: &str) -> BranchFacts {
        BranchFacts {
            name: name.to_string(),
            tip: tip.to_string(),
            is_ancestor_of_upstream: false,
            is_patch_equivalent_upstream: false,
            worktree: None,
            is_worktree_dirty: false,
        }
    }

    fn worktree(path: &str, name: &str) -> Worktree {
        Worktree {
            path: path.to_string(),
            head: "sha".to_string(),
            branch: Some(name.to_string()),
            is_locked: false,
            is_primary: false,
            idle_secs: STALE_AFTER_SECS + 1,
        }
    }

    fn backdate(path: &Path) {
        let status = Command::new("touch")
            .args(["-t", "202001010000"])
            .arg(path)
            .status()
            .expect("touch");
        assert!(status.success());
    }

    #[test]
    fn keeps_a_landed_worktree_that_was_touched_recently() {
        let mut item = branch("Al-Awwal", "oldsha");
        item.is_ancestor_of_upstream = true;
        let mut recent = worktree("/herdr/al-awwal", "Al-Awwal");
        recent.idle_secs = 3600;
        item.worktree = Some(recent);
        let repo = facts(vec![item.clone()]);
        assert_eq!(
            classify_branch(&item, &repo),
            Verdict::Keep("worktree was touched recently")
        );
    }

    fn path_of(root: &Path, name: &str) -> String {
        root.join(name).display().to_string()
    }

    #[test]
    fn parses_the_live_four_worktree_listing() {
        let porcelain = "\
worktree /Users/owaisquadri/Documents/agents
HEAD 4f2d91372
branch refs/heads/main

worktree /Users/owaisquadri/.herdr/worktrees/agents/al-awwal
HEAD eefd385b0
branch refs/heads/Al-Awwal

worktree /Users/owaisquadri/.herdr/worktrees/agents/al-jami
HEAD 4f2d91372
branch refs/heads/Al-Jami

worktree /Users/owaisquadri/.herdr/worktrees/agents/al-warith
HEAD 210fe4f05
branch refs/heads/Al-Warith
";
        let parsed = parse_worktree_list(porcelain);
        assert_eq!(parsed.len(), 4);
        assert!(parsed[0].is_primary);
        assert_eq!(parsed[0].branch.as_deref(), Some("main"));
        assert!(!parsed[3].is_primary);
        assert_eq!(parsed[2].branch.as_deref(), Some("Al-Jami"));
        assert_eq!(parsed[2].head, "4f2d91372");
        assert!(parsed.iter().all(|worktree| !worktree.is_locked));
    }

    #[test]
    fn parses_locked_and_detached_records() {
        let porcelain = "\
worktree /repo
HEAD aaa
branch refs/heads/main

worktree /tmp/locked
HEAD bbb
branch refs/heads/held
locked

worktree /tmp/loose
HEAD ccc
detached
";
        let parsed = parse_worktree_list(porcelain);
        assert_eq!(parsed.len(), 3);
        assert!(parsed[1].is_locked);
        assert_eq!(parsed[1].branch.as_deref(), Some("held"));
        assert_eq!(parsed[2].branch, None);
        assert!(!parsed[2].is_locked);
    }

    #[test]
    fn keeps_a_worktree_sitting_exactly_at_the_upstream_tip() {
        let mut item = branch("Al-Jami", "upstreamsha");
        item.worktree = Some(worktree("/herdr/al-jami", "Al-Jami"));
        item.is_ancestor_of_upstream = true;
        let repo = facts(vec![item.clone()]);
        assert_eq!(
            classify_branch(&item, &repo),
            Verdict::Keep("tip is origin/main's tip")
        );
        assert!(plan_actions(&repo).is_empty());
    }

    #[test]
    fn prunes_a_clean_landed_worktree_without_force() {
        let mut item = branch("landed", "oldsha");
        item.is_ancestor_of_upstream = true;
        item.worktree = Some(worktree("/herdr/landed", "landed"));
        let repo = facts(vec![item.clone()]);
        assert_eq!(
            classify_branch(&item, &repo),
            Verdict::PruneWorktreeAndBranch
        );
        assert_eq!(
            plan_actions(&repo),
            vec![
                Action::RemoveWorktree {
                    path: "/herdr/landed".to_string()
                },
                Action::DeleteBranch {
                    name: "landed".to_string(),
                    is_force: false
                },
            ]
        );
    }

    #[test]
    fn prunes_a_squash_landed_branch_with_force() {
        let mut item = branch("squashed", "othersha");
        item.is_patch_equivalent_upstream = true;
        item.worktree = Some(worktree("/herdr/squashed", "squashed"));
        let repo = facts(vec![item.clone()]);
        assert_eq!(
            classify_branch(&item, &repo),
            Verdict::PruneWorktreeAndBranch
        );
        assert_eq!(
            plan_actions(&repo),
            vec![
                Action::RemoveWorktree {
                    path: "/herdr/squashed".to_string()
                },
                Action::DeleteBranch {
                    name: "squashed".to_string(),
                    is_force: true
                },
            ]
        );
    }

    #[test]
    fn keeps_a_dirty_landed_worktree() {
        let mut item = branch("landed", "oldsha");
        item.is_ancestor_of_upstream = true;
        item.worktree = Some(worktree("/herdr/landed", "landed"));
        item.is_worktree_dirty = true;
        let repo = facts(vec![item.clone()]);
        assert_eq!(
            classify_branch(&item, &repo),
            Verdict::Keep("worktree has uncommitted work")
        );
    }

    #[test]
    fn keeps_a_locked_worktree() {
        let mut item = branch("landed", "oldsha");
        item.is_ancestor_of_upstream = true;
        let mut held = worktree("/herdr/landed", "landed");
        held.is_locked = true;
        item.worktree = Some(held);
        let repo = facts(vec![item.clone()]);
        assert_eq!(
            classify_branch(&item, &repo),
            Verdict::Keep("worktree is locked")
        );
    }

    #[test]
    fn keeps_the_primary_worktree() {
        let mut item = branch("landed", "oldsha");
        item.is_ancestor_of_upstream = true;
        let mut primary = worktree("/repo", "landed");
        primary.is_primary = true;
        item.worktree = Some(primary);
        let repo = facts(vec![item.clone()]);
        assert_eq!(
            classify_branch(&item, &repo),
            Verdict::Keep("worktree is the primary checkout")
        );
    }

    #[test]
    fn keeps_the_worktree_the_run_was_invoked_from() {
        let mut item = branch("landed", "oldsha");
        item.is_ancestor_of_upstream = true;
        item.worktree = Some(worktree("/herdr/landed", "landed"));
        let mut repo = facts(vec![item.clone()]);
        repo.invoked_from = "/herdr/landed/tools/worktree-hygiene".to_string();
        assert_eq!(
            classify_branch(&item, &repo),
            Verdict::Keep("this run was invoked from that worktree")
        );
        repo.invoked_from = "/herdr/landed-other".to_string();
        assert_eq!(
            classify_branch(&item, &repo),
            Verdict::PruneWorktreeAndBranch
        );
    }

    #[test]
    fn keeps_main() {
        let mut item = branch(MAIN, "oldsha");
        item.is_ancestor_of_upstream = true;
        let repo = facts(vec![item.clone()]);
        assert_eq!(
            classify_branch(&item, &repo),
            Verdict::Keep("the main branch")
        );
    }

    #[test]
    fn keeps_a_branch_with_no_landed_signal() {
        let item = branch("in-flight", "oldsha");
        let repo = facts(vec![item.clone()]);
        assert_eq!(
            classify_branch(&item, &repo),
            Verdict::Keep("no sign the work landed on origin/main")
        );
    }

    #[test]
    fn prunes_a_landed_branch_that_has_no_worktree() {
        let mut item = branch("worktree/calm-cloud-ee09", "oldsha");
        item.is_ancestor_of_upstream = true;
        let repo = facts(vec![item.clone()]);
        assert_eq!(classify_branch(&item, &repo), Verdict::PruneBranchOnly);
        assert_eq!(
            plan_actions(&repo),
            vec![Action::DeleteBranch {
                name: "worktree/calm-cloud-ee09".to_string(),
                is_force: false
            }]
        );
    }

    #[test]
    fn fast_forward_plans() {
        let mut repo = facts(vec![]);
        assert_eq!(plan_fast_forward(&repo), None);

        repo.main_tip = Some("behindsha".to_string());
        assert_eq!(plan_fast_forward(&repo), Some(Action::FastForwardMainRef));

        let mut holder = worktree("/repo", MAIN);
        holder.is_primary = true;
        repo.main_worktree = Some(holder);
        assert_eq!(
            plan_fast_forward(&repo),
            Some(Action::FastForwardMainInWorktree {
                path: "/repo".to_string()
            })
        );

        repo.is_main_worktree_dirty = true;
        assert_eq!(plan_fast_forward(&repo), None);

        repo.is_main_worktree_dirty = false;
        repo.is_main_ancestor_of_upstream = false;
        assert_eq!(plan_fast_forward(&repo), None);
    }

    #[test]
    fn cherry_output_marks_landed_and_unlanded() {
        assert!(is_cherry_fully_landed("- aaa\n- bbb\n"));
        assert!(!is_cherry_fully_landed("- aaa\n+ bbb\n"));
        assert!(!is_cherry_fully_landed("+ aaa\n"));
        assert!(!is_cherry_fully_landed(""));
    }

    #[test]
    fn agnt_inv_002_oracle_for_the_three_relied_on_git_semantics() {
        let root = scratch_root();
        let repo = build_fixture(&root);

        let porcelain = run(&repo, &["worktree", "list", "--porcelain"]);
        assert!(porcelain.contains("branch refs/heads/main"));
        let parsed = parse_worktree_list(&porcelain);
        assert_eq!(parsed[0].path, repo.display().to_string());
        assert!(parsed[0].is_primary);
        assert!(parsed
            .iter()
            .any(|item| item.is_locked && item.branch.as_deref() == Some("locked-landed")));

        let landed_tip = rev_parse(&repo, "landed-ff").expect("landed-ff");
        let unlanded_tip = rev_parse(&repo, "unlanded").expect("unlanded");
        let upstream_tip = rev_parse(&repo, UPSTREAM).expect("origin/main");
        assert!(git_ok(
            &repo,
            &["merge-base", "--is-ancestor", &landed_tip, &upstream_tip]
        ));
        assert!(!git_ok(
            &repo,
            &["merge-base", "--is-ancestor", &unlanded_tip, &upstream_tip]
        ));

        assert!(is_cherry_fully_landed(&run(
            &repo,
            &["cherry", UPSTREAM, "landed-squash"]
        )));
        assert!(!is_cherry_fully_landed(&run(
            &repo,
            &["cherry", UPSTREAM, "unlanded"]
        )));

        run(&repo, &["worktree", "unlock", &path_of(&root, "wt-locked")]);
        fs::remove_dir_all(&root).expect("cleanup");
    }

    #[test]
    fn plans_and_applies_against_a_real_repository() {
        let root = scratch_root();
        let repo = build_fixture(&root);
        let gathered = gather(&repo, "/nowhere").expect("gather");
        let actions = plan_actions(&gathered);
        let described: Vec<String> = actions.iter().map(describe).collect();

        assert!(described.contains(&format!("fast-forward main in {}", repo.display())));
        assert!(described.contains(&"delete branch landed-ff".to_string()));
        assert!(described.contains(&"delete branch landed-squash (force)".to_string()));
        assert!(described.contains(&"delete branch stale-landed".to_string()));
        assert!(described.contains(&format!(
            "remove worktree {}",
            path_of(&root, "wt-landed-ff")
        )));
        assert!(!described.iter().any(|line| line.contains("unlanded")));
        assert!(!described.iter().any(|line| line.contains("fresh")));
        assert!(!described.iter().any(|line| line.contains("dirty-landed")));
        assert!(!described.iter().any(|line| line.contains("locked-landed")));
        assert!(!described.iter().any(|line| line == "delete branch main"));

        for action in &actions {
            apply(&repo, action, &gathered.upstream_tip).expect("apply");
        }

        assert_eq!(rev_parse(&repo, MAIN), Some(gathered.upstream_tip.clone()));
        assert!(rev_parse(&repo, "refs/heads/landed-ff").is_none());
        assert!(rev_parse(&repo, "refs/heads/landed-squash").is_none());
        assert!(rev_parse(&repo, "refs/heads/stale-landed").is_none());
        assert!(rev_parse(&repo, "refs/heads/unlanded").is_some());
        assert!(rev_parse(&repo, "refs/heads/fresh").is_some());
        assert!(rev_parse(&repo, "refs/heads/dirty-landed").is_some());
        assert!(rev_parse(&repo, "refs/heads/locked-landed").is_some());
        assert!(!root.join("wt-landed-ff").exists());
        assert!(root.join("wt-dirty").exists());
        assert!(root.join("wt-locked").exists());
        assert!(root.join("wt-fresh").exists());

        run(&repo, &["worktree", "unlock", &path_of(&root, "wt-locked")]);
        fs::remove_dir_all(&root).expect("cleanup");
    }

    fn build_fixture(root: &Path) -> PathBuf {
        let remote = root.join("origin.git");
        let status = Command::new("git")
            .args(["init", "--quiet", "--bare", "-b", "main"])
            .arg(&remote)
            .status()
            .expect("git init bare");
        assert!(status.success());

        let repo = root.join("repo");
        let status = Command::new("git")
            .args(["init", "--quiet", "-b", "main"])
            .arg(&repo)
            .status()
            .expect("git init");
        assert!(status.success());
        run(&repo, &["config", "user.email", "test@example.invalid"]);
        run(&repo, &["config", "user.name", "hygiene test"]);
        run(&repo, &["config", "commit.gpgsign", "false"]);
        commit(&repo, "base.txt", "base\n", "base");
        run(
            &repo,
            &["remote", "add", "origin", &remote.display().to_string()],
        );
        run(&repo, &["push", "-q", "-u", "origin", "main"]);

        run(&repo, &["checkout", "-q", "-b", "landed-ff"]);
        commit(&repo, "ff.txt", "ff\n", "ff work");
        run(&repo, &["checkout", "-q", "main"]);
        run(&repo, &["merge", "-q", "--ff-only", "landed-ff"]);

        run(&repo, &["checkout", "-q", "-b", "landed-squash"]);
        commit(&repo, "squash.txt", "squash\n", "squash work");
        run(&repo, &["checkout", "-q", "main"]);
        commit(&repo, "squash.txt", "squash\n", "squash work (#1)");

        run(&repo, &["checkout", "-q", "-b", "unlanded"]);
        commit(&repo, "wip.txt", "wip\n", "wip");
        run(&repo, &["checkout", "-q", "main"]);

        for name in ["dirty-landed", "locked-landed", "stale-landed"] {
            run(&repo, &["branch", name, "landed-ff"]);
        }

        commit(&repo, "tip.txt", "tip\n", "tip commit");
        run(&repo, &["push", "-q", "origin", "main"]);
        run(&repo, &["branch", "fresh"]);
        run(&repo, &["reset", "-q", "--hard", "HEAD~1"]);

        for (dir, name) in [
            ("wt-landed-ff", "landed-ff"),
            ("wt-landed-squash", "landed-squash"),
            ("wt-unlanded", "unlanded"),
            ("wt-fresh", "fresh"),
            ("wt-dirty", "dirty-landed"),
            ("wt-locked", "locked-landed"),
        ] {
            run(&repo, &["worktree", "add", "-q", &path_of(root, dir), name]);
        }
        fs::write(root.join("wt-dirty").join("scratch.txt"), "dirty\n").expect("dirty file");
        run(&repo, &["worktree", "lock", &path_of(root, "wt-locked")]);
        for dir in [
            "wt-landed-ff",
            "wt-landed-squash",
            "wt-unlanded",
            "wt-fresh",
            "wt-dirty",
            "wt-locked",
        ] {
            backdate(&root.join(dir));
        }

        repo
    }
}
