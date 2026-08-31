// Turns a flat cue list into candidate "moments" (chapter-like spans) using two
// signals: a pause-gap heuristic (a real pause between consecutive cues longer
// than `gap_threshold_s` is a likely topic boundary, per research), and a
// periodic fallback (`max_span_s`) for when no such pause ever appears.
//
// The fallback exists because YouTube's auto-generated (ASR) captions are
// rolling/overlapping: each cue's start_s lands before the previous cue's
// end_s (a smooth-scroll caption effect), so `next.start_s - prev.end_s` is
// negative for the entire transcript and the gap heuristic can never fire—
// confirmed against a real 68-minute auto-captioned video, which collapsed to
// a single moment before this fallback was added. Hand-written SRT/VTT with
// real silences still segments on the gap heuristic alone, since a genuine gap
// will always be found before max_span_s elapses.

use crate::model::Moment;
use crate::srt::Cue;

const DEFAULT_GAP_THRESHOLD_S: f64 = 1.5;
const DEFAULT_MAX_SPAN_S: f64 = 120.0;

pub fn segment(cues: &[Cue]) -> Vec<Moment> {
    segment_with_params(cues, DEFAULT_GAP_THRESHOLD_S, DEFAULT_MAX_SPAN_S)
}

pub fn segment_with_params(cues: &[Cue], gap_threshold_s: f64, max_span_s: f64) -> Vec<Moment> {
    if cues.is_empty() {
        return Vec::new();
    }

    let mut moments = Vec::new();
    let mut current_start = cues[0].start_s;
    let mut current_texts: Vec<&str> = vec![cues[0].text.as_str()];
    let mut current_end = cues[0].end_s;

    for window in cues.windows(2) {
        let (prev, next) = (&window[0], &window[1]);
        let gap = next.start_s - prev.end_s;
        let span_so_far = next.start_s - current_start;
        let boundary = if gap >= gap_threshold_s {
            Some(BoundaryKind::Pause(gap))
        } else if span_so_far >= max_span_s {
            Some(BoundaryKind::Forced)
        } else {
            None
        };
        if let Some(kind) = boundary {
            moments.push(build_moment(moments.len(), current_start, current_end, &current_texts, kind));
            current_start = next.start_s;
            current_texts = Vec::new();
        }
        current_texts.push(next.text.as_str());
        current_end = next.end_s;
    }
    moments.push(build_moment(moments.len(), current_start, current_end, &current_texts, BoundaryKind::Pause(gap_threshold_s)));

    moments
}

enum BoundaryKind {
    Pause(f64),
    Forced,
}

fn build_moment(index: usize, start_s: f64, end_s: f64, texts: &[&str], boundary: BoundaryKind) -> Moment {
    let excerpt = texts.join(" ");
    let title = texts
        .first()
        .map(|t| t.chars().take(60).collect::<String>())
        .unwrap_or_default();
    // A pause-detected boundary's confidence scales with how far past the
    // threshold the gap sits (diminishing returns, never a measured-accuracy
    // claim). A periodic fallback boundary carries a flat, low confidence—it
    // marks "no natural pause was found here," not a real detected topic shift.
    let confidence = match boundary {
        BoundaryKind::Pause(gap) => (gap / (gap + 2.0)).clamp(0.0, 1.0),
        BoundaryKind::Forced => 0.15,
    };
    Moment {
        index,
        start_s,
        end_s,
        title,
        confidence,
        transcript_excerpt: excerpt,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cue(start: f64, end: f64, text: &str) -> Cue {
        Cue {
            start_s: start,
            end_s: end,
            text: text.to_string(),
        }
    }

    #[test]
    fn no_cues_yields_no_moments() {
        assert_eq!(segment(&[]), vec![]);
    }

    #[test]
    fn consecutive_cues_with_no_gap_stay_one_moment() {
        let cues = vec![cue(0.0, 1.0, "a"), cue(1.0, 2.0, "b"), cue(2.0, 3.0, "c")];
        let moments = segment(&cues);
        assert_eq!(moments.len(), 1);
        assert_eq!(moments[0].start_s, 0.0);
        assert_eq!(moments[0].end_s, 3.0);
    }

    #[test]
    fn a_long_pause_splits_into_two_moments() {
        let cues = vec![cue(0.0, 1.0, "intro"), cue(10.0, 11.0, "next topic")];
        let moments = segment_with_params(&cues, 1.5, DEFAULT_MAX_SPAN_S);
        assert_eq!(moments.len(), 2);
        assert_eq!(moments[0].transcript_excerpt, "intro");
        assert_eq!(moments[1].transcript_excerpt, "next topic");
        assert_eq!(moments[1].start_s, 10.0);
    }

    #[test]
    fn moment_indices_are_sequential() {
        let cues = vec![cue(0.0, 1.0, "a"), cue(10.0, 11.0, "b"), cue(20.0, 21.0, "c")];
        let moments = segment_with_params(&cues, 1.5, DEFAULT_MAX_SPAN_S);
        let indices: Vec<usize> = moments.iter().map(|m| m.index).collect();
        assert_eq!(indices, vec![0, 1, 2]);
    }

    // Reproduces the real-world failure this fallback was added for: YouTube's
    // auto-generated captions overlap (each cue starts before the previous
    // cue's end), so a pure pause-gap heuristic never fires across an entire
    // continuously-narrated video and collapses everything into one moment.
    #[test]
    fn overlapping_rolling_captions_still_split_via_the_periodic_fallback() {
        let mut cues = Vec::new();
            let mut t = 0.0;
            while t < 300.0 {
                // each cue overlaps the previous one by 1s, mirroring YouTube's
                // rolling-caption timing
                cues.push(cue(t, t + 3.0, "word"));
                t += 2.0;
            }
        let moments = segment_with_params(&cues, 1.5, 60.0);
        assert!(
            moments.len() >= 4,
            "a 300s continuous transcript with a 60s max span should force at least 4 boundaries, got {}",
            moments.len()
        );
        for window in moments.windows(2) {
            assert!(window[1].start_s > window[0].start_s, "moments must be in increasing time order");
        }
    }

    #[test]
    fn a_forced_boundary_carries_lower_confidence_than_a_real_pause() {
        let cues = vec![cue(0.0, 1.0, "intro"), cue(10.0, 11.0, "next topic")];
        let pause_split = segment_with_params(&cues, 1.5, 1000.0);
        let forced_split = segment_with_params(&cues, 1000.0, 5.0);
        assert!(forced_split[0].confidence < pause_split[0].confidence);
    }
}
