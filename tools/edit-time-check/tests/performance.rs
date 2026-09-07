use edit_time_check::{decode_request, evaluate, Decision, Options, Stage};
use std::time::Instant;

#[test]
#[ignore = "explicit release-mode timing run"]
fn registry_scaling() {
    let root = std::env::temp_dir().join(format!("edit-time-timing-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let root = root.canonicalize().unwrap();
    std::fs::write(root.join("config"), "version=1").unwrap();
    std::fs::write(
        root.join("rules"),
        "Only approved comment shapes. Boolean names use is prefix.",
    )
    .unwrap();
    std::fs::write(root.join("identifiers"), "").unwrap();
    let mut options =
        Options::for_paths(root.join("config"), root.join("rules"), root.join("rules"));
    options.identifiers = Some(root.join("identifiers"));
    options.judgment_configuration = "timing-v1".into();
    for (count, is_rename) in [
        (100usize, false),
        (1000, false),
        (10_000, false),
        (200, true),
        (400, true),
        (800, true),
        (1300, true),
    ] {
        let text = (0..count)
            .map(|i| format!("fn is_f{i}() -> bool {{ let is_ready = true; is_ready }}\n"))
            .collect::<String>();
        let proposed = if is_rename {
            text.replace("is_f", "is_g").replace("is_ready", "is_set")
        } else {
            text.replacen("true", "false", 1)
        };
        for operation in ["edit", "write"] {
            let value = serde_json::json!({"version":1,"request_id":"timing","operation":operation,"path":root.join("x.rs"),"repository_root":null,"original_text":text,"proposed_text":proposed,"changed_ranges":[],"budget_ms":20000});
            let request = decode_request(&serde_json::to_vec(&value).unwrap()).unwrap();
            let mut samples = Vec::new();
            for _ in 0..20 {
                let started = Instant::now();
                let response = evaluate(&request, &options, &Stage::new());
                samples.push(started.elapsed().as_micros());
                assert_eq!(response.decision, Decision::Pass, "{response:?}");
            }
            samples.sort_unstable();
            println!(
                "{operation} two_symbol_rename={is_rename} bytes={} declarations={count} trials=20 p50_us={} p95_us={} max_us={}",
                text.len(),
                samples[9],
                samples[18],
                samples[19]
            );
            assert!(samples[18] < 3_000_000);
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}
