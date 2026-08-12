use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::error::SyncError;

/// Reads a file to a string, mapping absence to None.
/// Takes the path; returns the contents or None when the file does not exist.
///
/// # Errors
/// Io on any read failure other than not-found.
pub fn read_opt(path: &Path) -> Result<Option<String>, SyncError> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(SyncError::Io(path.to_path_buf(), err)),
    }
}

/// Copies the target to a stamped backup and verifies the copy is on disk.
/// Takes the path; returns the backup path, or None when the target is absent.
///
/// # Errors
/// BackupFailed when the copy is missing after the write; Io on read failure.
pub fn backup(path: &Path) -> Result<Option<PathBuf>, SyncError> {
    match path.symlink_metadata() {
        Ok(_) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(SyncError::Io(path.to_path_buf(), err)),
    }
    let (date, time) = stamps();
    let mut dest = suffixed(path, &format!(".pre-sync-{date}"));
    if dest.symlink_metadata().is_ok() {
        dest = suffixed(&dest, &format!(".{time}"));
    }
    fs::copy(path, &dest).map_err(|err| SyncError::Io(path.to_path_buf(), err))?;
    if dest.symlink_metadata().is_err() {
        return Err(SyncError::BackupFailed(dest));
    }
    Ok(Some(dest))
}

/// Writes content atomically after proving the target is unchanged since read.
/// Compares the target against the snapshot taken at read time, backs it up,
/// writes a temp file in the same dir, renames it in, and re-reads to verify.
/// Takes the path, the new content, the read-time snapshot, and the dry-run
/// flag; returns unit. In dry-run it prints the would-write line and touches
/// nothing.
///
/// # Errors
/// ChangedSinceRead when the target no longer matches the snapshot;
/// BackupFailed from the backup step; VerifyFailed when the re-read differs;
/// Io on any filesystem failure.
pub fn write_verified(
    path: &Path,
    content: &str,
    snapshot: Option<&str>,
    is_dry_run: bool,
) -> Result<(), SyncError> {
    let current = read_opt(path)?;
    if current.as_deref() != snapshot {
        return Err(SyncError::ChangedSinceRead(path.to_path_buf()));
    }
    if is_dry_run {
        println!("dry:  write {}", path.display());
        return Ok(());
    }
    sweep_stale_tmp(path);
    backup(path)?;
    let tmp = suffixed(path, &unique_tmp_suffix());
    fs::write(&tmp, content).map_err(|err| SyncError::Io(tmp.clone(), err))?;
    fs::rename(&tmp, path).map_err(|err| SyncError::Io(path.to_path_buf(), err))?;
    let reread =
        fs::read_to_string(path).map_err(|err| SyncError::Io(path.to_path_buf(), err))?;
    if reread != content {
        return Err(SyncError::VerifyFailed(
            path.to_path_buf(),
            "re-read does not match written content".to_string(),
        ));
    }
    Ok(())
}

fn suffixed(path: &Path, suffix: &str) -> PathBuf {
    let mut os = path.as_os_str().to_os_string();
    os.push(suffix);
    PathBuf::from(os)
}

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn unique_tmp_suffix() -> String {
    let n = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!(".{}.{n}.sync-tmp", std::process::id())
}

// 60s: a live concurrent writer's own temp file is milliseconds-fresh, so
// this threshold never races it (that race is the D-10 collision the unique
// per-process suffix already closed); only a crash-orphaned temp, which is
// necessarily old by the time a later run sweeps it, ever crosses it.
const STALE_TMP_AGE: Duration = Duration::from_secs(60);

fn sweep_stale_tmp(path: &Path) {
    let dir = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    let Some(basename) = path.file_name().and_then(|name| name.to_str()) else {
        return;
    };
    let prefix = format!("{basename}.");
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.starts_with(&prefix) || !name.ends_with(".sync-tmp") {
            continue;
        }
        let is_stale = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| SystemTime::now().duration_since(modified).ok())
            .is_some_and(|age| age >= STALE_TMP_AGE);
        if is_stale {
            let _ = fs::remove_file(entry.path());
        }
    }
}

fn stamps() -> (String, String) {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before 1970")
        .as_secs() as i64;
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    let day = secs.rem_euclid(86_400);
    (
        format!("{y:04}{m:02}{d:02}"),
        format!("{:02}{:02}{:02}", day / 3_600, day % 3_600 / 60, day % 60),
    )
}

// Hinnant's civil_from_days (howardhinnant.github.io/date_algorithms.html)
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "mcp-sync-fsio-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create fixture dir");
        dir
    }

    #[test]
    fn read_opt_and_backup_treat_absent_as_none() {
        let dir = fixture("absent");
        let missing = dir.join("missing.toml");
        assert!(matches!(read_opt(&missing), Ok(None)));
        assert!(matches!(backup(&missing), Ok(None)));
    }

    #[test]
    fn backup_makes_verified_stamped_copy_and_suffixes_collisions() {
        let dir = fixture("backup");
        let target = dir.join("config.toml");
        fs::write(&target, "alpha").expect("seed target");
        let first = backup(&target).ok().flatten().expect("first backup path");
        assert!(first
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("config.toml.pre-sync-"));
        assert_eq!(fs::read_to_string(&first).unwrap(), "alpha");
        let second = backup(&target).ok().flatten().expect("second backup path");
        assert_ne!(first, second);
        assert_eq!(fs::read_to_string(&second).unwrap(), "alpha");
    }

    #[test]
    fn write_verified_rejects_snapshot_mismatch() {
        let dir = fixture("cas");
        let target = dir.join("config.toml");
        fs::write(&target, "current").expect("seed target");
        let stale = write_verified(&target, "next", Some("stale"), false);
        assert!(matches!(stale, Err(SyncError::ChangedSinceRead(_))));
        assert_eq!(fs::read_to_string(&target).unwrap(), "current");
        let absent = dir.join("gone.toml");
        let gone = write_verified(&absent, "next", Some("stale"), false);
        assert!(matches!(gone, Err(SyncError::ChangedSinceRead(_))));
    }

    #[test]
    fn write_verified_replaces_content_and_leaves_backup() {
        let dir = fixture("write");
        let target = dir.join("config.toml");
        fs::write(&target, "old").expect("seed target");
        assert!(write_verified(&target, "new", Some("old"), false).is_ok());
        assert_eq!(fs::read_to_string(&target).unwrap(), "new");
        let names: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names.len(), 2);
        assert!(names
            .iter()
            .any(|name| name.starts_with("config.toml.pre-sync-")));
        assert!(!names.iter().any(|name| name.ends_with(".sync-tmp")));
        let created = dir.join("fresh.toml");
        assert!(write_verified(&created, "seed", None, false).is_ok());
        assert_eq!(fs::read_to_string(&created).unwrap(), "seed");
        assert!(!dir.join("fresh.toml.sync-tmp").exists());
    }

    #[test]
    fn dry_run_prints_and_touches_nothing() {
        let dir = fixture("dry");
        let target = dir.join("config.toml");
        fs::write(&target, "old").expect("seed target");
        assert!(write_verified(&target, "new", Some("old"), true).is_ok());
        assert_eq!(fs::read_to_string(&target).unwrap(), "old");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
    }

    #[test]
    fn civil_from_days_matches_known_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_677), (2026, 8, 12));
    }

    #[test]
    fn unique_tmp_suffix_differs_across_calls_and_embeds_pid() {
        let pid = std::process::id().to_string();
        let first = unique_tmp_suffix();
        let second = unique_tmp_suffix();
        assert_ne!(first, second);
        assert!(first.contains(&pid));
        assert!(second.contains(&pid));
        assert!(first.ends_with(".sync-tmp"));
        assert!(second.ends_with(".sync-tmp"));
    }

    #[test]
    fn write_verified_leaves_no_sync_tmp_residue() {
        let dir = fixture("no-residue");
        let target = dir.join("config.toml");
        fs::write(&target, "old").expect("seed target");
        assert!(write_verified(&target, "new", Some("old"), false).is_ok());
        let leftover: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(".sync-tmp"))
            .collect();
        assert!(leftover.is_empty(), "leftover tmp files: {leftover:?}");
    }

    #[test]
    fn write_verified_sweeps_stale_tmp_but_keeps_fresh_tmp() {
        let dir = fixture("sweep");
        let target = dir.join("config.toml");
        fs::write(&target, "old").expect("seed target");

        let stale = suffixed(&target, ".9999.5.sync-tmp");
        fs::write(&stale, "orphaned").expect("seed stale tmp");
        let backdated = SystemTime::now() - Duration::from_secs(61);
        fs::File::open(&stale)
            .and_then(|file| file.set_modified(backdated))
            .expect("backdate stale tmp");

        let fresh = suffixed(&target, ".9999.6.sync-tmp");
        fs::write(&fresh, "in-flight").expect("seed fresh tmp");

        assert!(write_verified(&target, "new", Some("old"), false).is_ok());

        assert!(!stale.exists(), "stale sync-tmp should be swept");
        assert!(fresh.exists(), "fresh sync-tmp should survive the sweep");
    }
}
