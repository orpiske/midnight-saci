//! A stand-in Julia 1 model and a running `saci-julia` service for tests.
//!
//! The stand-in graph has the Julia 1 interface and returns each option's
//! marker position as its logit, so the expected probabilities follow from
//! the token layout. The stand-in tokenizer makes each word one token.
#![allow(dead_code)] // Each test file uses a part of this module.

use std::path::Path;
use std::sync::Arc;

use saci_julia::encode::Limits;
use saci_julia::engine::{Engine, Options};
use saci_julia::server;
use serde_json::json;
use tokio_util::sync::CancellationToken;

/// Protocol Buffers encoding, enough for a small ONNX model.
mod proto {
    #[allow(clippy::cast_possible_truncation)] // Varints keep 7 bits per byte on purpose.
    pub fn varint(mut value: u64, out: &mut Vec<u8>) {
        while value >= 0x80 {
            out.push((value as u8) | 0x80);
            value >>= 7;
        }
        out.push(value as u8);
    }

    pub fn number(field: u64, value: u64) -> Vec<u8> {
        let mut out = Vec::new();
        varint(field << 3, &mut out);
        varint(value, &mut out);
        out
    }

    pub fn bytes(field: u64, value: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        varint((field << 3) | 2, &mut out);
        varint(value.len() as u64, &mut out);
        out.extend_from_slice(value);
        out
    }

    pub fn text(field: u64, value: &str) -> Vec<u8> {
        bytes(field, value.as_bytes())
    }
}

const FLOAT: u64 = 1;
const INT64: u64 = 7;
const BOOL: u64 = 9;

/// A tensor value description with symbolic dimensions.
fn value_info(name: &str, elem_type: u64, dims: &[&str]) -> Vec<u8> {
    let shape: Vec<u8> = dims
        .iter()
        .flat_map(|dim| proto::bytes(1, &proto::text(2, dim)))
        .collect();
    let tensor = [proto::number(1, elem_type), proto::bytes(2, &shape)].concat();
    [
        proto::text(1, name),
        proto::bytes(2, &proto::bytes(1, &tensor)),
    ]
    .concat()
}

/// `logits = Cast(marker_pos, to = FLOAT)`.
fn cast_node() -> Vec<u8> {
    let to = [
        proto::text(1, "to"),
        proto::number(3, FLOAT),
        proto::number(20, 2), // AttributeProto.INT
    ]
    .concat();
    [
        proto::text(1, "marker_pos"),
        proto::text(2, "logits"),
        proto::text(3, "cast"),
        proto::text(4, "Cast"),
        proto::bytes(5, &to),
    ]
    .concat()
}

/// The Julia 1 graph interface, with `logits = float(marker_pos)`.
fn stand_in_model() -> Vec<u8> {
    let inputs = [
        ("input_ids", INT64, &["rows", "tokens"][..]),
        ("attention_mask", INT64, &["rows", "tokens"]),
        ("marker_pos", INT64, &["rows", "options"]),
        ("marker_mask", BOOL, &["rows", "options"]),
        ("qtype", INT64, &["rows"]),
    ];
    let mut graph = [proto::bytes(1, &cast_node()), proto::text(2, "stand-in")].concat();
    for (name, elem_type, dims) in inputs {
        graph.extend(proto::bytes(11, &value_info(name, elem_type, dims)));
    }
    graph.extend(proto::bytes(
        12,
        &value_info("logits", FLOAT, &["rows", "options"]),
    ));
    [
        proto::number(1, 8), // IR version
        proto::bytes(7, &graph),
        proto::bytes(8, &[proto::text(1, ""), proto::number(2, 18)].concat()),
    ]
    .concat()
}

/// A whitespace tokenizer that knows no words: every word is one token.
fn stand_in_tokenizer() -> String {
    json!({
        "version": "1.0", "truncation": null, "padding": null, "added_tokens": [],
        "normalizer": null, "pre_tokenizer": {"type": "Whitespace"},
        "post_processor": null, "decoder": null,
        "model": {"type": "WordLevel", "vocab": {"[UNK]": 3}, "unk_token": "[UNK]"}
    })
    .to_string()
}

fn model_dir(dir: &Path) {
    std::fs::write(dir.join("model.onnx"), stand_in_model()).unwrap();
    std::fs::write(dir.join("tokenizer.json"), stand_in_tokenizer()).unwrap();
}

pub async fn start(token: Option<&str>) -> (String, tempfile::TempDir, CancellationToken) {
    let dir = tempfile::tempdir().unwrap();
    model_dir(dir.path());
    let options = Options {
        limits: Limits {
            max_length: 64,
            head_length: 32,
            strict: true,
        },
        threads: Some(1),
    };
    let engine = Arc::new(Engine::load(dir.path(), options).unwrap());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let shutdown = CancellationToken::new();
    let stop = shutdown.clone();
    let token = token.map(str::to_owned);
    tokio::spawn(async move { server::serve(engine, token, listener, stop).await.unwrap() });
    (url, dir, shutdown)
}

pub fn sigmoid(x: f64) -> f64 {
    1.0 / (1.0 + (-x).exp())
}
