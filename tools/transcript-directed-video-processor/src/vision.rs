// Calls a configured vision-capable multimodal model to review one already-
// extracted frame. Uses the `genai` crate (rust-genai) as a single multi-provider
// client — model selection is a plain model-name string (e.g. "gpt-5.1",
// "claude-sonnet-4-5", "gemini-2.5-pro") that genai routes to the right provider
// adapter, per research, rather than hand-writing three separate provider clients.
//
// This module makes a real network call and is exercised only by the manual-test
// checklist, not by `cargo test` — no API key is assumed present in CI(continuous
// integration), and unit tests instead cover the deterministic parts (prompt/
// message construction) via `build_review_request`.

use genai::Client;
use genai::chat::{ChatMessage, ChatRequest, ContentPart};

pub fn build_review_request(prompt: &str, frame_path: &str) -> Result<ChatRequest, String> {
    let part = ContentPart::from_binary_file(frame_path)
        .map_err(|error| format!("failed to read frame {frame_path}: {error}"))?;
    Ok(ChatRequest::default()
        .with_system("You are reviewing a single extracted video frame to confirm or refute a visual claim. Be concise and literal about what is visible.")
        .append_message(ChatMessage::user(vec![ContentPart::from_text(prompt), part])))
}

pub async fn review_frame(model: &str, prompt: &str, frame_path: &str) -> Result<String, String> {
    let request = build_review_request(prompt, frame_path)?;
    let client = Client::default();
    let response = client
        .exec_chat(model, request, None)
        .await
        .map_err(|error| format!("vision model call failed: {error}"))?;
    response
        .content
        .joined_texts()
        .map(|text| text.to_string())
        .ok_or_else(|| "vision model returned no text content".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_review_request_errors_clearly_on_missing_frame() {
        let error = build_review_request("what is this?", "/nonexistent/frame.jpg").unwrap_err();
        assert!(error.contains("/nonexistent/frame.jpg"));
    }
}
