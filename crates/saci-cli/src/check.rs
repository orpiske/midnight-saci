//! `saci check`: validates a plan bundle offline without submitting it.

use std::collections::BTreeMap;
use std::path::Path;

use saci_core::bundle::{self, ExpandedPlan};
use saci_core::parser::{
    Diagnostic, DocumentRole, PlanError, code_blocks, parse_document_with_warnings,
};
use saci_core::plan::{PlanBundle, SourceLocation, UnitKind};

use crate::client::{CliError, exit};

/// Validates the plan at `path` and every procedure it invokes.
///
/// Prints a summary on standard output and each warning on standard error.
/// Returns [`exit::OK`], or [`exit::INVALID`] when `strict` and there are
/// warnings. With `original`, the directory holding the documents the plan
/// was converted from, also checks that the bundle's code blocks are exactly
/// the originals' (see [`compare_commands`]).
///
/// # Errors
///
/// [`exit::INVALID`] with every diagnostic when the bundle is not valid, or
/// with every difference when its code blocks differ from the originals.
pub fn plan(path: &Path, strict: bool, original: Option<&Path>) -> Result<u8, CliError> {
    let invalid = |error: PlanError| {
        CliError::new(
            exit::INVALID,
            format!("{} is not a valid plan:\n{error}", path.display()),
        )
    };
    let bundle = bundle::load(path).map_err(invalid)?;
    let expanded = bundle::expand(&bundle).map_err(invalid)?;
    let warnings = warnings(&bundle).map_err(invalid)?;
    for warning in &warnings {
        eprintln!("warning: {warning}");
    }
    println!(
        "{} is valid: {}",
        path.display(),
        summary(&bundle, &expanded, warnings.len())
    );
    if let Some(original) = original {
        compare_commands(&bundle, original)?;
    }
    Ok(if strict && !warnings.is_empty() {
        exit::INVALID
    } else {
        exit::OK
    })
}

/// Code blocks keyed by language and contents, with where each copy is.
type Blocks = BTreeMap<(String, String), Vec<SourceLocation>>;

/// Checks that the bundle's code blocks are exactly those of the documents
/// with the same paths under `original`, compared across the whole bundle
/// so a block may move between documents, and says so.
fn compare_commands(bundle: &PlanBundle, original: &Path) -> Result<(), CliError> {
    let (mut before, mut after) = (Blocks::new(), Blocks::new());
    for (document, source) in &bundle.documents {
        collect(&mut after, document, source);
        if let Some(source) = read_original(original, document)? {
            collect(&mut before, document, &source);
        }
    }
    let mut problems = surplus(&before, &after, "original code block missing or changed");
    problems.extend(surplus(&after, &before, "code block not in the originals"));
    if problems.is_empty() {
        println!(
            "{} in {} match the originals in {}.",
            count(after.values().map(Vec::len).sum(), "code block"),
            count(bundle.documents.len(), "document"),
            original.display()
        );
        return Ok(());
    }
    problems.sort_by(|a, b| (&a.0.document, a.0.line).cmp(&(&b.0.document, b.0.line)));
    let lines: Vec<_> = problems
        .iter()
        .map(|(location, message)| format!("{location}: {message}"))
        .collect();
    let message = format!(
        "the code blocks differ from the originals in {}:\n{}",
        original.display(),
        lines.join("\n")
    );
    Err(CliError::new(exit::INVALID, message))
}

/// The original of `document`, or `None` when it is a new document.
fn read_original(original: &Path, document: &str) -> Result<Option<String>, CliError> {
    let file = original.join(document);
    if !file.exists() {
        return Ok(None);
    }
    std::fs::read_to_string(&file).map(Some).map_err(|error| {
        CliError::new(
            exit::INVALID,
            format!("cannot read {}: {error}", file.display()),
        )
    })
}

fn collect(blocks: &mut Blocks, document: &str, source: &str) {
    for block in code_blocks(document, source) {
        blocks
            .entry((block.language, block.code))
            .or_default()
            .push(block.location);
    }
}

/// Copies of blocks in `from` beyond the number of the same block in `to`.
fn surplus(from: &Blocks, to: &Blocks, what: &str) -> Vec<(SourceLocation, String)> {
    let mut found = Vec::new();
    for (key, locations) in from {
        let kept = to.get(key).map_or(0, Vec::len);
        for location in locations.iter().skip(kept) {
            found.push((location.clone(), format!("{what}: {}", first_line(&key.1))));
        }
    }
    found
}

fn first_line(code: &str) -> String {
    let line = code
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("");
    format!("`{}`", line.trim())
}

/// Warnings of every document in the bundle, entry document first.
fn warnings(bundle: &PlanBundle) -> Result<Vec<Diagnostic>, PlanError> {
    let mut warnings = Vec::new();
    for (document, source) in &bundle.documents {
        let role = if *document == bundle.entry {
            DocumentRole::Entry
        } else {
            DocumentRole::Procedure
        };
        let (_, found) = parse_document_with_warnings(document, source, role)?;
        if role == DocumentRole::Entry {
            warnings.splice(0..0, found);
        } else {
            warnings.extend(found);
        }
    }
    Ok(warnings)
}

fn summary(bundle: &PlanBundle, expanded: &ExpandedPlan, warnings: usize) -> String {
    let units = expanded.units.len();
    let tests = expanded
        .units
        .iter()
        .filter(|unit| unit.identity.kind == UnitKind::Test)
        .count();
    format!(
        "\"{}\", {} ({}, {}), {}, {}.",
        expanded.plan.title,
        count(units, "unit"),
        count(tests, "test"),
        count(units - tests, "step"),
        count(bundle.documents.len() - 1, "procedure"),
        count(warnings, "warning"),
    )
}

fn count(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}
