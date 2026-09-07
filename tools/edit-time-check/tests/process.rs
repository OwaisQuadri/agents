use edit_time_check::{Decision, Response};
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture(std::path::PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "edit-time-process-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        let path = path.canonicalize().unwrap();
        std::fs::write(path.join("config"), "version=1\nrules=['boolean-name']").unwrap();
        std::fs::write(path.join("rules"), "Boolean names start with is").unwrap();
        Self(path)
    }
    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_edit-time-check"));
        command
            .args(["--config"])
            .arg(self.0.join("config"))
            .args(["--rule-document"])
            .arg(self.0.join("rules"))
            .args(["--code-style"])
            .arg(self.0.join("rules"))
            .args(["--judgment-configuration", "fixture"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command
    }
    fn request(&self, text: &str, budget: u64) -> serde_json::Value {
        serde_json::json!({"version":1,"request_id":"fixture","operation":"write","path":self.0.join("new/deep/x.rs"),"repository_root":null,"original_text":null,"proposed_text":text,"changed_ranges":[],"budget_ms":budget})
    }
    fn run(&self, request: &serde_json::Value) -> (std::process::Output, Response) {
        let mut child = self.command().spawn().unwrap();
        serde_json::to_writer(child.stdin.take().unwrap(), request).unwrap();
        let output = child.wait_with_output().unwrap();
        let response: Response = serde_json::from_slice(&output.stdout).unwrap();
        assert!(response.is_valid());
        assert!(output.stderr.is_empty());
        (output, response)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}
#[test]
fn decision_exit_and_no_filesystem_effects() {
    let fixture = Fixture::new();
    for (text, code, decision) in [
        ("fn main() { let isReady = true; }", 0, Decision::Pass),
        ("fn main() { let ready = true; }", 1, Decision::Block),
    ] {
        let (output, response) = fixture.run(&fixture.request(text, 20_000));
        assert_eq!(output.status.code(), Some(code));
        assert_eq!(response.decision, decision);
        assert_eq!(response.request_id, "fixture");
        assert!(!fixture.0.join("new").exists());
    }
    let mut child = fixture.command().spawn().unwrap();
    child.stdin.take().unwrap().write_all(b"{}").unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(serde_json::from_slice::<Response>(&output.stdout)
        .unwrap()
        .is_valid());
}
#[test]
fn original_request_budget_survives_errors() {
    let fixture = Fixture::new();
    std::fs::write(fixture.0.join("config"), "bad").unwrap();
    let (output, response) = fixture.run(&fixture.request("", 400));
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(response.request_id, "fixture");
    assert_eq!(response.budget_ms, 400);
    assert_eq!(
        serde_json::to_value(&response).unwrap()["diagnostics"][0]["rule"],
        "checker"
    );
}
#[test]
fn missing_end_of_input_is_bounded() {
    let fixture = Fixture::new();
    let start = Instant::now();
    let mut child = fixture.command().spawn().unwrap();
    let input = child.stdin.take().unwrap();
    let output = child.wait_with_output().unwrap();
    drop(input);
    assert_eq!(output.status.code(), Some(2));
    assert!(start.elapsed() < Duration::from_secs(5));
    let response: Response = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response.decision, Decision::Error);
    assert!(response.diagnostics[0].reason.contains("deadline"));
    assert_eq!(
        serde_json::to_value(&response).unwrap()["diagnostics"][0]["rule"],
        "checker"
    );
}
