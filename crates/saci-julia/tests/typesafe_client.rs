#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The TypeSafe client that the Saci judge uses reads `saci-julia` answers.

mod common;

use std::collections::BTreeMap;

use common::{sigmoid, start};
use rig::http_client::ReqwestClient;
use rig::typesafeai::types::{Answer, Question};
use rig::typesafeai::{DynamicQuery, Evaluate, JevConfig};
use saci_julia::server;
use serde_json::{Value, json};

/// The question kinds the judge uses, and a choice.
fn questions() -> DynamicQuery {
    DynamicQuery::new(BTreeMap::from([
        (
            "criterion_0".to_owned(),
            Question::Noul {
                instructions: Value::String("Does the output show ok?".into()),
                criteria: Some(BTreeMap::from([
                    ("true".to_owned(), json!("It holds.")),
                    ("false".to_owned(), json!("It does not.")),
                ])),
            },
        ),
        (
            "team".to_owned(),
            Question::Choice {
                instructions: Value::String("Who?".into()),
                criteria: BTreeMap::from([
                    ("access".to_owned(), Some(json!("log in"))),
                    ("billing".to_owned(), Some(json!("pay"))),
                ]),
            },
        ),
    ]))
    .unwrap()
}

#[tokio::test]
async fn the_judge_client_reads_saci_julia_answers() {
    let (url, _dir, shutdown) = start(None).await;
    let http = rig::rig_reqwest::reqwest::Client::new();
    // As the worker connects to a judge without a credential.
    let jev = JevConfig::new("none")
        .model("julia-1")
        .with_endpoint(format!("{url}{}", server::EVALUATE_PATH))
        .connect(ReqwestClient::from(http));
    let questions = questions();
    let state = json!({"step": "Check", "evidence": ["ok"]});
    let result = jev.evaluation().evaluate(&state, questions).await.unwrap();
    assert_eq!(result.model, "julia-1");
    match result.answers.get("criterion_0") {
        // "It does not." is 4 tokens (the period is its own) and its marker.
        Some(Answer::Noul { noul }) => assert!((noul - sigmoid(5.0)).abs() < 1e-6, "{noul}"),
        other => panic!("unexpected answer {other:?}"),
    }
    match result.answers.get("team") {
        Some(Answer::Choice { choice, .. }) => assert_eq!(choice, "billing"),
        other => panic!("unexpected answer {other:?}"),
    }
    shutdown.cancel();
}
