use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum Source {
    Youtube { url: String, video_id: String },
    Local { path: PathBuf },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum TranscriptOrigin {
    Captions,
    AutoCaptions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptMeta {
    pub origin: TranscriptOrigin,
    pub language: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Moment {
    pub index: usize,
    pub start_s: f64,
    pub end_s: f64,
    pub title: String,
    pub confidence: f64,
    pub transcript_excerpt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisOutput {
    pub source: Source,
    pub transcript: TranscriptMeta,
    pub moments: Vec<Moment>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewEvidence {
    pub moment_index: usize,
    pub frame_path: PathBuf,
    pub frame_timestamp_s: f64,
    // Present only when --clip was requested for this moment: a short clip
    // extracted alongside the still frame, kept for human archival review. The
    // vision model always reviews `frame_path` (a still image) — no vision
    // provider in this tool's genai integration accepts a video clip as input,
    // so a clip is evidence, not a model input.
    pub clip_path: Option<PathBuf>,
    pub vision_model: String,
    pub model_response: String,
    pub reviewed_at: String,
}
