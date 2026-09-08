use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

fn is_process_finished(pid: &str) -> bool {
    let output = Command::new("ps")
        .args(["-o", "stat=", "-p", pid])
        .output()
        .unwrap();
    !output.status.success()
        || String::from_utf8_lossy(&output.stdout)
            .trim()
            .starts_with('Z')
}

#[test]
fn terminal_signal_reaps_the_child_group() {
    let dir =
        std::env::temp_dir().join(format!("tier-dispatch-signal-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let tiers = dir.join("tiers.json");
    let prompt = dir.join("prompt.md");
    let pid_file = dir.join("pids");
    let script = dir.join("provider.sh");
    std::fs::write(
        &tiers,
        r#"{"tiers":{"T1":{"pi":{"model":"test/model","thinking":"low"},"fallbacks":[]}},"orchestrator":"T1"}"#,
    )
    .unwrap();
    std::fs::write(&prompt, "prompt\n").unwrap();
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nsh -c 'trap \"\" TERM; sleep 30' >/dev/null 2>&1 &\necho \"$$ $!\" > {}\nwait\n",
            pid_file.display()
        ),
    )
    .unwrap();
    let mut permissions = std::fs::metadata(&script).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&script, permissions).unwrap();

    let mut dispatcher = Command::new(env!("CARGO_BIN_EXE_tier-dispatch"))
        .args([
            "--tiers-file",
            tiers.to_str().unwrap(),
            "--tier",
            "T1",
            "--system-prompt-file",
            prompt.to_str().unwrap(),
            "--input",
            "test",
            "--dispatch-bin",
            script.to_str().unwrap(),
        ])
        .spawn()
        .unwrap();
    let started = Instant::now();
    let pids = loop {
        if let Ok(contents) = std::fs::read_to_string(&pid_file) {
            let pids: Vec<_> = contents
                .split_whitespace()
                .filter_map(|pid| pid.parse::<i32>().ok())
                .map(|pid| pid.to_string())
                .collect();
            if pids.len() == 2 {
                break pids;
            }
        }
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "provider did not write two process identifiers within five seconds"
        );
        thread::sleep(Duration::from_millis(50));
    };
    Command::new("kill")
        .args(["-TERM", &dispatcher.id().to_string()])
        .status()
        .unwrap();
    let status = dispatcher.wait().unwrap();
    assert_eq!(status.code(), Some(130));
    let finished = Instant::now();
    while !pids.iter().all(|pid| is_process_finished(pid))
        && finished.elapsed() < Duration::from_secs(5)
    {
        thread::sleep(Duration::from_millis(50));
    }
    assert!(pids.iter().all(|pid| is_process_finished(pid)));
    std::fs::remove_dir_all(dir).ok();
}
