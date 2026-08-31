// Shells out to the `ffmpeg` CLI binary rather than linking `ffmpeg-next` (the
// libav* Rust bindings) — matches this repo's existing external-process pattern
// (tools/usage-limit-watch, tools/dispatch-baseline both spawn a CLI rather than
// linking a library), and avoids ffmpeg-next's compile-time dependency on FFmpeg's
// dev headers, which this repo's other tools don't require of a build machine.

use std::path::Path;
use std::process::Command;

/// Builds the argv for extracting a single frame at `timestamp_s` from `input`,
/// writing a JPEG to `output`. Pure and unit-testable: no process is spawned here.
pub fn frame_extract_args(input: &Path, timestamp_s: f64, output: &Path) -> Vec<String> {
    vec![
        "-y".to_string(),
        "-ss".to_string(),
        format!("{timestamp_s:.3}"),
        "-i".to_string(),
        input.to_string_lossy().to_string(),
        "-frames:v".to_string(),
        "1".to_string(),
        output.to_string_lossy().to_string(),
    ]
}

/// Builds the argv for extracting a short clip from `start_s` to `end_s`.
pub fn clip_extract_args(input: &Path, start_s: f64, end_s: f64, output: &Path) -> Vec<String> {
    let duration = (end_s - start_s).max(0.0);
    vec![
        "-y".to_string(),
        "-ss".to_string(),
        format!("{start_s:.3}"),
        "-i".to_string(),
        input.to_string_lossy().to_string(),
        "-t".to_string(),
        format!("{duration:.3}"),
        "-c".to_string(),
        "copy".to_string(),
        output.to_string_lossy().to_string(),
    ]
}

// audio_extract_args (16kHz mono PCM WAV, the format whisper-family tools expect)
// is deferred along with the rest of the local-transcription fallback — see
// plan.md's scope decision. Adding it back unwired ahead of that path existing
// would be speculative code with no call site; it returns when whisper.cpp
// integration is actually built.

pub fn run_ffmpeg(args: &[String]) -> Result<(), String> {
    let output = Command::new("ffmpeg")
        .args(args)
        .output()
        .map_err(|error| format!("failed to spawn ffmpeg: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "ffmpeg exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn frame_extract_args_places_timestamp_before_input_for_fast_seek() {
        let args = frame_extract_args(
            Path::new("in.mp4"),
            12.5,
            &PathBuf::from("out.jpg"),
        );
        let ss_pos = args.iter().position(|a| a == "-ss").unwrap();
        let i_pos = args.iter().position(|a| a == "-i").unwrap();
        assert!(
            ss_pos < i_pos,
            "-ss must precede -i for fast (keyframe-seek then decode) extraction: {args:?}"
        );
        assert_eq!(args[ss_pos + 1], "12.500");
    }

    #[test]
    fn clip_extract_args_computes_positive_duration() {
        let args = clip_extract_args(Path::new("in.mp4"), 2.0, 5.5, &PathBuf::from("out.mp4"));
        let t_pos = args.iter().position(|a| a == "-t").unwrap();
        assert_eq!(args[t_pos + 1], "3.500");
    }

    #[test]
    fn clip_extract_args_never_produces_a_negative_duration() {
        let args = clip_extract_args(Path::new("in.mp4"), 5.0, 2.0, &PathBuf::from("out.mp4"));
        let t_pos = args.iter().position(|a| a == "-t").unwrap();
        assert_eq!(args[t_pos + 1], "0.000");
    }

    // AGNT-INV-002: ffmpeg's `-ss`/`-i` seek-and-extract semantics are a
    // third-party behavior this whole tool relies on for frame-accurate
    // extraction — proven here against a hand-built known answer (a synthetic
    // fixture whose color at each timestamp is analytically known, per
    // tests/fixtures/generate.sh) rather than trusted from documentation alone.
    #[test]
    fn oracle_frame_extraction_matches_known_fixture_colors() {
        if std::process::Command::new("ffmpeg").arg("-version").output().is_err() {
            eprintln!("skipping: ffmpeg not on PATH");
            return;
        }
        let dir = tempfile::tempdir().unwrap(); // AGNT-INV-001: unique per test run
        let script = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/generate.sh");
        let status = std::process::Command::new(script)
            .arg(dir.path())
            .status()
            .expect("failed to run fixture generator");
        assert!(status.success(), "fixture generator failed");
        let video = dir.path().join("video.mp4");

        let cases: [(f64, [u8; 3]); 3] = [(1.0, [255, 0, 0]), (3.0, [0, 0, 255]), (5.0, [0, 128, 0])];
        for (timestamp, expected_rgb) in cases {
            let frame_path = dir.path().join(format!("frame_{timestamp}.jpg"));
            let args = frame_extract_args(&video, timestamp, &frame_path);
            run_ffmpeg(&args).unwrap();
            let img = image::open(&frame_path).unwrap().to_rgb8();
            let pixel = img.get_pixel(img.width() / 2, img.height() / 2);
            for channel in 0..3 {
                let diff = (pixel[channel] as i16 - expected_rgb[channel] as i16).abs();
                assert!(
                    diff < 20,
                    "at t={timestamp}s expected ~{expected_rgb:?}, got {:?} (channel {channel} diff {diff})",
                    pixel.0
                );
            }
        }
    }
}
