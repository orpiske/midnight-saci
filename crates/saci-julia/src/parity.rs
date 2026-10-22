//! Compares the model's logits with the reference PyTorch values in
//! `parity-cases.json` of Julia-1-ONNX.

use std::path::Path;

use anyhow::Context;
use serde::Deserialize;

use crate::encode::Kind;
use crate::engine::Engine;

/// One reference case: a row and the PyTorch logits for its options.
#[derive(Debug, Deserialize)]
pub struct Case {
    pub request: Row,
    pub pytorch_logits: Vec<f32>,
}

#[derive(Debug, Deserialize)]
pub struct Row {
    #[serde(default)]
    pub state: String,
    pub question: String,
    pub options: Vec<String>,
    #[serde(rename = "type", default = "choice")]
    pub kind: String,
}

fn choice() -> String {
    "choice".to_owned()
}

/// The result of a parity check.
#[derive(Debug, Default)]
pub struct Report {
    pub cases: usize,
    /// Case numbers (from 1) that did not match, with the reason.
    pub mismatches: Vec<(usize, String)>,
    pub largest_difference: f32,
}

/// Reads the cases from `path`.
///
/// # Errors
///
/// Fails when the file cannot be read or parsed.
pub fn read_cases(path: &Path) -> anyhow::Result<Vec<Case>> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("cannot parse {}", path.display()))
}

fn argmax(values: &[f32]) -> usize {
    values
        .iter()
        .enumerate()
        .fold(
            (0, f32::NEG_INFINITY),
            |top, (i, &v)| if v > top.1 { (i, v) } else { top },
        )
        .0
}

/// Why `case` does not match the reference, or `None` when it matches,
/// with the largest logit difference.
fn compare(engine: &Engine, case: &Case, tolerance: f32) -> anyhow::Result<(Option<String>, f32)> {
    let row = &case.request;
    let Some(kind) = Kind::parse(&row.kind) else {
        return Ok((Some(format!("unknown type {:?}", row.kind)), 0.0));
    };
    let encoded = match engine.encode(kind, &row.question, &row.options, &row.state) {
        Ok(encoded) => encoded,
        Err(error) => return Ok((Some(error), 0.0)),
    };
    let logits = engine.logits(std::slice::from_ref(&encoded))?.remove(0);
    let difference = if logits.len() == case.pytorch_logits.len() {
        logits
            .iter()
            .zip(&case.pytorch_logits)
            .map(|(a, b)| (a - b).abs())
            .fold(0f32, f32::max)
    } else {
        f32::INFINITY
    };
    let mismatch = (difference > tolerance || argmax(&logits) != argmax(&case.pytorch_logits))
        .then(|| {
            format!(
                "largest difference {difference:.6}, got {logits:?}, expected {:?}",
                case.pytorch_logits
            )
        });
    Ok((mismatch, difference))
}

/// Runs every case. A case matches when each logit is within `tolerance`
/// of the reference and the top option is the same.
///
/// # Errors
///
/// Fails when the model fails.
pub fn check(engine: &Engine, cases: &[Case], tolerance: f32) -> anyhow::Result<Report> {
    let mut report = Report {
        cases: cases.len(),
        ..Report::default()
    };
    for (index, case) in cases.iter().enumerate() {
        let (mismatch, difference) = compare(engine, case, tolerance)?;
        if difference.is_finite() {
            report.largest_difference = report.largest_difference.max(difference);
        }
        if let Some(reason) = mismatch {
            report.mismatches.push((index + 1, reason));
        }
    }
    Ok(report)
}
