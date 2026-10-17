//! Deterministic parsing and validation of Markdown test plans.
//!
//! The conventions are specified in `docs/test-plans.md`. The parser finds
//! the title, sections, phases, tests and steps, actions, fenced commands,
//! expected-result criteria, verification instructions and procedure
//! invocations. It never interprets the prose itself.

mod blocks;

use std::collections::BTreeMap;
use std::fmt;

use blocks::{Block, BlockKind, Paragraph};

use crate::plan::{
    CodeBlock, Criterion, Phase, Plan, ProcedureRef, SourceLocation, Unit, UnitKind,
};

/// Whether a document is the plan entry point or an invoked procedure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentRole {
    /// Titled `# Test Plan: <title>`.
    Entry,
    /// Titled `# Procedure: <title>`.
    Procedure,
}

impl DocumentRole {
    const fn title_prefix(self) -> &'static str {
        match self {
            Self::Entry => "Test Plan:",
            Self::Procedure => "Procedure:",
        }
    }
}

/// A problem found in a submitted document, with its location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// Where the problem is.
    pub location: SourceLocation,
    /// What is wrong and how to fix it.
    pub message: String,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.location, self.message)
    }
}

/// A document failed validation.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{}", self.diagnostics.iter().map(ToString::to_string).collect::<Vec<_>>().join("\n"))]
pub struct PlanError {
    /// Every problem found, in source order.
    pub diagnostics: Vec<Diagnostic>,
}

/// Parses and validates one document.
///
/// `document` is its bundle-relative path, used in locations.
///
/// # Errors
///
/// Returns every structural problem found: a missing or malformed title,
/// malformed or duplicate headings, units outside phases, missing actions,
/// missing expected results, or no executable units at all.
pub fn parse_document(document: &str, source: &str, role: DocumentRole) -> Result<Plan, PlanError> {
    let mut state = State::new(document, source);
    state.run(role, &blocks::blocks(source));
    state.finish()
}

/// Line-number lookup for byte offsets.
struct Lines(Vec<usize>);

impl Lines {
    fn new(source: &str) -> Self {
        let starts = std::iter::once(0)
            .chain(source.match_indices('\n').map(|(index, _)| index + 1))
            .collect();
        Self(starts)
    }

    fn line(&self, offset: usize) -> usize {
        self.0.partition_point(|start| *start <= offset)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Preamble,
    Overview,
    Prerequisites,
    Phase,
    Other,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum UnitPart {
    Action,
    Expected,
    Verification,
}

struct OpenUnit {
    unit: Unit,
    body_start: usize,
    part: UnitPart,
    action_end: Option<usize>,
    expected_marker: Option<SourceLocation>,
    verification_start: Option<usize>,
    end: usize,
}

struct State<'a> {
    document: &'a str,
    source: &'a str,
    lines: Lines,
    title: Option<String>,
    section: Section,
    section_start: usize,
    overview: Option<String>,
    prerequisites: Option<String>,
    phases: Vec<Phase>,
    unit: Option<OpenUnit>,
    diagnostics: Vec<Diagnostic>,
}

impl<'a> State<'a> {
    fn new(document: &'a str, source: &'a str) -> Self {
        Self {
            document,
            source,
            lines: Lines::new(source),
            title: None,
            section: Section::Preamble,
            section_start: 0,
            overview: None,
            prerequisites: None,
            phases: Vec::new(),
            unit: None,
            diagnostics: Vec::new(),
        }
    }

    fn location(&self, offset: usize) -> SourceLocation {
        SourceLocation::new(self.document, self.lines.line(offset))
    }

    fn error(&mut self, offset: usize, message: impl Into<String>) {
        let location = self.location(offset);
        self.diagnostics.push(Diagnostic {
            location,
            message: message.into(),
        });
    }

    fn run(&mut self, role: DocumentRole, blocks: &[Block]) {
        for block in blocks {
            match &block.kind {
                BlockKind::Heading { level, text } => {
                    self.heading(role, *level, text, block.range.start);
                }
                _ => self.body_block(block),
            }
        }
        self.close_unit(self.source.len());
        self.close_section(self.source.len());
    }

    fn finish(mut self) -> Result<Plan, PlanError> {
        if self.title.is_none() {
            self.error(
                0,
                "missing title: start the document with `# Test Plan: <title>`",
            );
        }
        if self.phases.iter().all(|phase| phase.units.is_empty()) {
            self.error(0, "no executable units: add a `### Test <id>: <title>` or `### Step <id>: <title>` inside a `## Phase <id>: <title>`");
        }
        self.check_duplicates();
        if !self.diagnostics.is_empty() {
            self.diagnostics
                .sort_by_key(|diagnostic| diagnostic.location.line);
            return Err(PlanError {
                diagnostics: self.diagnostics,
            });
        }
        Ok(Plan {
            title: self.title.unwrap_or_default(),
            document: self.document.to_owned(),
            overview: self.overview,
            prerequisites: self.prerequisites,
            phases: self.phases,
        })
    }

    fn check_duplicates(&mut self) {
        let mut units: BTreeMap<String, SourceLocation> = BTreeMap::new();
        let mut duplicates = Vec::new();
        for unit in self.phases.iter().flat_map(|phase| &phase.units) {
            if let Some(first) = units.get(&unit.id) {
                duplicates.push(Diagnostic {
                    location: unit.location.clone(),
                    message: format!(
                        "duplicate unit ID `{}` (first used at line {})",
                        unit.id, first.line
                    ),
                });
            } else {
                units.insert(unit.id.clone(), unit.location.clone());
            }
        }
        self.diagnostics.extend(duplicates);
    }

    fn heading(&mut self, role: DocumentRole, level: u8, text: &str, offset: usize) {
        match level {
            1 => self.title_heading(role, text, offset),
            2 => {
                self.close_unit(offset);
                self.close_section(offset);
                self.section_heading(text, offset);
            }
            3 => {
                self.close_unit(offset);
                self.unit_heading(text, offset);
            }
            _ => self.extend_unit(offset),
        }
    }

    fn title_heading(&mut self, role: DocumentRole, text: &str, offset: usize) {
        let prefix = role.title_prefix();
        if self.title.is_some() {
            self.error(offset, "only one `#` title is allowed");
            return;
        }
        match text.strip_prefix(prefix).map(str::trim) {
            Some(title) if !title.is_empty() => self.title = Some(title.to_owned()),
            _ => self.error(
                offset,
                format!("malformed title: expected `# {prefix} <title>`"),
            ),
        }
    }

    fn section_heading(&mut self, text: &str, offset: usize) {
        self.section_start = offset;
        self.section = match text {
            "Overview" => Section::Overview,
            "Prerequisites" => Section::Prerequisites,
            _ if matches!(first_word(text), "Phase" | "Phase:") => {
                match split_labeled(text, "Phase") {
                    Some((id, title)) => {
                        let location = self.location(offset);
                        self.phases.push(Phase {
                            id,
                            title,
                            location,
                            units: Vec::new(),
                        });
                        Section::Phase
                    }
                    None => {
                        self.error(
                            offset,
                            "malformed phase heading: expected `## Phase <id>: <title>`",
                        );
                        Section::Other
                    }
                }
            }
            _ => Section::Other,
        };
    }

    fn close_section(&mut self, end: usize) {
        let body = self.section_body(end);
        match self.section {
            Section::Overview => self.overview = body,
            Section::Prerequisites => self.prerequisites = body,
            _ => {}
        }
        self.section = Section::Other;
    }

    fn section_body(&self, end: usize) -> Option<String> {
        let start = self.source[self.section_start..end]
            .find('\n')
            .map_or(end, |newline| self.section_start + newline + 1);
        let body = self.source[start.min(end)..end].trim();
        (!body.is_empty()).then(|| body.to_owned())
    }

    fn unit_heading(&mut self, text: &str, offset: usize) {
        let kind = match first_word(text) {
            "Test" | "Test:" => UnitKind::Test,
            "Step" | "Step:" => UnitKind::Step,
            _ => return,
        };
        let label = label(kind);
        if !looks_like_unit(text) {
            return;
        }
        let Some((id, title)) = split_labeled(text, label) else {
            self.error(
                offset,
                format!("malformed heading: expected `### {label} <id>: <title>`"),
            );
            return;
        };
        if self.section != Section::Phase {
            self.error(offset, format!("{label} {id} is outside a phase: add a `## Phase <id>: <title>` heading before it"));
            return;
        }
        let location = self.location(offset);
        self.unit = Some(OpenUnit::new(
            kind,
            id,
            title,
            location,
            self.line_end(offset),
        ));
    }

    fn line_end(&self, offset: usize) -> usize {
        self.source[offset..]
            .find('\n')
            .map_or(self.source.len(), |newline| offset + newline + 1)
    }

    const fn extend_unit(&mut self, offset: usize) {
        if let Some(open) = self.unit.as_mut() {
            open.end = offset;
        }
    }

    fn body_block(&mut self, block: &Block) {
        let location = self.location(block.range.start);
        let Some(open) = self.unit.as_mut() else {
            return;
        };
        open.end = block.range.end;
        if let BlockKind::Paragraph(paragraph) = &block.kind
            && let Some(marker) = Marker::of(paragraph)
        {
            if let Err(message) = open.marker(marker, block, location) {
                self.error(block.range.start, message);
            }
            return;
        }
        open.content(block, location);
    }

    fn close_unit(&mut self, end: usize) {
        let Some(mut open) = self.unit.take() else {
            return;
        };
        let end = open.end.max(open.body_start).min(end);
        open.unit.action = self.slice(open.body_start, open.action_end.unwrap_or(end));
        if let Some(start) = open.verification_start {
            let text = self.slice(start, end);
            open.unit.verification = (!text.is_empty()).then_some(text);
        }
        self.validate_unit(&open);
        if let Some(phase) = self.phases.last_mut() {
            phase.units.push(open.unit);
        }
    }

    fn slice(&self, start: usize, end: usize) -> String {
        self.source[start.min(end)..end].trim().to_owned()
    }

    fn validate_unit(&mut self, open: &OpenUnit) {
        let unit = &open.unit;
        let name = format!("{} {}", label(unit.kind), unit.id);
        let offset_line = unit.location.line;
        let mut push = |message: String| {
            self.diagnostics.push(Diagnostic {
                location: SourceLocation::new(self.document, offset_line),
                message,
            });
        };
        if unit.action.is_empty() {
            push(format!(
                "{name} has no action: describe what to do before `**Expected result:**`"
            ));
        }
        match &open.expected_marker {
            None => push(format!(
                "{name} has no expected result: add `**Expected result:**` followed by a list of observable criteria"
            )),
            Some(_) if unit.expected.is_empty() => push(format!(
                "{name} lists no criteria under `**Expected result:**`"
            )),
            Some(_) => {}
        }
    }
}

impl OpenUnit {
    const fn new(
        kind: UnitKind,
        id: String,
        title: String,
        location: SourceLocation,
        body_start: usize,
    ) -> Self {
        Self {
            unit: Unit {
                kind,
                id,
                title,
                location,
                action: String::new(),
                commands: Vec::new(),
                expected: Vec::new(),
                verification: None,
                procedures: Vec::new(),
            },
            body_start,
            part: UnitPart::Action,
            action_end: None,
            expected_marker: None,
            verification_start: None,
            end: body_start,
        }
    }

    fn marker(
        &mut self,
        marker: Marker,
        block: &Block,
        location: SourceLocation,
    ) -> Result<(), String> {
        match marker {
            Marker::Expected(rest) => {
                if self.expected_marker.is_some() {
                    return Err(
                        "`**Expected result:**` appears more than once in this unit".to_owned()
                    );
                }
                if self.part == UnitPart::Verification {
                    return Err("put `**Expected result:**` before `**Verification:**`".to_owned());
                }
                self.action_end = Some(block.range.start);
                self.expected_marker = Some(location.clone());
                self.part = UnitPart::Expected;
                self.push_criterion(rest, location);
            }
            Marker::Verification => {
                if self.part == UnitPart::Action {
                    self.action_end = Some(block.range.start);
                }
                self.part = UnitPart::Verification;
                self.verification_start = Some(block.range.end);
            }
        }
        Ok(())
    }

    fn push_criterion(&mut self, text: String, location: SourceLocation) {
        if !text.is_empty() {
            let index = self.unit.expected.len();
            self.unit.expected.push(Criterion {
                index,
                text,
                location,
            });
        }
    }

    fn content(&mut self, block: &Block, location: SourceLocation) {
        match (&block.kind, self.part) {
            (BlockKind::Code { language, code }, UnitPart::Action) => {
                self.unit.commands.push(CodeBlock {
                    language: language.clone(),
                    code: code.clone(),
                    location,
                });
            }
            (BlockKind::Paragraph(paragraph), UnitPart::Action) => {
                if let Some(target) = procedure_target(paragraph) {
                    self.unit.procedures.push(ProcedureRef { target, location });
                }
            }
            (BlockKind::List { items }, UnitPart::Expected) => {
                for (_, text) in items {
                    self.push_criterion(text.clone(), location.clone());
                }
            }
            (BlockKind::Paragraph(paragraph), UnitPart::Expected) => {
                self.push_criterion(paragraph.text.clone(), location);
            }
            _ => {}
        }
    }
}

enum Marker {
    /// `**Expected result:**`, with any criterion text on the same paragraph.
    Expected(String),
    /// `**Verification:**`.
    Verification,
}

impl Marker {
    fn of(paragraph: &Paragraph) -> Option<Self> {
        let strong = paragraph.leading_strong.as_deref()?;
        let name = strong.trim_end_matches(':').trim().to_ascii_lowercase();
        let rest = paragraph.text.get(strong.len()..).unwrap_or_default();
        let rest = rest.trim_start_matches(':').trim().to_owned();
        match name.as_str() {
            "expected result" | "expected results" => Some(Self::Expected(rest)),
            "verification" => Some(Self::Verification),
            _ => None,
        }
    }
}

/// The target of a `Follow [path](path).` paragraph.
fn procedure_target(paragraph: &Paragraph) -> Option<String> {
    let invokes = paragraph
        .before_first_link
        .trim()
        .eq_ignore_ascii_case("follow");
    paragraph.first_link.clone().filter(|_| invokes)
}

/// Whether a `Test`/`Step` heading is meant as an execution unit: it has a
/// colon, or its second word starts with a digit (`Test 1.1 ...`). Other
/// headings such as `### Test notes` are explanatory.
fn looks_like_unit(text: &str) -> bool {
    text.contains(':')
        || text
            .split_whitespace()
            .nth(1)
            .is_some_and(|word| word.starts_with(|c: char| c.is_ascii_digit()))
}

fn first_word(text: &str) -> &str {
    text.split_whitespace().next().unwrap_or_default()
}

/// Splits `<label> <id>: <title>` into its ID and title.
fn split_labeled(text: &str, label: &str) -> Option<(String, String)> {
    let rest = text.strip_prefix(label)?;
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let (id, title) = rest.split_once(':')?;
    let (id, title) = (id.trim(), title.trim());
    let valid = !id.is_empty() && !title.is_empty() && !id.contains(char::is_whitespace);
    valid.then(|| (id.to_owned(), title.to_owned()))
}

const fn label(kind: UnitKind) -> &'static str {
    match kind {
        UnitKind::Test => "Test",
        UnitKind::Step => "Step",
    }
}

#[cfg(test)]
mod tests;
