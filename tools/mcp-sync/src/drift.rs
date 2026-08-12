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
