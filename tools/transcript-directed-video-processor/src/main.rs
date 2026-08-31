mod cli;
mod ffmpeg;
mod model;
mod segment;
mod srt;
mod transcript;
mod vision;

use cli::Flags;
use model::{AnalysisOutput, ReviewEvidence, Source};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "usage:
  transcript-directed-video-processor analyze --url <youtube-url> --out <dir>
  transcript-directed-video-processor analyze --input <local-video> --out <dir>
  transcript-directed-video-processor review --dir <analyze-out-dir> --moments <comma-separated indices> --model <name> [--clip yes]
";

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(message) => {
            eprintln!("transcript-directed-video-processor: {message}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<ExitCode, String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Checked before Flags::parse (which requires every `--flag` to carry a
    // following value, so a bare `--help`/`-h` would otherwise fail as
    // "requires a value" instead of showing usage) and anywhere in argv, so
    // both `--help` and `analyze --help` work.
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print!("{USAGE}");
        return Ok(ExitCode::SUCCESS);
    }
    let subcommand = args.first().map(String::as_str).ok_or(USAGE)?;
    let flags = Flags::parse(&args[1..])?;
    match subcommand {
        "analyze" => cmd_analyze(&flags),
        "review" => cmd_review(&flags),
        _ => Err(USAGE.to_string()),
    }
}

// Rejects a --out/--dir destination that escapes the directory the caller is
// running from — matches tools/dispatch-baseline's output_is_inside_repo guard;
// an "analysis wrote outside the folder I pointed it at" failure is exactly the
// kind of silent scope violation that guard exists to catch.
fn resolve_contained_dir(raw: &str) -> Result<PathBuf, String> {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    resolve_contained_dir_under(raw, &cwd)
}

// Split out from resolve_contained_dir so tests can pass an explicit base
// directory instead of mutating the process-global current directory — cargo
// test runs tests concurrently in one process, and set_current_dir there would
// race across threads (the exact per-test-isolated-path discipline AGNT-INV-001
// asks for, applied to process state rather than a file path).
fn resolve_contained_dir_under(raw: &str, base: &Path) -> Result<PathBuf, String> {
    // Lexical check FIRST, before anything touches disk: a raw path carrying a
    // '..' component can climb outside `base` no matter how it's joined, and
    // rejecting it here means a destination this function is about to refuse
    // never gets created in the first place (the bug a prior version of this
    // function had: create_dir_all ran before the containment check, so a
    // rejected --out/--dir still landed a directory on disk).
    let raw_path = Path::new(raw);
    if raw_path.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        return Err(format!(
            "{raw} contains a '..' component — refusing a destination that can climb outside the granted directory"
        ));
    }
    let base_resolved = base.canonicalize().map_err(|e| e.to_string())?;
    let target = if raw_path.is_absolute() {
        PathBuf::from(raw)
    } else {
        base.join(raw_path)
    };

    // Resolve symlinks in whatever prefix of `target` already exists (target
    // itself usually doesn't yet, so a full canonicalize() would just fail) and
    // check containment against that BEFORE creating anything — this is what
    // catches an absolute path whose existing ancestor differs from `base`'s
    // canonical form only by a symlink (e.g. /var vs /private/var on macOS)
    // without ever calling create_dir_all on a destination we're about to reject.
    let lexical = canonicalize_existing_ancestor(&target)?;
    if !lexical.starts_with(&base_resolved) {
        return Err(format!(
            "{} escapes the granted directory {} — refusing to write outside it",
            lexical.display(),
            base_resolved.display()
        ));
    }

    std::fs::create_dir_all(&target).map_err(|e| format!("could not create {}: {e}", target.display()))?;
    let resolved = target
        .canonicalize()
        .map_err(|e| format!("could not resolve {}: {e}", target.display()))?;
    if !resolved.starts_with(&base_resolved) {
        // A symlink created between the lexical check and this point (or one
        // inside `target` itself) pointed the just-created path outside `base`
        // — caught only after creation since canonicalize() requires the path
        // to exist. Best-effort cleanup of exactly what we created; this does
        // not guard every possible pre-existing symlink race, but it turns a
        // silent escape into a removed directory plus a clear error instead of
        // a stray write outside the caller's granted root.
        let _ = std::fs::remove_dir_all(&resolved);
        return Err(format!(
            "{} escapes the granted directory {} (via a symlink) — refusing to write outside it",
            resolved.display(),
            base_resolved.display()
        ));
    }
    Ok(resolved)
}

fn canonicalize_existing_ancestor(path: &Path) -> Result<PathBuf, String> {
    let mut existing = path.to_path_buf();
    let mut suffix: Vec<std::ffi::OsString> = Vec::new();
    while !existing.exists() {
        let name = existing
            .file_name()
            .ok_or_else(|| format!("could not resolve any existing ancestor of {}", path.display()))?
            .to_os_string();
        suffix.push(name);
        existing = existing
            .parent()
            .ok_or_else(|| format!("could not resolve any existing ancestor of {}", path.display()))?
            .to_path_buf();
    }
    let mut resolved = existing.canonicalize().map_err(|e| e.to_string())?;
    for component in suffix.into_iter().rev() {
        resolved.push(component);
    }
    Ok(resolved)
}

fn cmd_analyze(flags: &Flags) -> Result<ExitCode, String> {
    // Validated before resolve_contained_dir runs so a rejected --url/--input
    // combination never creates the --out directory at all.
    match (flags.get("url"), flags.get("input")) {
        (Some(_), Some(_)) => return Err("pass exactly one of --url or --input, not both".to_string()),
        (None, None) => return Err("one of --url or --input is required".to_string()),
        _ => {}
    }
    let out_dir = resolve_contained_dir(flags.require("out")?)?;

    let (source, transcript_meta) = match (flags.get("url"), flags.get("input")) {
        (Some(url), None) => transcript::fetch_youtube_captions(url, &out_dir)?,
        (None, Some(input)) => transcript::acquire_local(Path::new(input))?,
        _ => unreachable!("validated above"),
    };

    let raw = std::fs::read_to_string(&transcript_meta.path)
        .map_err(|e| format!("could not read transcript {}: {e}", transcript_meta.path.display()))?;
    let cues = srt::parse(&raw)?;
    let moments = segment::segment(&cues);

    let output = AnalysisOutput {
        source,
        transcript: transcript_meta,
        moments,
    };
    let json = serde_json::to_string_pretty(&output).map_err(|e| e.to_string())? + "\n";
    let chapters_path = out_dir.join("chapters.json");
    std::fs::write(&chapters_path, json).map_err(|e| e.to_string())?;
    println!("wrote {} moments to {}", output.moments.len(), chapters_path.display());
    Ok(ExitCode::SUCCESS)
}

fn cmd_review(flags: &Flags) -> Result<ExitCode, String> {
    let dir = resolve_contained_dir(flags.require("dir")?)?;
    let moments_raw = flags.require("moments")?;
    let requested: Vec<usize> = moments_raw
        .split(',')
        .map(|s| s.trim().parse::<usize>().map_err(|_| format!("not a valid moment index: {s}")))
        .collect::<Result<_, _>>()?;

    let chapters_path = dir.join("chapters.json");
    let chapters_raw = std::fs::read_to_string(&chapters_path)
        .map_err(|e| format!("could not read {}: {e} — run `analyze` first", chapters_path.display()))?;
    let analysis: AnalysisOutput = serde_json::from_str(&chapters_raw).map_err(|e| e.to_string())?;

    let model = flags
        .get("model")
        .map(str::to_string)
        .ok_or_else(|| "--model is required (a genai model-name string, e.g. gpt-5.1, claude-sonnet-4-5, gemini-2.5-pro)".to_string())?;

    let video_path = ensure_video_available(&analysis.source, &dir)?;

    let frames_dir = dir.join("frames");
    std::fs::create_dir_all(&frames_dir).map_err(|e| e.to_string())?;

    let runtime = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
    let mut evidence = load_existing_evidence(&dir)?;

    for index in requested {
        let moment = analysis
            .moments
            .iter()
            .find(|m| m.index == index)
            .ok_or_else(|| format!("no moment with index {index} in {}", chapters_path.display()))?;

        let frame_path = frames_dir.join(format!("moment_{index}.jpg"));
        let frame_args = ffmpeg::frame_extract_args(&video_path, moment.start_s, &frame_path);
        ffmpeg::run_ffmpeg(&frame_args)?;

        // AGNT-INV-003: the frame this loop reports as extracted is the exact
        // path handed to the vision model below — never a stand-in.
        if !frame_path.exists() {
            return Err(format!("ffmpeg reported success but {} was not written", frame_path.display()));
        }

        // A per-moment --clip flag additionally extracts a short clip (the issue
        // asks the tool to pick "frames or short clips" per moment) for archival
        // evidence alongside the still frame the vision model actually reviews.
        let clip_path = if flags.get("clip") == Some("yes") {
            let clip_path = frames_dir.join(format!("moment_{index}.mp4"));
            let clip_args = ffmpeg::clip_extract_args(&video_path, moment.start_s, moment.end_s, &clip_path);
            ffmpeg::run_ffmpeg(&clip_args)?;
            if !clip_path.exists() {
                return Err(format!("ffmpeg reported success but {} was not written", clip_path.display()));
            }
            Some(clip_path)
        } else {
            None
        };

        let prompt = format!(
            "This frame is from timestamp {:.1}s, transcript context: \"{}\". Describe what is visible.",
            moment.start_s, moment.transcript_excerpt
        );
        let response = runtime
            .block_on(vision::review_frame(&model, &prompt, &frame_path.to_string_lossy()))?;

        evidence.push(ReviewEvidence {
            moment_index: index,
            frame_path: frame_path.clone(),
            frame_timestamp_s: moment.start_s,
            clip_path,
            vision_model: model.clone(),
            model_response: response,
            reviewed_at: now_rfc3339(),
        });

        // Persisted after every moment, not once at the end of the loop: a
        // later moment in the same --moments list failing (ffmpeg error, vision
        // API error) must not discard the evidence already earned for the
        // moments that succeeded before it.
        write_evidence(&dir, &evidence)?;
    }

    println!("wrote {} evidence records to {}", evidence.len(), dir.join("evidence.json").display());
    Ok(ExitCode::SUCCESS)
}

fn write_evidence(dir: &Path, evidence: &[ReviewEvidence]) -> Result<(), String> {
    let json = serde_json::to_string_pretty(evidence).map_err(|e| e.to_string())? + "\n";
    std::fs::write(dir.join("evidence.json"), json).map_err(|e| e.to_string())
}

fn load_existing_evidence(dir: &Path) -> Result<Vec<ReviewEvidence>, String> {
    let path = dir.join("evidence.json");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    serde_json::from_str(&raw).map_err(|e| e.to_string())
}

fn ensure_video_available(source: &Source, dir: &Path) -> Result<PathBuf, String> {
    match source {
        Source::Local { path } => Ok(path.clone()),
        Source::Youtube { url, .. } => {
            for ext in ["mp4", "mkv", "webm"] {
                let candidate = dir.join(format!("video.{ext}"));
                if candidate.exists() {
                    return Ok(candidate);
                }
            }
            let out_prefix = dir.join("video");
            // Capped at 720p: `review` only needs still frames, not the
            // highest-quality rendition of the whole video — an uncapped
            // download undercuts the "selected frames only, cost-conscious"
            // design this tool's two-phase split exists for.
            //
            // "bestvideo+bestaudio" (merged via ffmpeg, which this tool already
            // depends on) rather than a bare "best": confirmed live against a
            // real video that YouTube no longer serves a single combined
            // video+audio ("progressive") format at most resolutions — a plain
            // `-f "best[height<=720]/best"` selector fails outright with
            // "Requested format is not available" because no single format
            // satisfies both the height cap and having audio.
            let output = std::process::Command::new("yt-dlp")
                .args([
                    "-f",
                    "bestvideo[height<=720]+bestaudio/best[height<=720]/best",
                    "-o",
                    &format!("{}.%(ext)s", out_prefix.to_string_lossy()),
                    url,
                ])
                .output()
                .map_err(|e| format!("failed to spawn yt-dlp: {e}"))?;
            if !output.status.success() {
                return Err(format!("yt-dlp video download failed: {}", String::from_utf8_lossy(&output.stderr)));
            }
            for ext in ["mp4", "mkv", "webm"] {
                let candidate = dir.join(format!("video.{ext}"));
                if candidate.exists() {
                    return Ok(candidate);
                }
            }
            Err(format!("yt-dlp reported success but no video.* file was found in {}", dir.display()))
        }
    }
}

fn now_rfc3339() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "unknown".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_contained_dir_rejects_a_path_outside_the_base() {
        let base = tempfile::tempdir().unwrap();
        let error = resolve_contained_dir_under("/tmp/definitely-outside-xyz", base.path()).unwrap_err();
        assert!(error.contains("escapes"));
    }

    #[test]
    fn resolve_contained_dir_accepts_a_relative_subdir() {
        let base = tempfile::tempdir().unwrap();
        let result = resolve_contained_dir_under("out-subdir", base.path());
        assert!(result.is_ok());
        assert!(result.unwrap().starts_with(base.path().canonicalize().unwrap()));
    }

    #[test]
    fn resolve_contained_dir_rejects_a_dot_dot_component_without_creating_anything() {
        let base = tempfile::tempdir().unwrap();
        let error = resolve_contained_dir_under("../escape-attempt", base.path()).unwrap_err();
        assert!(error.contains("'..'"));
        assert!(!base.path().parent().unwrap().join("escape-attempt").exists());
    }

    #[test]
    fn resolve_contained_dir_rejects_an_absolute_path_outside_base_without_creating_it() {
        let base = tempfile::tempdir().unwrap();
        let outside = base.path().parent().unwrap().join("outside-attempt-xyz");
        let error = resolve_contained_dir_under(outside.to_str().unwrap(), base.path()).unwrap_err();
        assert!(error.contains("escapes"));
        assert!(!outside.exists(), "a rejected destination must never be created on disk");
    }

    #[test]
    fn resolve_contained_dir_rejects_a_symlink_escape_and_cleans_up_after_itself() {
        let base = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let link = base.path().join("escape-link");
        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.path(), &link).unwrap();
        let error = resolve_contained_dir_under("escape-link/subdir", base.path()).unwrap_err();
        assert!(error.contains("escapes"));
        assert!(!outside.path().join("subdir").exists(), "escaped directory must be cleaned up, not left behind");
    }
}
