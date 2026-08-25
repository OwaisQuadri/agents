use std::collections::BTreeMap;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
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
    let raw = git_bytes(
        repo,
        &["status", "--porcelain", "-z", "--untracked-files=all"],
    )?;
    let mut fields = raw
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty());
    let mut entries = BTreeMap::new();
    while let Some(field) = fields.next() {
        if field.len() < 3 {
            continue;
        }
        let code = std::str::from_utf8(&field[..2])
            .map_err(|_| "git status returned a non-UTF-8 status code".to_string())?;
        let path = git_path(&field[3..])?;
        let is_rename = code.starts_with('R') || code.starts_with('C');
        if is_rename {
            if let Some(origin) = fields.next() {
                let origin = git_path(origin)?;
                let stamp = format!(
                    "{code} {} {}",
                    content_hash(repo, origin),
                    index_hash(repo, origin)
                );
                entries.insert(origin.to_string(), stamp);
            }
        }
        let stamp = format!(
            "{code} {} {}",
            content_hash(repo, path),
            index_hash(repo, path)
        );
        entries.insert(path.to_string(), stamp);
    }
    for path in ignored_files(repo)? {
        entries
            .entry(path.clone())
            .or_insert_with(|| format!("!! {} none", content_hash(repo, &path)));
    }
    Ok(entries)
}

fn git_path(path: &[u8]) -> Result<&str, String> {
    std::str::from_utf8(path).map_err(|_| {
        "non-UTF-8 git paths are unsupported; refusing an incomplete stamp".to_string()
    })
}

fn ignored_files(repo: &Path) -> Result<Vec<String>, String> {
    let raw = git_bytes(
        repo,
        &[
            "ls-files",
            "--others",
            "--ignored",
            "--exclude-standard",
            "-z",
        ],
    )?;
    raw.split(|byte| *byte == 0)
        .filter(|field| !field.is_empty())
        .map(|path| git_path(path).map(str::to_string))
        .collect()
}

fn index_hash(repo: &Path, path: &str) -> String {
    git(repo, &["ls-files", "-s", "--", path])
        .ok()
        .and_then(|line| line.split_whitespace().nth(1).map(str::to_string))
        .unwrap_or_else(|| "none".to_string())
}

fn content_hash(repo: &Path, path: &str) -> String {
    let full = repo.join(path);
    let content = match std::fs::symlink_metadata(&full) {
        Ok(metadata) if metadata.file_type().is_symlink() => std::fs::read_link(&full)
            .map(|target| (b'L', 0, target.as_os_str().as_encoded_bytes().to_vec())),
        Ok(metadata) => std::fs::read(&full).map(|bytes| (b'F', file_mode(&metadata), bytes)),
        Err(error) => return format!("absent:{error}"),
    };
    match content {
        Ok((kind, mode, bytes)) => {
            let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
            for byte in std::iter::once(kind).chain(mode.to_le_bytes()).chain(bytes) {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(0x1000_0000_01b3);
            }
            format!("{hash:016x}")
        }
        Err(error) => format!("unreadable:{error}"),
    }
}

#[cfg(unix)]
fn file_mode(metadata: &std::fs::Metadata) -> u32 {
    metadata.permissions().mode()
}

#[cfg(not(unix))]
fn file_mode(metadata: &std::fs::Metadata) -> u32 {
    u32::from(metadata.permissions().readonly())
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

fn git_bytes(repo: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let mut full = vec!["-C", repo.to_str().ok_or("repo path is not utf-8")?];
    full.extend_from_slice(args);
    let output = Command::new("git")
        .args(&full)
        .output()
        .map_err(|error| format!("git: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "git {}: {}",
            full.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(output.stdout)
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
