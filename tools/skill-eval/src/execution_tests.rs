use super::*;

#[test]
fn candidate_input_preserves_absent_and_supplied_repository_paths() {
    let fixture = Path::new("/tmp/execution-unit/fixture");
    for input in [
        r#"{"task":"review","base":"main","head":"feature-auth"}"#.to_string(),
        "Continue in this scratch workspace with native tools.".to_string(),
    ] {
        let actual = attempt_input(&input, fixture, false);
        assert!(actual.starts_with(&input));
        assert!(!actual.contains(&*fixture.to_string_lossy()));
        assert!(!actual.contains("repo_path"));
        assert!(!actual.contains("Execution workspace:"));
        assert!(actual.contains("Evidence files outside the workspace are runner-owned"));
        assert!(actual.contains("inherit_context:false and run_in_background:false"));
    }
    let input = format!(r#"{{"repo_path":"{FIXED_REPO}","source":"{FIXED_REPO}/app.py"}}"#);
    let actual = attempt_input(&input, fixture, false);
    assert!(actual.starts_with(&format!(
        r#"{{"repo_path":"{}","source":"{}/app.py"}}"#,
        fixture.display(),
        fixture.display()
    )));
    assert!(!actual.contains(FIXED_REPO));
    assert!(!actual.contains("Execution workspace:"));
    assert!(actual.contains("Do not commit or land."));
}

#[test]
fn attempt_input_overhead_measurement() {
    let fixture = Path::new("/tmp/execution-unit/fixture");
    let input = format!(r#"{{"repo_path":"{FIXED_REPO}"}}"#);
    for is_judge in [false, true] {
        let started = Instant::now();
        for _ in 0..10_000 {
            std::hint::black_box(attempt_input(
                std::hint::black_box(&input),
                fixture,
                is_judge,
            ));
        }
        eprintln!(
            "attempt_input is_judge={is_judge} samples=10000 mean_ns={:.2}",
            started.elapsed().as_nanos() as f64 / 10_000.0
        );
    }
}

#[test]
fn judge_input_keeps_explicit_source_context_without_a_supplied_path() {
    let fixture = Path::new("/tmp/execution-unit/judge-fixture");
    let actual = attempt_input("Inspect the local fixture source and diff.", fixture, true);
    assert!(actual.contains(&format!("Execution workspace: {}.", fixture.display())));
    assert!(actual.contains("Evidence files outside the workspace are runner-owned"));
}

fn worker_fixture(root: &Path) -> (Vec<Value>, Value) {
    let state = json!({"files": {"cost.mjs": {"hash": "final"}}});
    let mut events = Vec::new();
    for index in 0..2 {
        let id = format!("worker-{index}");
        let call = format!("call-{index}");
        let child = root.join(format!("{index}.jsonl"));
        let entries = [
            json!({"type": "session", "id": id}),
            json!({"message": {"role": "assistant", "content": [{"type": "toolCall", "id": "read-1", "name": "read", "arguments": {"path": "cost.mjs"}}]}}),
            json!({"message": {"role": "toolResult", "toolCallId": "read-1", "content": [{"type": "text", "text": "source"}]}}),
        ];
        fs::write(
            &child,
            entries
                .iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join("\n")
                + "\n",
        )
        .unwrap();
        events.extend([
            json!({"type": "tool_execution_start", "toolCallId": call, "toolName": "Agent", "args": {"inherit_context": false}, "snapshot": state}),
            json!({"type": "worker_started", "id": id, "snapshot": state}),
            json!({"type": "worker_settled", "id": id, "status": "completed", "sessionCopy": child}),
            json!({"type": "tool_execution_end", "toolCallId": call, "toolName": "Agent", "result": {"details": {"agentId": id}}}),
        ]);
    }
    events.push(json!({"type": "recorder_complete"}));
    (events, state)
}

#[test]
fn top_level_workers_use_returned_ids_and_native_sessions() {
    let temp = TempDir::create(&env::temp_dir(), "execution-unit").unwrap();
    let (events, _) = worker_fixture(&temp.path);
    paired_tools(&events).unwrap();
    let children = child_evidence(&events, "root").unwrap();
    assert_eq!(children.len(), 2);
    assert_eq!(children[0]["session_id"], "worker-0");
    assert_eq!(children[1]["dispatch"]["toolCallId"], "call-1");
}

#[test]
fn late_parent_edits_preserve_worker_provenance() {
    let temp = TempDir::create(&env::temp_dir(), "execution-unit").unwrap();
    let (events, mut state) = worker_fixture(&temp.path);
    state["files"]["cost.mjs"]["hash"] = json!("later");
    assert_ne!(events[0]["snapshot"], state);
    assert_eq!(child_evidence(&events, "root").unwrap().len(), 2);
    save(&temp.path.join("before.json"), &events[0]["snapshot"]).unwrap();
    save(&temp.path.join("after.json"), &state).unwrap();
    fs::write(
        temp.path.join("root.jsonl"),
        "{\"type\":\"session\",\"id\":\"root\"}\n",
    )
    .unwrap();
    fs::write(
        temp.path.join("events.jsonl"),
        events
            .iter()
            .map(|event| format!("{event}\n"))
            .collect::<String>(),
    )
    .unwrap();
    let trace = evidence(&temp.path, true, false).unwrap();
    assert_eq!(trace["after"], state);
    assert_eq!(trace["is_mutated"], true);
}

#[test]
fn missing_child_evidence_is_incomplete() {
    let temp = TempDir::create(&env::temp_dir(), "execution-unit").unwrap();
    let (events, _) = worker_fixture(&temp.path);
    fs::write(
        temp.path.join("0.jsonl"),
        "{\"type\":\"session\",\"id\":\"worker-0\"}\n",
    )
    .unwrap();
    assert_eq!(
        child_evidence(&events, "root").unwrap_err(),
        "missing child tool evidence"
    );
    fs::remove_file(temp.path.join("0.jsonl")).unwrap();
    assert!(child_evidence(&events, "root").is_err());
}

#[test]
fn workflows_and_nested_dispatch_are_incomplete() {
    let temp = TempDir::create(&env::temp_dir(), "execution-unit").unwrap();
    let (mut events, _) = worker_fixture(&temp.path);
    let child = temp.path.join("0.jsonl");
    let text = fs::read_to_string(&child)
        .unwrap()
        .replace("\"name\":\"read\"", "\"name\":\"Agent\"");
    fs::write(child, text).unwrap();
    assert_eq!(
        child_evidence(&events, "root").unwrap_err(),
        "unsupported nested child dispatch"
    );
    events[0]["toolName"] = json!("SubagentWorkflow");
    assert_eq!(
        paired_tools(&events).unwrap_err(),
        "unsupported dispatch: SubagentWorkflow"
    );
}

#[test]
fn incomplete_root_tools_and_recorder_errors_are_rejected() {
    let events = vec![
        json!({"type": "tool_execution_start", "toolCallId": "x", "toolName": "read"}),
        json!({"type": "recorder_complete"}),
    ];
    assert_eq!(
        paired_tools(&events).unwrap_err(),
        "missing native tool result"
    );
    assert!(
        paired_tools(&[json!({"type": "recorder_error", "message": "missing session"})]).is_err()
    );
}

#[test]
fn snapshots_measure_file_content_and_git_identity() {
    let temp = TempDir::create(&env::temp_dir(), "execution-unit").unwrap();
    let fixture = temp.path.join("fixture");
    let eval_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../agents/code-reviewer/evals");
    seed(&fixture, &eval_dir, "cost-growing", false).unwrap();
    let before = snapshot(&fixture).unwrap();
    fs::write(fixture.join("cost.mjs"), "changed").unwrap();
    assert_ne!(before, snapshot(&fixture).unwrap());
}

#[test]
fn snapshot_overhead_measurement() {
    let temp = TempDir::create(&env::temp_dir(), "execution-unit").unwrap();
    let fixture = temp.path.join("fixture");
    let eval_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../agents/code-reviewer/evals");
    seed(&fixture, &eval_dir, "cost-growing", false).unwrap();
    let started = Instant::now();
    for _ in 0..25 {
        std::hint::black_box(snapshot(&fixture).unwrap());
    }
    let elapsed = started.elapsed();
    eprintln!(
        "snapshot fixture=cost-growing samples=25 bytes={} total_ms={} mean_ms={:.2}",
        snapshot(&fixture).unwrap().to_string().len(),
        elapsed.as_millis(),
        elapsed.as_secs_f64() * 1000.0 / 25.0
    );
}

#[test]
fn worker_dispatch_start_mismatch_and_missing_snapshots_reject() {
    let temp = TempDir::create(&env::temp_dir(), "execution-unit").unwrap();
    let (events, _) = worker_fixture(&temp.path);
    let mut mismatch = events.clone();
    mismatch[1]["snapshot"]["files"]["cost.mjs"]["hash"] = json!("different");
    assert!(child_evidence(&mismatch, "root").is_err());
    for index in [0, 1] {
        let mut missing = events.clone();
        missing[index].as_object_mut().unwrap().remove("snapshot");
        assert!(child_evidence(&missing, "root").is_err());
    }
}

fn judge_trace() -> Value {
    json!({"events": [
        {"type":"tool_execution_start", "toolName":"read", "toolCallId":"read"},
        {"type":"tool_execution_end", "toolName":"read", "toolCallId":"read", "result":{}},
        {"type":"tool_execution_start", "toolName":"bash", "toolCallId":"proof", "args":{"command":"node proof.mjs"}},
        {"type":"tool_execution_end", "toolName":"bash", "toolCallId":"proof", "result":{}},
        {"type":"recorder_complete"}
    ]})
}

#[test]
fn judge_requires_explicit_inventory_and_complete_proof_coverage() {
    let trace = judge_trace();
    let valid = json!({"score":10,"findings":[{"id":"f1"},{"id":"f2"}],"proof_checks":[{"finding_id":"f1","command":"node proof.mjs"},{"finding_id":"f2","command":"node proof.mjs"}]});
    assert_eq!(validate_judge(&valid.to_string(), &trace).unwrap(), 10);
    let no_findings = json!({"score":10,"findings":[],"proof_checks":[],"no_findings":true});
    assert_eq!(
        validate_judge(&no_findings.to_string(), &trace).unwrap(),
        10
    );
    let mut inspection_only = trace.clone();
    inspection_only["events"]
        .as_array_mut()
        .unwrap()
        .drain(2..4);
    assert_eq!(
        validate_judge(&no_findings.to_string(), &inspection_only).unwrap(),
        10
    );
    assert!(validate_judge("} invalid {", &trace).is_err());
    for invalid in [
        json!({"score":10,"proof_checks":[]}),
        json!({"score":10,"findings":[],"proof_checks":[]}),
        json!({"score":10,"findings":[],"proof_checks":[],"no_findings":false}),
        json!({"score":10,"findings":[{"id":"f1"}],"proof_checks":[]}),
        json!({"score":10,"findings":[{}],"proof_checks":[]}),
        json!({"score":10,"findings":[{"id":""}],"proof_checks":[]}),
    ] {
        assert!(
            validate_judge(&invalid.to_string(), &trace).is_err(),
            "{invalid}"
        );
    }
    let mut variants = Vec::new();
    let mut invalid = valid.clone();
    invalid["proof_checks"].as_array_mut().unwrap().pop();
    variants.push(invalid);
    for (field, index, key, value) in [
        ("findings", 1, "id", json!("f1")),
        ("proof_checks", 1, "finding_id", json!("f1")),
        ("proof_checks", 1, "finding_id", json!("unknown")),
        ("proof_checks", 1, "finding_id", Value::Null),
        ("proof_checks", 1, "command", json!("never executed")),
    ] {
        let mut invalid = valid.clone();
        invalid[field][index][key] = value;
        variants.push(invalid);
    }
    for flag in [json!(true), Value::Null, json!("false")] {
        let mut invalid = valid.clone();
        invalid["no_findings"] = flag;
        variants.push(invalid);
    }
    for invalid in variants {
        assert!(
            validate_judge(&invalid.to_string(), &trace).is_err(),
            "{invalid}"
        );
    }
    for index in [1, 3] {
        let mut missing_result = trace.clone();
        missing_result["events"]
            .as_array_mut()
            .unwrap()
            .remove(index);
        assert!(validate_judge(&valid.to_string(), &missing_result).is_err());
        assert!(validate_judge(&no_findings.to_string(), &missing_result).is_err());
    }
}

#[test]
fn detached_snapshots_preserve_commit_identity_and_nonrepo_errors() {
    let temp = TempDir::create(&env::temp_dir(), "execution-unit").unwrap();
    assert!(snapshot(&temp.path).is_err());
    let fixture = temp.path.join("fixture");
    seed(&fixture, Path::new("."), "c1", false).unwrap();
    let attached = snapshot(&fixture).unwrap();
    git(&fixture, &["checkout", "-q", "--detach", "HEAD"]).unwrap();
    let detached = snapshot(&fixture).unwrap();
    assert!(detached["branch"].is_null());
    assert_eq!(attached["head"], detached["head"]);
    assert_ne!(attached, detached);
    git(&fixture, &["checkout", "-q", "--detach", "feature-auth"]).unwrap();
    let other = snapshot(&fixture).unwrap();
    assert!(other["branch"].is_null());
    assert_ne!(detached["head"], other["head"]);
    assert_ne!(detached, other);
}

#[test]
fn snapshot_limits_and_file_identity() {
    let temp = TempDir::create(&env::temp_dir(), "execution-unit").unwrap();
    let path = temp.path.join("data");
    fs::write(&path, "abcd").unwrap();
    let first = files(&temp.path, 1, 4).unwrap();
    assert!(first["data"].get("text").is_none());
    assert_eq!(
        first["data"]["hash"],
        format!("{:x}", Sha1::digest(b"abcd"))
    );
    assert!(files(&temp.path, 0, 4).is_err());
    assert!(files(&temp.path, 1, 3).is_err());
    fs::write(&path, "abce").unwrap();
    assert_ne!(first, files(&temp.path, 1, 4).unwrap());
    let before_mode = files(&temp.path, 1, 4).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    assert_ne!(before_mode, files(&temp.path, 1, 4).unwrap());
    let link = temp.path.join("link");
    std::os::unix::fs::symlink("data", &link).unwrap();
    let before_link = files(&temp.path, 2, 8).unwrap();
    fs::remove_file(&link).unwrap();
    std::os::unix::fs::symlink("else", &link).unwrap();
    assert_ne!(before_link, files(&temp.path, 2, 8).unwrap());
    assert!(files(&temp.path, 2, 7).is_err());
    fs::create_dir(temp.path.join("empty")).unwrap();
    assert!(files(&temp.path, 2, 8).is_err());
    assert!(files(&temp.path, 3, 8).is_ok());
}

#[test]
fn large_fixture_hashing_size_and_overhead() {
    let temp = TempDir::create(&env::temp_dir(), "execution-unit").unwrap();
    let path = temp.path.join("large");
    fs::write(&path, vec![65; MAX_FIXTURE_BYTES as usize]).unwrap();
    let started = Instant::now();
    let contents = files(&temp.path, MAX_FIXTURE_ENTRIES, MAX_FIXTURE_BYTES).unwrap();
    let elapsed = started.elapsed();
    let size = serde_json::to_vec(&contents).unwrap().len();
    assert!(size < 200);
    eprintln!(
        "snapshot hashing fixture_bytes={} serialized_files_bytes={size} elapsed_ms={:.2}",
        MAX_FIXTURE_BYTES,
        elapsed.as_secs_f64() * 1000.0
    );
    fs::OpenOptions::new()
        .write(true)
        .open(path)
        .unwrap()
        .set_len(MAX_FIXTURE_BYTES + 1)
        .unwrap();
    assert!(files(&temp.path, MAX_FIXTURE_ENTRIES, MAX_FIXTURE_BYTES).is_err());
}

#[test]
fn evidence_bounds_accept_boundary_and_reject_overflow_or_truncation() {
    let temp = TempDir::create(&env::temp_dir(), "execution-unit").unwrap();
    let path = temp.path.join("log");
    fs::write(&path, "{}\n").unwrap();
    assert_eq!(bounded_read(&path, 3).unwrap(), b"{}\n");
    assert!(bounded_read(&path, 2).is_err());
    assert_eq!(records(&path).unwrap().len(), 1);
    for text in ["{}", "{\n", "", "{\"type\":\"recorder_error\"}\n"] {
        fs::write(&path, text).unwrap();
        assert!(records(&path).is_err());
    }
    fs::write(&path, "{\"type\":\"recorder_started\"}\n").unwrap();
    assert!(paired_tools(&records(&path).unwrap()).is_err());
    fs::write(
        &path,
        format!("{}{{}}\n", " ".repeat(MAX_LOG_BYTES as usize - 3)),
    )
    .unwrap();
    assert_eq!(records(&path).unwrap().len(), 1);
    fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(b"\n")
        .unwrap();
    assert!(records(&path).is_err());
    let value = json!({"repeat": ["snapshot", "snapshot"]});
    let size = value.to_string().len();
    assert_eq!(evidence_size(&value, size).unwrap(), size);
    assert!(evidence_size(&value, size - 1).is_err());
    let oversized = json!({"events": vec!["x".repeat(1024); 32768]});
    assert!(evidence_size(&oversized, MAX_EVIDENCE_BYTES).is_err());
}
