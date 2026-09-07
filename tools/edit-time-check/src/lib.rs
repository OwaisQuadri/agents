mod boolean;
mod config;
mod registry;

pub use boolean::{boolean_findings, BooleanFinding};
pub use config::{Config, Options};
pub use registry::{evaluate, Stage};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path};

pub const MAX_REQUEST_BYTES: usize = 8 * 1024 * 1024;
pub const PROCESS_BUDGET_MS: u64 = 3000;
pub const RULE_BUDGET_MS: u64 = 500;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum Rule {
    Privacy,
    CommentLength,
    CommentShape,
    BooleanName,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum DiagnosticRule {
    Rule(Rule),
    Checker(Checker),
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Checker {
    Checker,
}

impl From<Rule> for DiagnosticRule {
    fn from(rule: Rule) -> Self {
        Self::Rule(rule)
    }
}

impl DiagnosticRule {
    pub const CHECKER: Self = Self::Checker(Checker::Checker);
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Operation {
    Edit,
    Write,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ByteRange {
    pub start_byte: usize,
    pub end_byte: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub version: u8,
    pub request_id: String,
    pub operation: Operation,
    pub path: String,
    #[serde(deserialize_with = "required_nullable")]
    pub repository_root: Option<String>,
    #[serde(deserialize_with = "required_nullable")]
    pub original_text: Option<String>,
    pub proposed_text: String,
    pub changed_ranges: Vec<ByteRange>,
    pub budget_ms: u64,
}

fn required_nullable<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Option::<String>::deserialize(d)
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Pass,
    Block,
    NeedsJudgment,
    Error,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Diagnostic {
    pub path: String,
    pub line: usize,
    pub rule: DiagnosticRule,
    pub reason: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JudgmentInput {
    pub comment: String,
    pub code_context: String,
    pub language: String,
    pub rule_document: String,
    pub prompt_version: u32,
    pub schema_version: u32,
    pub judgment_configuration: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JudgmentRequest {
    pub line: usize,
    pub input: JudgmentInput,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    pub version: u8,
    pub request_id: String,
    pub decision: Decision,
    pub diagnostics: Vec<Diagnostic>,
    pub judgments: Vec<JudgmentRequest>,
    pub elapsed_ms: u64,
    pub budget_ms: u64,
}

impl Response {
    pub fn error(
        request_id: String,
        path: String,
        rule: impl Into<DiagnosticRule>,
        reason: String,
    ) -> Self {
        Self {
            version: 1,
            request_id,
            decision: Decision::Error,
            diagnostics: vec![Diagnostic {
                path,
                line: 1,
                rule: rule.into(),
                reason,
            }],
            judgments: Vec::new(),
            elapsed_ms: 0,
            budget_ms: 20_000,
        }
    }

    pub fn is_valid(&self) -> bool {
        self.version == 1
            && self.budget_ms > 0
            && self.budget_ms <= 20_000
            && self.diagnostics.iter().all(|d| {
                d.line > 0
                    && !d.reason.is_empty()
                    && (d.rule != DiagnosticRule::CHECKER || self.decision == Decision::Error)
            })
            && self.judgments.iter().all(|j| {
                j.line > 0
                    && j.input.prompt_version == 1
                    && j.input.schema_version == 1
                    && !j.input.rule_document.is_empty()
            })
            && match self.decision {
                Decision::Pass => self.diagnostics.is_empty() && self.judgments.is_empty(),
                Decision::Block | Decision::Error => {
                    !self.diagnostics.is_empty() && self.judgments.is_empty()
                }
                Decision::NeedsJudgment => {
                    self.diagnostics.is_empty() && !self.judgments.is_empty()
                }
            }
    }
}

pub fn decode_request(bytes: &[u8]) -> Result<Request, String> {
    if bytes.len() > MAX_REQUEST_BYTES {
        return Err("request exceeds byte limit".into());
    }
    let request: Request =
        serde_json::from_slice(bytes).map_err(|_| "invalid request JSON or fields")?;
    if request.version != 1
        || request.request_id.is_empty()
        || request.request_id.len() > 128
        || !request
            .request_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.:".contains(&b))
        || !is_absolute_clean(&request.path)
        || request
            .repository_root
            .as_deref()
            .is_some_and(|root| !is_absolute_clean(root))
        || request.budget_ms == 0
        || request.budget_ms > 20_000
        || (request.operation == Operation::Edit && request.original_text.is_none())
    {
        return Err("invalid request version, identity, path, operation, or budget".into());
    }
    let mut end = 0;
    for (index, range) in request.changed_ranges.iter().enumerate() {
        if range.start_byte > range.end_byte
            || range.end_byte > request.proposed_text.len()
            || !request.proposed_text.is_char_boundary(range.start_byte)
            || !request.proposed_text.is_char_boundary(range.end_byte)
            || (index > 0 && range.start_byte < end)
        {
            return Err("invalid changed range".into());
        }
        end = range.end_byte;
    }
    Ok(request)
}

pub(crate) fn is_absolute_clean(path: &str) -> bool {
    Path::new(path).is_absolute()
        && !path.contains('\0')
        && !Path::new(path)
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
}

pub fn changed_ranges(
    original: Option<&str>,
    proposed: &str,
    operation: Operation,
) -> Vec<ByteRange> {
    if operation == Operation::Write || original.is_none() {
        return vec![ByteRange {
            start_byte: 0,
            end_byte: proposed.len(),
        }];
    }
    let old: Vec<&str> = original.unwrap_or_default().split_inclusive('\n').collect();
    let new: Vec<&str> = proposed.split_inclusive('\n').collect();
    let mut offsets = vec![0];
    for line in &new {
        offsets.push(offsets.last().copied().unwrap_or(0) + line.len());
    }
    let mut ranges = Vec::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(100);
    for op in similar::capture_diff_slices_deadline(
        similar::Algorithm::Patience,
        &old,
        &new,
        Some(deadline),
    ) {
        if op.tag() == similar::DiffTag::Equal {
            continue;
        }
        let old_lines = &old[op.old_range()];
        let new_range = op.new_range();
        let new_lines = &new[new_range.clone()];
        let paired = old_lines.len().min(new_lines.len());
        for index in 0..paired {
            refine_line(
                old_lines[index],
                new_lines[index],
                offsets[new_range.start + index],
                &mut ranges,
            );
        }
        if old_lines.len() != new_lines.len() {
            ranges.push(ByteRange {
                start_byte: offsets[new_range.start + paired],
                end_byte: offsets[new_range.end],
            });
        }
    }
    ranges
}

fn refine_line(old: &str, new: &str, offset: usize, ranges: &mut Vec<ByteRange>) {
    let old: Vec<char> = old.chars().collect();
    let new_chars: Vec<char> = new.chars().collect();
    let offsets: Vec<usize> = new
        .char_indices()
        .map(|(i, _)| offset + i)
        .chain(std::iter::once(offset + new.len()))
        .collect();
    let length = old.len().max(new_chars.len());
    for start in (0..length).step_by(256) {
        let old_chunk = &old[start.min(old.len())..(start + 256).min(old.len())];
        let new_start = start.min(new_chars.len());
        let new_chunk = &new_chars[new_start..(start + 256).min(new_chars.len())];
        for op in similar::capture_diff_slices(similar::Algorithm::Myers, old_chunk, new_chunk) {
            if op.tag() != similar::DiffTag::Equal {
                let range = op.new_range();
                ranges.push(ByteRange {
                    start_byte: offsets[new_start + range.start],
                    end_byte: offsets[new_start + range.end],
                });
            }
        }
    }
}

pub(crate) fn touches(ranges: &[ByteRange], start: usize, end: usize) -> bool {
    let index = ranges.partition_point(|r| r.end_byte < start);
    ranges[index..]
        .iter()
        .take_while(|r| r.start_byte <= end)
        .any(|r| {
            if r.start_byte == r.end_byte {
                start <= r.start_byte && r.start_byte <= end
            } else {
                r.start_byte < end && start < r.end_byte
            }
        })
}
