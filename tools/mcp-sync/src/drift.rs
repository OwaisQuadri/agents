pub enum Tool {
    Claude,
    Codex,
}

pub enum DriftState {
    Ok,
    Missing,
    Drifted,
    Unmanaged,
}

pub struct DriftRow {
    pub server: String,
    pub tool: Tool,
    pub state: DriftState,
}

pub enum ChangeKind {
    Add,
    Update,
    Remove,
}

pub struct Change {
    pub tool: Tool,
    pub server: String,
    pub kind: ChangeKind,
}

/// Renders changes as plan lines in install.sh's line language.
/// Takes the changes and the dry-run flag; returns the printable text, one
/// line per change, dry: prefixed under dry-run and plan: otherwise.
///
/// # Errors
/// none
pub fn render_plan(changes: &[Change], is_dry_run: bool) -> String {
    let _ = (changes, is_dry_run);
    // TODO(AGNT-0001.T07): body lands in the phase-13 build; contract: interfaces.md drift section
    unimplemented!()
}

/// Renders drift rows as the one-screen check readout.
/// Takes the rows; returns the readout text grouped by server, one state
/// word per tool.
///
/// # Errors
/// none
pub fn render_check(rows: &[DriftRow]) -> String {
    let _ = rows;
    // TODO(AGNT-0001.T07): body lands in the phase-13 build; contract: interfaces.md drift section
    unimplemented!()
}

/// Reports whether any row is not Ok.
/// Takes the rows; returns true on any Missing, Drifted, or Unmanaged row.
///
/// # Errors
/// none
pub fn has_drift(rows: &[DriftRow]) -> bool {
    let _ = rows;
    // TODO(AGNT-0001.T07): body lands in the phase-13 build; contract: interfaces.md drift section
    unimplemented!()
}
