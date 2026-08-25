use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Serialize, Deserialize)]
pub struct Baseline {
    pub schema_version: u32,
    pub captured_at: String,
    pub repo: String,
    pub entries: BTreeMap<String, String>,
    pub refs: BTreeMap<String, String>,
}

/// Captures the porcelain status and the ref hashes of a repository.
///
/// Takes the path of the repository to stamp. Returns the baseline, whose entries map
/// each dirty path to its two-character status code followed by the content hash of the
/// file, and whose refs map `HEAD`, the current branch ref, and `merge-base` to their
/// hashes.
///
/// The content hash is what makes an edit to an ALREADY-DIRTY file visible. Git reports
/// the same `??` for an untracked file whatever its bytes, and untracked is the normal
/// state of a work product, so a status code alone lets a verifier rewrite the very file
/// it was sent to grade and report a clean delta.
///
/// # Errors
///
/// Returns the git or date invocation's stderr when the repository does not resolve or a
/// command fails.
pub fn capture(repo: &Path) -> Result<Baseline, String> {
    let root = git(repo, &["rev-parse", "--show-toplevel"])?;
    let root_path = Path::new(&root);
    let entries = status_entries(root_path)?;
    let refs = refs(root_path)?;
    Ok(Baseline {
        schema_version: SCHEMA_VERSION,
        captured_at: local_timestamp()?,
        repo: root,
        entries,
        refs,
    })
}

fn status_entries(repo: &Path) -> Result<BTreeMap<String, String>, String> {
    let raw = git(repo, &["status", "--porcelain", "-z"])?;
    let mut fields = raw.split('\0').filter(|field| !field.is_empty());
    let mut entries = BTreeMap::new();
    while let Some(field) = fields.next() {
        let (code, path) = match field.split_at_checked(3) {
            Some((code, path)) => (code[..2].to_string(), path.to_string()),
            None => continue,
        };
        let is_rename = code.starts_with('R') || code.starts_with('C');
        if is_rename {
            if let Some(origin) = fields.next() {
                let stamp = format!("{code} {}", content_hash(repo, origin));
                entries.insert(origin.to_string(), stamp);
            }
        }
        let stamp = format!("{code} {}", content_hash(repo, &path));
        entries.insert(path, stamp);
    }
    Ok(entries)
}

fn content_hash(repo: &Path, path: &str) -> String {
    let full = repo.join(path);
    match std::fs::read(&full) {
        Ok(bytes) => {
            let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
            for byte in bytes {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(0x1000_0000_01b3);
            }
            format!("{hash:016x}")
        }
        Err(_) => "absent".to_string(),
    }
}

fn refs(repo: &Path) -> Result<BTreeMap<String, String>, String> {
    let mut refs = BTreeMap::new();
    if let Ok(head) = git(repo, &["rev-parse", "HEAD"]) {
        refs.insert("HEAD".to_string(), head);
    }
    if let Ok(branch) = git(repo, &["symbolic-ref", "--quiet", "HEAD"]) {
        if let Ok(tip) = git(repo, &["rev-parse", &branch]) {
            refs.insert(branch, tip);
        }
    }
    if let Some(default_branch) = default_branch(repo) {
        if let Ok(base) = git(repo, &["merge-base", "HEAD", &default_branch]) {
            refs.insert(format!("merge-base:{default_branch}"), base);
        }
    }
    Ok(refs)
}

fn default_branch(repo: &Path) -> Option<String> {
    if let Ok(head) = git(
        repo,
        &["symbolic-ref", "--quiet", "refs/remotes/origin/HEAD"],
    ) {
        return Some(head);
    }
    ["refs/heads/main", "refs/heads/master"]
        .into_iter()
        .find(|candidate| git(repo, &["rev-parse", "--verify", "--quiet", candidate]).is_ok())
        .map(str::to_string)
}

fn local_timestamp() -> Result<String, String> {
    run("date", &["+%Y-%m-%dT%H:%M:%S%z"])
}

fn git(repo: &Path, args: &[&str]) -> Result<String, String> {
    let mut full = vec!["-C", repo.to_str().ok_or("repo path is not utf-8")?];
    full.extend_from_slice(args);
    run("git", &full)
}

fn run(program: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|error| format!("{program}: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "{program} {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .trim_end_matches('\n')
        .to_string())
}
