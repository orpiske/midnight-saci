//! The Julia 1 ONNX graph in ONNX Runtime.
//!
//! Graph inputs (from `export.py` in Julia-1-ONNX): `input_ids`,
//! `attention_mask` and `marker_pos` are int64, `marker_mask` is bool and
//! `qtype` is int64 with one value per row. The output `logits` has one
//! score per option marker: `[rows, markers]`.

use std::path::Path;

use anyhow::{Context, anyhow, bail};
use ort::session::Session;
use ort::session::builder::GraphOptimizationLevel;
use ort::value::Tensor;
use serde_json::{Map, Value, json};

use crate::encode::{Encoded, Encoder, Kind, Limits};
use crate::wire;

/// The model name in answers.
pub const MODEL_NAME: &str = "julia-1";

/// Token rows are padded to a multiple of this length, as in the reference
/// runtime.
const LENGTH_ALIGN: usize = 8;

/// Why an evaluation failed.
#[derive(Debug, thiserror::Error)]
pub enum EvaluateError {
    /// The request is not valid, or does not fit the token limits.
    #[error("{0}")]
    Invalid(String),
    /// The model failed.
    #[error("{0}")]
    Model(String),
}

/// Flat, padded tensors for one batch of rows.
#[derive(Debug, PartialEq, Eq)]
pub struct Batch {
    pub rows: usize,
    pub length: usize,
    pub count: usize,
    pub ids: Vec<i64>,
    pub attention: Vec<i64>,
    pub positions: Vec<i64>,
    pub mask: Vec<bool>,
    pub qtype: Vec<i64>,
}

impl Batch {
    #[must_use]
    pub fn new(encoded: &[Encoded]) -> Self {
        let rows = encoded.len();
        let longest = encoded.iter().map(|e| e.ids.len()).max().unwrap_or(0);
        let length = longest.div_ceil(LENGTH_ALIGN) * LENGTH_ALIGN;
        let count = encoded.iter().map(|e| e.markers.len()).max().unwrap_or(0);
        let mut batch = Self {
            rows,
            length,
            count,
            ids: vec![0; rows * length],
            attention: vec![0; rows * length],
            positions: vec![0; rows * count],
            mask: vec![false; rows * count],
            qtype: Vec::with_capacity(rows),
        };
        for (row, e) in encoded.iter().enumerate() {
            for (col, &id) in e.ids.iter().enumerate() {
                batch.ids[row * length + col] = i64::from(id);
                batch.attention[row * length + col] = 1;
            }
            for (col, &marker) in e.markers.iter().enumerate() {
                batch.positions[row * count + col] = i64::try_from(marker).unwrap_or(i64::MAX);
                batch.mask[row * count + col] = true;
            }
            batch.qtype.push(i64::from(e.qtype));
        }
        batch
    }
}

/// Settings for loading the model.
#[derive(Debug, Clone, Copy, Default)]
pub struct Options {
    pub limits: Limits,
    /// CPU threads for one evaluation; ONNX Runtime chooses when `None`.
    pub threads: Option<usize>,
}

/// The loaded model and its encoder. ONNX Runtime sessions can run
/// evaluations from several threads at once.
pub struct Engine {
    encoder: Encoder,
    session: Session,
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Engine")
            .field("encoder", &self.encoder)
            .finish_non_exhaustive()
    }
}

impl Engine {
    /// Loads `model.onnx` (with `model.onnx.data` beside it) and
    /// `tokenizer.json` from `model_dir`.
    ///
    /// # Errors
    ///
    /// Fails when a file is missing or ONNX Runtime cannot load the graph.
    pub fn load(model_dir: &Path, options: Options) -> anyhow::Result<Self> {
        let tokenizer_path = model_dir.join("tokenizer.json");
        let tokenizer = std::fs::read_to_string(&tokenizer_path)
            .with_context(|| format!("cannot read {}", tokenizer_path.display()))?;
        let encoder = Encoder::new(&tokenizer, options.limits).map_err(|error| anyhow!(error))?;

        let mut builder =
            Session::builder()?.with_optimization_level(GraphOptimizationLevel::Level3)?;
        if let Some(threads) = options.threads {
            builder = builder.with_intra_threads(threads)?;
        }
        let model_path = model_dir.join("model.onnx");
        let session = builder
            .commit_from_file(&model_path)
            .with_context(|| format!("cannot load {}", model_path.display()))?;
        Ok(Self { encoder, session })
    }

    /// Encodes one question.
    ///
    /// # Errors
    ///
    /// Fails when the question does not fit the encoder rules.
    pub fn encode(
        &self,
        kind: Kind,
        question: &str,
        options: &[String],
        state: &str,
    ) -> Result<Encoded, String> {
        self.encoder.encode(kind, question, options, state)
    }

    /// Runs encoded rows in one batch and returns the raw logits of each row.
    ///
    /// # Errors
    ///
    /// Fails when ONNX Runtime fails or returns an unexpected shape.
    pub fn logits(&self, encoded: &[Encoded]) -> anyhow::Result<Vec<Vec<f32>>> {
        let batch = Batch::new(encoded);
        let (rows, length, count) = (batch.rows, batch.length, batch.count);
        let outputs = self.session.run(ort::inputs! {
            "input_ids" => Tensor::from_array(([rows, length], batch.ids))?,
            "attention_mask" => Tensor::from_array(([rows, length], batch.attention))?,
            "marker_pos" => Tensor::from_array(([rows, count], batch.positions))?,
            "marker_mask" => Tensor::from_array(([rows, count], batch.mask))?,
            "qtype" => Tensor::from_array(([rows], batch.qtype))?,
        }?)?;
        let (shape, data) = outputs["logits"].try_extract_raw_tensor::<f32>()?;
        if data.len() != rows * count {
            bail!("unexpected logits shape {shape:?} for {rows} rows of {count} options");
        }
        Ok(encoded
            .iter()
            .enumerate()
            .map(|(row, e)| data[row * count..row * count + e.markers.len()].to_vec())
            .collect())
    }

    /// Answers an evaluation request in the TypeSafe System One format.
    ///
    /// # Errors
    ///
    /// [`EvaluateError::Invalid`] for a request the model cannot take, and
    /// [`EvaluateError::Model`] when the model fails.
    pub fn evaluate(&self, request: &Value) -> Result<Value, EvaluateError> {
        let request = wire::parse(request).map_err(EvaluateError::Invalid)?;
        let encoded = request
            .questions
            .iter()
            .map(|q| {
                self.encode(q.kind, &q.instructions, &q.options, &request.state)
                    .map_err(|error| {
                        EvaluateError::Invalid(format!("question {:?}: {error}", q.id))
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let logits = self
            .logits(&encoded)
            .map_err(|error| EvaluateError::Model(format!("{error:#}")))?;
        let answers = request
            .questions
            .iter()
            .zip(&logits)
            .map(|(q, logits)| Ok((q.id.clone(), wire::answer(q, logits)?)))
            .collect::<Result<Map<String, Value>, String>>()
            .map_err(EvaluateError::Model)?;
        Ok(json!({"model": MODEL_NAME, "answers": answers}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batches_pad_tokens_to_eight_and_markers_to_the_widest_row() {
        let batch = Batch::new(&[
            Encoded {
                ids: vec![2, 10, 1, 4, 11, 4, 12, 1, 13, 1],
                markers: vec![3, 5],
                qtype: 0,
            },
            Encoded {
                ids: vec![2, 20, 1, 4, 21, 4, 22, 4, 23, 1, 1],
                markers: vec![3, 5, 7],
                qtype: 1,
            },
        ]);
        assert_eq!((batch.rows, batch.length, batch.count), (2, 16, 3));
        assert_eq!(&batch.ids[..10], &[2, 10, 1, 4, 11, 4, 12, 1, 13, 1]);
        assert_eq!(&batch.ids[10..16], &[0; 6]);
        assert_eq!(batch.attention[..16].iter().sum::<i64>(), 10);
        assert_eq!(batch.attention[16..].iter().sum::<i64>(), 11);
        assert_eq!(batch.positions, [3, 5, 0, 3, 5, 7]);
        assert_eq!(batch.mask, [true, true, false, true, true, true]);
        assert_eq!(batch.qtype, [0, 1]);
    }
}
