use crate::config::read_required;
use crate::{
    boolean_findings, changed_ranges, touches, Config, Decision, Diagnostic, DiagnosticRule,
    JudgmentInput, JudgmentRequest, Options, Request, Response, Rule,
};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone)]
pub struct Stage(Arc<Mutex<Timing>>);
struct Timing {
    started: Instant,
    rule_started: Instant,
    rule: Rule,
    rule_limit: Option<u64>,
    process_limit: u64,
    total_limit: u64,
    request_id: String,
    path: String,
}
impl Default for Stage {
    fn default() -> Self {
        Self::new()
    }
}
impl Stage {
    pub fn new() -> Self {
        let started = Instant::now();
        Self(Arc::new(Mutex::new(Timing {
            started,
            rule_started: started,
            rule: Rule::Privacy,
            rule_limit: None,
            process_limit: 3000,
            total_limit: 20000,
            request_id: "invalid-request".into(),
            path: "/".into(),
        })))
    }
    pub fn bind(&self, request: &Request) {
        let mut timing = self.0.lock().unwrap_or_else(|e| e.into_inner());
        timing.request_id = request.request_id.clone();
        timing.path = request.path.clone();
        timing.total_limit = request.budget_ms;
        timing.process_limit = timing.process_limit.min(request.budget_ms);
    }
    pub fn error(&self, rule: impl Into<DiagnosticRule>, reason: String) -> Response {
        let timing = self.0.lock().unwrap_or_else(|e| e.into_inner());
        Response::error(timing.request_id.clone(), timing.path.clone(), rule, reason)
    }
    fn configure(&self, config: &Config, request: &Request) {
        let mut timing = self.0.lock().unwrap_or_else(|e| e.into_inner());
        timing.total_limit = request.budget_ms.min(config.total_ms).min(20_000);
        timing.process_limit = config.process_budget_ms.min(timing.total_limit);
    }
    fn enter(&self, rule: Rule, limit: u64) {
        let mut timing = self.0.lock().unwrap_or_else(|e| e.into_inner());
        timing.rule = rule;
        timing.rule_started = Instant::now();
        timing.rule_limit = Some(limit);
    }
    pub fn expired(&self) -> Option<(DiagnosticRule, String)> {
        let timing = self.0.lock().unwrap_or_else(|e| e.into_inner());
        Self::expired_timing(&timing)
    }
    fn complete_rule(&self) -> Result<(), (DiagnosticRule, String)> {
        let mut timing = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(error) = Self::expired_timing(&timing) {
            return Err(error);
        }
        timing.rule_limit = None;
        Ok(())
    }
    fn expired_timing(timing: &Timing) -> Option<(DiagnosticRule, String)> {
        let elapsed = timing.started.elapsed();
        if elapsed >= Duration::from_millis(timing.process_limit.min(timing.total_limit)) {
            return Some((
                DiagnosticRule::CHECKER,
                format!("checker deadline exceeded after {} ms", elapsed.as_millis()),
            ));
        }
        if timing
            .rule_limit
            .is_some_and(|limit| timing.rule_started.elapsed() >= Duration::from_millis(limit))
        {
            return Some((
                timing.rule.into(),
                format!(
                    "rule deadline exceeded after {} ms",
                    timing.rule_started.elapsed().as_millis()
                ),
            ));
        }
        None
    }
    pub fn finish(&self, response: &mut Response) {
        let timing = self.0.lock().unwrap_or_else(|e| e.into_inner());
        response.elapsed_ms = timing.started.elapsed().as_millis() as u64;
        response.budget_ms = timing.total_limit;
    }
}

pub fn evaluate(request: &Request, options: &Options, stage: &Stage) -> Response {
    stage.bind(request);
    let result = evaluate_inner(request, options, stage);
    let mut response = match result {
        Ok(response) => response,
        Err((rule, reason)) => Response::error(
            request.request_id.clone(),
            request.path.clone(),
            rule,
            reason,
        ),
    };
    if let Some((rule, reason)) = stage.expired() {
        response = Response::error(
            request.request_id.clone(),
            request.path.clone(),
            rule,
            reason,
        );
    }
    stage.finish(&mut response);
    response
}

fn evaluate_inner(
    request: &Request,
    options: &Options,
    stage: &Stage,
) -> Result<Response, (DiagnosticRule, String)> {
    let (config, relative) = options
        .resolve(Path::new(&request.path), request.repository_root.as_deref())
        .map_err(|e| (DiagnosticRule::CHECKER, e))?;
    stage.configure(&config, request);
    let mut response = Response {
        version: 1,
        request_id: request.request_id.clone(),
        decision: Decision::Pass,
        diagnostics: Vec::new(),
        judgments: Vec::new(),
        elapsed_ms: 0,
        budget_ms: request.budget_ms.min(config.total_ms),
    };
    if !config
        .is_selected(&relative)
        .map_err(|e| (DiagnosticRule::CHECKER, e))?
        || config.rules.is_empty()
    {
        return Ok(response);
    }
    let ranges = changed_ranges(
        request.original_text.as_deref(),
        &request.proposed_text,
        request.operation,
    );
    if let Some(error) = stage.expired() {
        return Err(error);
    }
    let lines = Lines::new(&request.proposed_text);
    let mut output_bytes = 1024usize;
    let mut comment_document = None;
    let mut comment_spans = None;
    let judgment_configuration =
        serde_json::to_string(&(options.judgment_configuration.as_str(), &config)).map_err(
            |_| {
                (
                    Rule::CommentShape.into(),
                    "cannot encode judgment configuration".into(),
                )
            },
        )?;
    for rule in &config.rules {
        stage.enter(*rule, config.rule_limit(*rule));
        match rule {
            Rule::Privacy => {
                let names = identifiers(options).map_err(|e| ((*rule).into(), e))?;
                for (index, line) in lines.text.iter().enumerate() {
                    if touches(&ranges, lines.offsets[index], lines.offsets[index + 1])
                        && !privacy_lint::scan_line(line, index + 1, &names).is_empty()
                    {
                        response.diagnostics.push(diagnostic(
                            request,
                            index + 1,
                            *rule,
                            "Remove the private identifier; use a public placeholder.",
                        ));
                    }
                }
            }
            Rule::CommentLength | Rule::CommentShape => {
                if comment_document.is_none() {
                    comment_document = Some(
                        read_required(&options.rule_document).map_err(|e| ((*rule).into(), e))?,
                    );
                }
                let document = comment_document
                    .as_ref()
                    .ok_or(((*rule).into(), "missing comment rules".into()))?;
                let language = language_label(&request.path, &request.proposed_text);
                let fixed_judgment_bytes = 256
                    + json_string_bound(document)
                    + json_string_bound(&judgment_configuration)
                    + json_string_bound(&language);
                let spans = comment_spans.get_or_insert_with(|| {
                    comment_check::extract(&request.path, &request.proposed_text)
                        .unwrap_or_default()
                });
                for span in spans.iter() {
                    let start = span.start_line - 1;
                    let end = span.end_line;
                    let context_end = (end + 3).min(lines.text.len());
                    if !touches(&ranges, lines.offsets[start], lines.offsets[end])
                        && !touches(&ranges, lines.offsets[end], lines.offsets[context_end])
                    {
                        continue;
                    }
                    if *rule == Rule::CommentLength {
                        if span.kind == comment_check::CommentKind::Plain
                            && end - start > comment_check::MAX_COMMENT_LINES
                        {
                            response.diagnostics.push(diagnostic(request, start + 1, *rule, "Shorten this non-documentation comment to three lines or remove it."));
                        }
                    } else {
                        if language == "rs"
                            && span.is_full_line
                            && is_empty_rust_comment(&lines.text[start..end])
                        {
                            response.diagnostics.push(diagnostic(
                                request,
                                start + 1,
                                *rule,
                                "Remove the empty comment; it cannot match an approved shape.",
                            ));
                            continue;
                        }
                        let context = lines.text[end..context_end]
                            .iter()
                            .copied()
                            .filter(|s| !s.trim().is_empty())
                            .collect::<Vec<_>>()
                            .join("\n");
                        let judgment = JudgmentRequest {
                            line: start + 1,
                            input: JudgmentInput {
                                comment: lines.text[start..end].join("\n"),
                                code_context: context,
                                language: language.clone(),
                                rule_document: document.clone(),
                                prompt_version: 1,
                                schema_version: 1,
                                judgment_configuration: judgment_configuration.clone(),
                            },
                        };
                        let size = fixed_judgment_bytes
                            + json_string_bound(&judgment.input.comment)
                            + json_string_bound(&judgment.input.code_context);
                        output_bytes = output_bytes.saturating_add(size + 1);
                        if output_bytes > crate::MAX_REQUEST_BYTES {
                            return Err(((*rule).into(), "judgments exceed output limit".into()));
                        }
                        response.judgments.push(judgment);
                    }
                }
            }
            Rule::BooleanName => {
                read_required(&options.code_style).map_err(|e| ((*rule).into(), e))?;
                for finding in boolean_findings(&request.path, &request.proposed_text)
                    .map_err(|e| ((*rule).into(), e))?
                {
                    if touches(&ranges, finding.start_byte, finding.end_byte)
                        || touches(
                            &ranges,
                            finding.evidence.start_byte,
                            finding.evidence.end_byte,
                        )
                    {
                        response.diagnostics.push(diagnostic(
                            request,
                            finding.line,
                            *rule,
                            "Rename this proven Boolean declaration with an is prefix.",
                        ));
                    }
                }
            }
        }
        stage.complete_rule()?;
    }
    if !response.diagnostics.is_empty() {
        response.decision = Decision::Block;
        response.judgments.clear();
    } else if !response.judgments.is_empty() {
        response.decision = Decision::NeedsJudgment;
    }
    Ok(response)
}

fn is_empty_rust_comment(lines: &[&str]) -> bool {
    if lines.iter().all(|line| {
        line.trim().strip_prefix("//").is_some_and(|content| {
            content
                .strip_prefix('/')
                .or_else(|| content.strip_prefix('!'))
                .unwrap_or(content)
                .trim()
                .is_empty()
        })
    }) {
        return true;
    }
    let text = lines.join("\n");
    text.trim()
        .strip_prefix("/*")
        .and_then(|content| content.strip_suffix("*/"))
        .is_some_and(|content| {
            content
                .strip_prefix('*')
                .or_else(|| content.strip_prefix('!'))
                .unwrap_or(content)
                .trim()
                .is_empty()
        })
}

fn json_string_bound(text: &str) -> usize {
    2 + text
        .bytes()
        .map(|b| match b {
            b'"' | b'\\' => 2,
            0..=31 => 6,
            _ => 1,
        })
        .sum::<usize>()
}

fn identifiers(options: &Options) -> Result<Vec<String>, String> {
    if let Some(path) = options.identifiers.as_ref() {
        return privacy_lint::read_local_names(path)
            .map_err(|_| "cannot read required private identifier configuration".into());
    }
    let path = if let Some(path) = std::env::var_os("PRIVACY_LINT_IDENTIFIERS") {
        return privacy_lint::read_local_names(Path::new(&path))
            .map_err(|_| "cannot read required private identifier configuration".into());
    } else {
        std::env::var_os("HOME")
            .map(|home| Path::new(&home).join(".config/privacy-lint/identifiers"))
            .ok_or("cannot locate private identifier configuration")?
    };
    match std::fs::symlink_metadata(&path) {
        Ok(_) => privacy_lint::read_local_names(&path)
            .map_err(|_| "cannot read present private identifier configuration".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(_) => Err("cannot inspect private identifier configuration".into()),
    }
}

fn diagnostic(request: &Request, line: usize, rule: Rule, reason: &str) -> Diagnostic {
    Diagnostic {
        path: request.path.clone(),
        line,
        rule: rule.into(),
        reason: reason.into(),
    }
}

fn language_label(path: &str, text: &str) -> String {
    Path::new(path)
        .extension()
        .and_then(|s| s.to_str())
        .map(str::to_owned)
        .unwrap_or_else(|| {
            let first = text.lines().next().unwrap_or_default();
            for language in ["python", "zsh", "ruby", "perl", "sh"] {
                if first.starts_with("#!") && first.contains(language) {
                    return language.into();
                }
            }
            "text".into()
        })
}

struct Lines<'a> {
    text: Vec<&'a str>,
    offsets: Vec<usize>,
}
impl<'a> Lines<'a> {
    fn new(text: &'a str) -> Self {
        let mut offsets = vec![0];
        for (index, byte) in text.bytes().enumerate() {
            if byte == b'\n' {
                offsets.push(index + 1);
            }
        }
        let lines: Vec<_> = text.lines().collect();
        offsets.truncate(lines.len());
        offsets.push(text.len());
        Self {
            text: lines,
            offsets,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_rule_does_not_expire_during_delivery() {
        let stage = Stage::new();
        stage.enter(Rule::BooleanName, 500);
        stage.complete_rule().unwrap();
        {
            let mut timing = stage.0.lock().unwrap();
            timing.rule_started = Instant::now() - Duration::from_secs(1);
        }
        assert!(stage.expired().is_none());
        {
            let mut timing = stage.0.lock().unwrap();
            timing.started = Instant::now() - Duration::from_secs(4);
        }
        assert_eq!(stage.expired().unwrap().0, DiagnosticRule::CHECKER);
    }

    #[test]
    fn completion_cannot_clear_an_expired_rule() {
        let stage = Stage::new();
        stage.enter(Rule::BooleanName, 500);
        stage.0.lock().unwrap().rule_started = Instant::now() - Duration::from_secs(1);
        assert_eq!(
            stage.complete_rule().unwrap_err().0,
            Rule::BooleanName.into()
        );
        assert_eq!(stage.expired().unwrap().0, Rule::BooleanName.into());
    }
}
