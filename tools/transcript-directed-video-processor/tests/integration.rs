// End-to-end test of `analyze` against the synthetic fixture (known transcript
// gaps at t=1.0-3.0 and t=3.5-5.5, matching the issue's explicit ask for "a
// test fixture with known transcript and visual events"). Invokes the actual
// compiled binary rather than internal functions — this crate has no lib
// target, and an integration test through the real CLI(command-line
// interface) surface is what a caller of this tool actually runs.

use std::path::Path;
use std::process::Command;

fn generate_fixture(dir: &Path) {
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/generate.sh");
    let status = Command::new(script)
        .arg(dir)
        .status()
        .expect("failed to run fixture generator");
    assert!(status.success(), "fixture generator failed");
}

#[test]
fn analyze_moments_match_known_transcript_gap_timestamps() {
    if Command::new("ffmpeg").arg("-version").output().is_err() {
        eprintln!("skipping: ffmpeg not on PATH");
        return;
    }
    // AGNT-INV-001: tempfile::tempdir() is process-locally unique, so this test
    // never collides with a concurrently running copy of itself.
    let fixture_dir = tempfile::tempdir().unwrap();
    generate_fixture(fixture_dir.path());

    let work_dir = tempfile::tempdir().unwrap();
    let out_dir = work_dir.path().join("out");

    let output = Command::new(env!("CARGO_BIN_EXE_transcript-directed-video-processor"))
        .current_dir(work_dir.path())
        .args([
            "analyze",
            "--input",
            fixture_dir.path().join("video.mp4").to_str().unwrap(),
            "--out",
            out_dir.to_str().unwrap(),
        ])
        .output()
        .expect("failed to run analyze");

    assert!(
        output.status.success(),
        "analyze failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let chapters_raw = std::fs::read_to_string(out_dir.join("chapters.json")).unwrap();
    let chapters: serde_json::Value = serde_json::from_str(&chapters_raw).unwrap();
    let moments = chapters["moments"].as_array().unwrap();

    assert_eq!(moments.len(), 3, "expected 3 moments from the known transcript gaps: {chapters_raw}");
    let starts: Vec<f64> = moments.iter().map(|m| m["start_s"].as_f64().unwrap()).collect();
    assert_eq!(starts, vec![0.0, 3.0, 5.5]);
}

#[test]
fn help_flags_print_usage_and_exit_success() {
    for flag in ["--help", "-h"] {
        let output = Command::new(env!("CARGO_BIN_EXE_transcript-directed-video-processor"))
            .arg(flag)
            .output()
            .unwrap_or_else(|e| panic!("failed to run with {flag}: {e}"));
        assert!(output.status.success(), "{flag} should exit 0");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("usage:"), "{flag} stdout should show usage, got: {stdout}");
    }
}

#[test]
fn help_flag_after_a_subcommand_also_shows_usage() {
    let output = Command::new(env!("CARGO_BIN_EXE_transcript-directed-video-processor"))
        .args(["analyze", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("usage:"));
}

#[test]
fn no_args_is_still_an_error_not_a_help_screen() {
    let output = Command::new(env!("CARGO_BIN_EXE_transcript-directed-video-processor"))
        .output()
        .unwrap();
    assert!(!output.status.success(), "no arguments should still be an error");
}
