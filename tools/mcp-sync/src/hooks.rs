use std::path::Path;

use crate::drift::Change;
use crate::error::SyncError;

pub struct HookRegistration {
    pub command: String,
    pub timeout_secs: u64,
}

/// Ensures the Codex hooks file carries the UserPromptSubmit registration.
/// Matches the managed entry by command path, so foreign hook entries
/// survive; creates the file when absent.
/// Takes the hooks.json path, the registration, and the dry-run flag;
/// returns the changes.
///
/// # Errors
/// Io, ParseJson, ChangedSinceRead, BackupFailed, VerifyFailed.
pub fn sync_codex_hook(
    path: &Path,
    reg: &HookRegistration,
    is_dry_run: bool,
) -> Result<Vec<Change>, SyncError> {
    let _ = (path, reg, is_dry_run);
    // TODO(AGNT-0001.T06): body lands in the phase-13 build; contract: interfaces.md hooks section
    unimplemented!()
}
