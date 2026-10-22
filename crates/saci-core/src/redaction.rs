//! Secret redaction and bounded output capture.
//!
//! Every observation is redacted before it is stored or shown to the model.
//! Redaction replaces known secret values with a marker naming the secret,
//! and masks headers that carry credentials whatever their value. It is a
//! safety net for accidental disclosure, not a guarantee: a secret that a
//! command transforms (encodes, splits, hashes) is not recognized.

use std::borrow::Cow;
use std::collections::BTreeMap;

use serde_json::Value;

use crate::event::EventKind;
use crate::evidence::{CapturedText, EvidenceRecord, HttpEvidence};
use crate::investigation::InvestigationReport;

/// Replacement for a masked value whose secret is not known by name.
pub const REDACTED: &str = "[REDACTED]";

/// Shortest secret value that is redacted. Shorter values would mask
/// ordinary text; configuration rejects them instead.
pub const MIN_SECRET_LEN: usize = 4;

/// Header names whose values are always masked.
const SENSITIVE_HEADERS: [&str; 9] = [
    "authorization",
    "proxy-authorization",
    "cookie",
    "set-cookie",
    "x-api-key",
    "api-key",
    "x-auth-token",
    "x-access-token",
    "x-csrf-token",
];

/// Whether a header carries credentials and must always be masked.
#[must_use]
pub fn is_sensitive_header(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    SENSITIVE_HEADERS.contains(&lower.as_str())
}

/// Known secret values and their markers.
#[derive(Clone, Default)]
pub struct Redactor {
    /// `(value, marker)`, longest value first so overlapping secrets redact
    /// completely.
    values: Vec<(String, String)>,
}

impl std::fmt::Debug for Redactor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Redactor")
            .field("secrets", &self.values.len())
            .finish()
    }
}

impl Redactor {
    /// A redactor that knows no secrets yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers `value` under `name`. Returns `false`, registering nothing,
    /// when the value is shorter than [`MIN_SECRET_LEN`].
    pub fn add(&mut self, name: &str, value: &str) -> bool {
        if value.chars().count() < MIN_SECRET_LEN {
            return false;
        }
        let marker = format!("[REDACTED:{name}]");
        if !self.values.iter().any(|(known, _)| known == value) {
            self.values.push((value.to_owned(), marker));
            self.values
                .sort_by_key(|(known, _)| std::cmp::Reverse(known.len()));
        }
        true
    }

    /// Length in bytes of the longest registered secret.
    #[must_use]
    pub fn longest(&self) -> usize {
        self.values.first().map_or(0, |(value, _)| value.len())
    }

    /// Replaces every registered secret in `text`.
    #[must_use]
    pub fn redact<'a>(&self, text: &'a str) -> Cow<'a, str> {
        let mut result = Cow::Borrowed(text);
        for (value, marker) in &self.values {
            if result.contains(value.as_str()) {
                result = Cow::Owned(result.replace(value.as_str(), marker));
            }
        }
        result
    }

    /// Redacts a string in place.
    pub fn redact_string(&self, text: &mut String) {
        if let Cow::Owned(redacted) = self.redact(text) {
            *text = redacted;
        }
    }

    /// Redacts every string, and every object key, in a JSON value.
    pub fn redact_json(&self, value: &mut Value) {
        match value {
            Value::String(text) => self.redact_string(text),
            Value::Array(items) => items.iter_mut().for_each(|item| self.redact_json(item)),
            Value::Object(map) => {
                let entries = std::mem::take(map);
                for (key, mut item) in entries {
                    self.redact_json(&mut item);
                    map.insert(self.redact(&key).into_owned(), item);
                }
            }
            Value::Null | Value::Bool(_) | Value::Number(_) => {}
        }
    }

    /// Masks credential headers and redacts secrets in the others.
    pub fn redact_headers(&self, headers: &mut BTreeMap<String, String>) {
        for (name, value) in headers.iter_mut() {
            if is_sensitive_header(name) {
                REDACTED.clone_into(value);
            } else {
                self.redact_string(value);
            }
        }
    }

    /// Redacts captured text. A redacted capture keeps its truncation flag
    /// and original size.
    pub fn redact_captured(&self, captured: &mut CapturedText) {
        self.redact_string(&mut captured.text);
    }

    /// Redacts every free-text field of an event: observations, assessment
    /// and criterion explanations, error messages and the reasons a unit was
    /// skipped or a run ended.
    pub fn redact_event(&self, kind: &mut EventKind) {
        match kind {
            EventKind::Observation { evidence } => self.redact_evidence(&mut evidence.record),
            EventKind::AttemptFinished {
                assessment, error, ..
            } => {
                if let Some(assessment) = assessment {
                    self.redact_string(&mut assessment.explanation);
                    for criterion in &mut assessment.criteria {
                        self.redact_string(&mut criterion.explanation);
                    }
                }
                if let Some(error) = error {
                    self.redact_string(&mut error.message);
                }
            }
            EventKind::UnitSkipped { reason } | EventKind::RunFinished { reason, .. } => {
                self.redact_string(reason);
            }
            EventKind::InvestigationConcluded { report } => self.redact_investigation(report),
            EventKind::RunStarted { .. }
            | EventKind::AttemptStarted
            | EventKind::UnitFinished { .. }
            | EventKind::CancellationRequested => {}
        }
    }

    /// Redacts the free text of an investigation result.
    fn redact_investigation(&self, report: &mut InvestigationReport) {
        self.redact_string(&mut report.summary);
        for hypothesis in &mut report.hypotheses {
            self.redact_string(&mut hypothesis.statement);
        }
        if let Some(bug) = &mut report.bug {
            self.redact_string(&mut bug.url);
            self.redact_string(&mut bug.title);
        }
        if let Some(reason) = &mut report.no_bug_reason {
            self.redact_string(reason);
        }
    }

    /// Redacts every field of an observation that can carry a secret.
    pub fn redact_evidence(&self, record: &mut EvidenceRecord) {
        match record {
            EvidenceRecord::Shell(shell) => {
                self.redact_string(&mut shell.command);
                self.redact_captured(&mut shell.stdout);
                self.redact_captured(&mut shell.stderr);
            }
            EvidenceRecord::Http(http) => self.redact_http(http),
            EvidenceRecord::Mcp(mcp) => {
                self.redact_json(&mut mcp.arguments);
                self.redact_captured(&mut mcp.result);
                if let Some(error) = &mut mcp.protocol_error {
                    self.redact_string(error);
                }
            }
            EvidenceRecord::Rejected(rejected) => {
                self.redact_string(&mut rejected.arguments);
                self.redact_string(&mut rejected.reason);
            }
        }
    }

    fn redact_http(&self, http: &mut HttpEvidence) {
        self.redact_string(&mut http.url);
        self.redact_headers(&mut http.request_headers);
        if let Some(body) = &mut http.request_body {
            self.redact_captured(body);
        }
        if let Some(response) = &mut http.response {
            self.redact_headers(&mut response.headers);
            self.redact_captured(&mut response.body);
            if let Some(error) = &mut response.body_error {
                self.redact_string(error);
            }
        }
        if let Some(error) = &mut http.transport_error {
            self.redact_string(error);
        }
    }
}

/// Collects a byte stream, keeping at most `limit` bytes of it.
///
/// A little more than `limit` is buffered so a secret straddling the cut is
/// still recognized; redaction happens before the final truncation.
#[derive(Debug, Clone)]
pub struct OutputCapture {
    limit: usize,
    keep: usize,
    buffer: Vec<u8>,
    total: u64,
}

impl OutputCapture {
    /// A capture keeping `limit` bytes, buffering enough extra to redact
    /// secrets known to `redactor`.
    #[must_use]
    pub fn new(limit: u64, redactor: &Redactor) -> Self {
        let limit = usize::try_from(limit).unwrap_or(usize::MAX);
        Self {
            limit,
            keep: limit.saturating_add(redactor.longest()),
            buffer: Vec::new(),
            total: 0,
        }
    }

    /// Appends a chunk; bytes beyond the buffer are counted and dropped.
    pub fn push(&mut self, chunk: &[u8]) {
        self.total = self.total.saturating_add(chunk.len() as u64);
        let room = self.keep.saturating_sub(self.buffer.len());
        self.buffer
            .extend_from_slice(chunk.get(..room.min(chunk.len())).unwrap_or(chunk));
    }

    /// Bytes received so far.
    #[must_use]
    pub const fn total(&self) -> u64 {
        self.total
    }

    /// Redacts and truncates the captured bytes. Invalid UTF-8 is replaced.
    #[must_use]
    pub fn finish(self, redactor: &Redactor) -> CapturedText {
        let lossy = String::from_utf8_lossy(&self.buffer);
        let redacted = redactor.redact(&lossy);
        let dropped = self.total > self.buffer.len() as u64;
        let (text, cut) = truncate(&redacted, self.limit);
        CapturedText {
            text: text.to_owned(),
            truncated: dropped || cut,
            total_bytes: self.total,
        }
    }
}

/// Redacts and bounds a complete text.
#[must_use]
pub fn capture_text(text: &str, limit: u64, redactor: &Redactor) -> CapturedText {
    let mut capture = OutputCapture::new(limit, redactor);
    capture.push(text.as_bytes());
    capture.finish(redactor)
}

fn truncate(text: &str, limit: usize) -> (&str, bool) {
    if text.len() <= limit {
        return (text, false);
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (&text[..end], true)
}

#[cfg(test)]
mod tests;
