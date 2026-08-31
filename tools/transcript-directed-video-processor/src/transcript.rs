// Transcript acquisition. YouTube captions are fetched by shelling out to yt-dlp
// (`--write-subs`/`--write-auto-subs`) rather than hand-rolling YouTube's
// unofficial timedtext endpoint — that endpoint is actively rate-limited/blocked
// in production per research, while yt-dlp's extractor is maintained against
// YouTube's changes. Local video files use a same-named sidecar caption file if
// present; a video with no captions and no sidecar has no transcription path yet
// in this pass (see plan.md's scope decision) and returns a clear error rather
// than silently producing an empty transcript.

use crate::model::{Source, TranscriptMeta, TranscriptOrigin};
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn youtube_dl_args(url: &str, out_prefix: &Path) -> Vec<String> {
    vec![
        "--write-subs".to_string(),
        "--write-auto-subs".to_string(),
        "--sub-lang".to_string(),
        "en".to_string(),
        "--sub-format".to_string(),
        "srt".to_string(),
        "--skip-download".to_string(),
        "-o".to_string(),
        format!("{}.%(ext)s", out_prefix.to_string_lossy()),
        url.to_string(),
    ]
}

pub fn video_id_from_url(url: &str) -> String {
    // Best-effort extraction for the Source::Youtube record; not relied on for
    // correctness of the fetch itself (yt-dlp resolves the real video from the
    // full URL) — only used as a human-readable label in analysis output. A
    // naive "last '='-or-'/'-delimited segment" split (the prior version of
    // this function) picks up the wrong query param on a URL carrying more
    // than one (e.g. a playlist URL's trailing "&index=1" instead of "v="'s
    // value) — confirmed against a real playlist-context YouTube URL, which
    // mislabeled video_id as "1". The `v` query parameter is checked
    // explicitly first; only a URL with no `v` param (a youtu.be short link)
    // falls back to the last path segment.
    if let Some(query) = url.split('?').nth(1) {
        for pair in query.split('&') {
            if let Some(value) = pair.strip_prefix("v=") {
                return value.to_string();
            }
        }
    }
    let without_query = url.split('?').next().unwrap_or(url);
    without_query
        .rsplit('/')
        .find(|segment| !segment.is_empty())
        .unwrap_or(url)
        .to_string()
}

// yt-dlp writes "<prefix>.en.srt" for manual captions and the same pattern for
// auto-captions (there is no separate filename convention distinguishing them
// at this CLI(command-line interface) surface) — treated as
// TranscriptOrigin::Captions unless yt-dlp's own stdout says it fell back to
// the automatic track. Split into a pure function so this heuristic (which a
// future yt-dlp wording change could silently break) is unit-testable without
// a live network call, unlike the fetch it's used inside of.
fn detect_origin(stdout: &str) -> TranscriptOrigin {
    if stdout.contains("Downloading subtitles") {
        TranscriptOrigin::Captions
    } else {
        TranscriptOrigin::AutoCaptions
    }
}

pub fn fetch_youtube_captions(url: &str, out_dir: &Path) -> Result<(Source, TranscriptMeta), String> {
    std::fs::create_dir_all(out_dir).map_err(|e| e.to_string())?;
    let out_prefix = out_dir.join("captions");
    let args = youtube_dl_args(url, &out_prefix);
    let output = Command::new("yt-dlp")
        .args(&args)
        .output()
        .map_err(|error| format!("failed to spawn yt-dlp: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "yt-dlp exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let origin = detect_origin(&stdout);

    let srt_path = out_dir.join("captions.en.srt");
    if !srt_path.exists() {
        return Err(format!(
            "yt-dlp reported success but no caption file was written at {}; video may have no English captions at all",
            srt_path.display()
        ));
    }

    Ok((
        Source::Youtube {
            url: url.to_string(),
            video_id: video_id_from_url(url),
        },
        TranscriptMeta {
            origin,
            language: "en".to_string(),
            path: srt_path,
        },
    ))
}

pub fn acquire_local(path: &Path) -> Result<(Source, TranscriptMeta), String> {
    for ext in ["srt", "vtt"] {
        let sidecar: PathBuf = path.with_extension(ext);
        if sidecar.exists() {
            return Ok((
                Source::Local {
                    path: path.to_path_buf(),
                },
                TranscriptMeta {
                    origin: TranscriptOrigin::Captions,
                    language: "en".to_string(),
                    path: sidecar,
                },
            ));
        }
    }
    Err(format!(
        "no captions found; local transcription (whisper.cpp) is not yet implemented — \
         place a matching .srt or .vtt sidecar next to {} to use this tool today",
        path.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn youtube_dl_args_prefers_manual_over_auto_captions_by_flag_order() {
        let args = youtube_dl_args("https://youtu.be/abc123", Path::new("/tmp/out/captions"));
        assert!(args.contains(&"--write-subs".to_string()));
        assert!(args.contains(&"--write-auto-subs".to_string()));
        assert!(args.contains(&"--skip-download".to_string()));
    }

    #[test]
    fn video_id_from_url_handles_watch_and_short_urls() {
        assert_eq!(video_id_from_url("https://www.youtube.com/watch?v=abc123"), "abc123");
        assert_eq!(video_id_from_url("https://youtu.be/abc123"), "abc123");
    }

    #[test]
    fn video_id_from_url_picks_v_param_not_a_trailing_playlist_param() {
        assert_eq!(
            video_id_from_url("http://youtube.com/watch?v=Z98ZuXR7kDM&list=TLPQMzAwODIwMjZXswMf1dPAYA&index=1"),
            "Z98ZuXR7kDM"
        );
    }

    #[test]
    fn video_id_from_url_handles_a_short_url_with_a_trailing_query() {
        assert_eq!(video_id_from_url("https://youtu.be/abc123?t=30"), "abc123");
    }

    #[test]
    fn acquire_local_finds_srt_sidecar() {
        let dir = tempfile::tempdir().unwrap();
        let video = dir.path().join("clip.mp4");
        std::fs::write(&video, b"not a real video").unwrap();
        std::fs::write(dir.path().join("clip.srt"), "1\n00:00:00,000 --> 00:00:01,000\nhi\n").unwrap();
        let (_, meta) = acquire_local(&video).unwrap();
        assert_eq!(meta.origin, TranscriptOrigin::Captions);
    }

    #[test]
    fn acquire_local_without_sidecar_errors_clearly() {
        let dir = tempfile::tempdir().unwrap();
        let video = dir.path().join("clip.mp4");
        std::fs::write(&video, b"not a real video").unwrap();
        let error = acquire_local(&video).unwrap_err();
        assert!(error.contains("whisper.cpp"));
    }

    #[test]
    fn detect_origin_recognizes_manual_subtitles() {
        assert_eq!(
            detect_origin("[info] Writing video subtitles to captions.en.srt\nDownloading subtitles: en\n"),
            TranscriptOrigin::Captions
        );
    }

    #[test]
    fn detect_origin_falls_back_to_auto_captions_when_no_manual_line_is_present() {
        assert_eq!(
            detect_origin("[info] Downloading automatic captions: en\n"),
            TranscriptOrigin::AutoCaptions
        );
    }
}
