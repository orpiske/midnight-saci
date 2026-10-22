#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! `saci-julia` end to end: a stand-in graph and tokenizer behind the real
//! encoder, ONNX Runtime session and HTTP service.

mod common;

use common::{sigmoid, start};
use saci_julia::server;
use serde_json::{Value, json};

/// The request the Saci judge sends (see saci-worker `agent::judge`).
fn judge_request() -> Value {
    json!({
        "model": "julia-1",
        "state": {"step": "Check", "action": "Run it.", "evidence": ["Evidence ev-1: ok"]},
        "questions": {
            "criterion_0": {
                "type": "noul",
                "instructions": "Does the recorded evidence show that this expected result holds: ok",
                "criteria": {"false": "The evidence does not show it.", "true": "It holds."}
            },
            "criterion_1": {"type": "noul", "instructions": "Is it ok?"}
        }
    })
}

#[tokio::test]
async fn the_judge_request_gets_typesafe_shaped_answers() {
    let (url, _dir, shutdown) = start(None).await;
    let client = reqwest::Client::new();
    let response = client
        .post(format!("{url}{}", server::EVALUATE_PATH))
        .bearer_auth("none")
        .json(&judge_request())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["model"], "julia-1");

    // Each option block is the marker and one token per word, so the
    // `true` marker comes 1 + (words of the `false` option) after the
    // `false` marker. The graph's logits are the marker positions.
    let first = &body["answers"]["criterion_0"];
    assert_eq!(first["type"], "noul");
    // "The evidence does not show it." is 7 tokens: the period is its own.
    let expected = sigmoid(1.0 + 7.0);
    assert!(
        (first["noul"].as_f64().unwrap() - expected).abs() < 1e-6,
        "{first}"
    );
    let plain = body["answers"]["criterion_1"]["noul"].as_f64().unwrap();
    assert!((plain - sigmoid(2.0)).abs() < 1e-6, "{plain}");
    shutdown.cancel();
}

#[tokio::test]
async fn choice_and_score_answers_follow_the_option_order() {
    let (url, _dir, shutdown) = start(None).await;
    let request = json!({
        "state": "A ticket.",
        "questions": {
            "team": {"type": "choice", "instructions": "Who?", "criteria": {"billing": "pay", "access": "log in"}},
            "level": {"type": "score", "instructions": "How bad?", "criteria": ["low", "high"]}
        }
    });
    let body: Value = reqwest::Client::new()
        .post(format!("{url}{}", server::EVALUATE_PATH))
        .json(&request)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    // The later option has the larger marker position, so it wins.
    let team = &body["answers"]["team"];
    assert_eq!(team["choice"], "access");
    assert!((team["probabilities"]["access"].as_f64().unwrap() - sigmoid(2.0)).abs() < 1e-6);
    let level = &body["answers"]["level"];
    assert!(
        (level["score"].as_f64().unwrap() - sigmoid(2.0)).abs() < 1e-6,
        "{level}"
    );
    assert_eq!(level["legend"]["1"], "high");
    shutdown.cancel();
}

#[tokio::test]
async fn invalid_and_oversized_requests_are_rejected() {
    let (url, _dir, shutdown) = start(None).await;
    let client = reqwest::Client::new();
    let post = |body: Value| {
        client
            .post(format!("{url}{}", server::EVALUATE_PATH))
            .json(&body)
            .send()
    };
    let bad = post(json!({"state": "s", "questions": {}})).await.unwrap();
    assert_eq!(bad.status(), 400);
    let body: Value = bad.json().await.unwrap();
    assert_eq!(
        body["error"]["message"],
        "questions must be a non-empty object"
    );

    let mut long = judge_request();
    long["state"] = json!(vec!["word"; 100].join(" "));
    let too_long = post(long).await.unwrap();
    assert_eq!(too_long.status(), 400);
    let body: Value = too_long.json().await.unwrap();
    let message = body["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("State exceeds lossless context budget"),
        "{message}"
    );
    shutdown.cancel();
}

#[tokio::test]
async fn a_configured_token_is_required_except_for_health() {
    let (url, _dir, shutdown) = start(Some("s3cret")).await;
    let client = reqwest::Client::new();
    let evaluate = format!("{url}{}", server::EVALUATE_PATH);
    for token in [None, Some("wrong")] {
        let mut request = client.post(&evaluate).json(&judge_request());
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        assert_eq!(request.send().await.unwrap().status(), 401, "{token:?}");
    }
    let allowed = client
        .post(&evaluate)
        .bearer_auth("s3cret")
        .json(&judge_request())
        .send()
        .await
        .unwrap();
    assert_eq!(allowed.status(), 200);
    let health = client
        .get(format!("{url}{}", server::HEALTH_PATH))
        .send()
        .await
        .unwrap();
    assert_eq!(health.status(), 200);
    shutdown.cancel();
}
