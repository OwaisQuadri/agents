// TODO(AGNT-0002.T06): Verify spared reporting and drift states.
pub enum Tool {
    Claude,
    Codex,
}

pub enum DriftState {
    Ok,
    Missing,
    Drifted,
    Spared,
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
    Spare,
}

pub struct Change {
    pub tool: Tool,
    pub server: String,
    pub kind: ChangeKind,
}

impl Tool {
    fn word(&self) -> &'static str {
        match self {
            Tool::Claude => "claude",
            Tool::Codex => "codex",
        }
    }
}

impl DriftState {
    fn word(&self) -> &'static str {
        match self {
            DriftState::Ok => "ok",
            DriftState::Missing => "missing",
            DriftState::Drifted => "drifted",
            DriftState::Spared => "spared",
            DriftState::Unmanaged => "unmanaged",
        }
    }
}

impl ChangeKind {
    fn word(&self) -> &'static str {
        match self {
            ChangeKind::Add => "add",
            ChangeKind::Update => "update",
            ChangeKind::Remove => "remove",
            ChangeKind::Spare => "spare",
        }
    }
}

/// Renders changes as plan lines in install.sh's line language.
/// Takes the changes and the dry-run flag; returns the printable text, one
/// line per change, dry: prefixed under dry-run and plan: otherwise.
pub fn render_plan(changes: &[Change], is_dry_run: bool) -> String {
    let prefix = if is_dry_run { "dry:  " } else { "plan: " };
    changes
        .iter()
        .map(|change| {
            format!(
                "{prefix}{} {} {}\n",
                change.tool.word(),
                change.kind.word(),
                change.server
            )
        })
        .collect()
}

/// Renders drift rows as the one-screen check readout.
/// Takes the rows; returns the readout text grouped by server, one state
/// word per tool.
pub fn render_check(rows: &[DriftRow]) -> String {
    let mut seen: Vec<&str> = Vec::new();
    let mut out = String::new();
    for row in rows {
        if seen.contains(&row.server.as_str()) {
            continue;
        }
        seen.push(&row.server);
        let claude = rows
            .iter()
            .find(|r| r.server == row.server && matches!(r.tool, Tool::Claude));
        let codex = rows
            .iter()
            .find(|r| r.server == row.server && matches!(r.tool, Tool::Codex));
        out.push_str(&row.server);
        out.push(' ');
        for pair in [claude, codex].into_iter().flatten() {
            out.push(' ');
            out.push_str(pair.tool.word());
            out.push('=');
            out.push_str(pair.state.word());
        }
        out.push('\n');
    }
    out
}

/// Reports whether any row is not Ok.
/// Takes the rows; returns true on any Missing, Drifted, or Unmanaged row.
pub fn is_drift_present(rows: &[DriftRow]) -> bool {
    rows.iter().any(|row| !matches!(row.state, DriftState::Ok))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(tool: Tool, kind: ChangeKind, server: &str) -> Change {
        Change {
            tool,
            server: server.to_string(),
            kind,
        }
    }

    fn row(server: &str, tool: Tool, state: DriftState) -> DriftRow {
        DriftRow {
            server: server.to_string(),
            tool,
            state,
        }
    }

    fn mixed_changes() -> Vec<Change> {
        vec![
            change(Tool::Claude, ChangeKind::Add, "supermemory"),
            change(Tool::Codex, ChangeKind::Update, "mobbin"),
            change(Tool::Claude, ChangeKind::Remove, "linear"),
        ]
    }

    fn mixed_rows() -> Vec<DriftRow> {
        vec![
            row("supermemory", Tool::Claude, DriftState::Ok),
            row("mobbin", Tool::Claude, DriftState::Drifted),
            row("scratchpad", Tool::Claude, DriftState::Unmanaged),
            row("supermemory", Tool::Codex, DriftState::Missing),
            row("mobbin", Tool::Codex, DriftState::Ok),
            row("apollo", Tool::Codex, DriftState::Ok),
        ]
    }

    #[test]
    fn plan_lines_for_a_mixed_change_set() {
        assert_eq!(
            render_plan(&mixed_changes(), false),
            "plan: claude add supermemory\n\
             plan: codex update mobbin\n\
             plan: claude remove linear\n"
        );
    }

    #[test]
    fn dry_lines_align_with_plan_lines() {
        assert_eq!(
            render_plan(&mixed_changes(), true),
            "dry:  claude add supermemory\n\
             dry:  codex update mobbin\n\
             dry:  claude remove linear\n"
        );
    }

    #[test]
    fn spare_action_uses_existing_plan_language() {
        let changes = [change(Tool::Codex, ChangeKind::Spare, "linear")];

        assert_eq!(render_plan(&changes, false), "plan: codex spare linear\n");
        assert_eq!(render_plan(&changes, true), "dry:  codex spare linear\n");
    }

    #[test]
    fn check_screen_for_a_mixed_drift_set() {
        assert_eq!(
            render_check(&mixed_rows()),
            "supermemory  claude=ok codex=missing\n\
             mobbin  claude=drifted codex=ok\n\
             scratchpad  claude=unmanaged\n\
             apollo  codex=ok\n"
        );
    }

    #[test]
    fn check_screen_exposes_spared_state_per_tool() {
        let rows = [
            row("linear", Tool::Claude, DriftState::Spared),
            row("linear", Tool::Codex, DriftState::Ok),
        ];

        assert_eq!(render_check(&rows), "linear  claude=spared codex=ok\n");
    }

    #[test]
    fn is_drift_present_is_true_iff_any_non_ok_row() {
        assert!(is_drift_present(&mixed_rows()));
        assert!(!is_drift_present(&[]));
        assert!(!is_drift_present(&[row(
            "supermemory",
            Tool::Claude,
            DriftState::Ok
        )]));
        assert!(is_drift_present(&[row(
            "supermemory",
            Tool::Codex,
            DriftState::Missing
        )]));
        assert!(is_drift_present(&[row(
            "mobbin",
            Tool::Claude,
            DriftState::Drifted
        )]));
        assert!(is_drift_present(&[row(
            "linear",
            Tool::Claude,
            DriftState::Spared
        )]));
        assert!(is_drift_present(&[row(
            "scratchpad",
            Tool::Codex,
            DriftState::Unmanaged
        )]));
    }
}
