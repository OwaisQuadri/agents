pub struct Manifest {
    pub servers: Vec<ServerEntry>,
}

pub struct ServerEntry {
    pub name: String,
    pub transport: Transport,
    pub scope: ToolScope,
}

pub enum Transport {
    Stdio(StdioSpec),
    Remote(RemoteSpec),
}

pub struct StdioSpec {
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub cwd: Option<String>,
}

pub struct RemoteSpec {
    pub url: String,
    pub bearer_token_env_var: Option<String>,
}

pub enum ToolScope {
    Both,
    ClaudeOnly,
    CodexOnly,
}

pub struct SyncState {
    pub claude_managed: Vec<String>,
    pub codex_managed: Vec<String>,
}
