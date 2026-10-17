//! `saci-fake-model --listen 127.0.0.1:11435 --playbook playbook.json`: an
//! OpenAI-compatible model that answers from a playbook
//! (`saci_testkit::playbook`), for demonstrations without a provider.
//! Requests the playbook does not cover get HTTP 500 with the reason.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use saci_testkit::playbook::Playbook;
use serde_json::Value;

type Error = Box<dyn std::error::Error>;

const USAGE: &str = "usage: saci-fake-model --listen ADDRESS --playbook FILE";

#[tokio::main]
async fn main() -> Result<(), Error> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [listen, address, playbook, file] = args.as_slice() else {
        return Err(USAGE.into());
    };
    if listen != "--listen" || playbook != "--playbook" {
        return Err(USAGE.into());
    }
    let text =
        std::fs::read_to_string(file).map_err(|error| format!("cannot read {file}: {error}"))?;
    let book = Playbook::from_json(&text)?;
    let app = axum::Router::new().route(
        "/v1/chat/completions",
        post(move |Json(request): Json<Value>| {
            let book = book.clone();
            async move { answer(&book, &request) }
        }),
    );
    let listener = tokio::net::TcpListener::bind(address).await?;
    eprintln!(
        "saci-fake-model listening on http://{}/v1",
        listener.local_addr()?
    );
    axum::serve(listener, app).await?;
    Ok(())
}

fn answer(book: &Playbook, request: &Value) -> Response {
    match book.reply(request) {
        Ok(reply) => Json(reply).into_response(),
        Err(reason) => {
            eprintln!("saci-fake-model: {reason}");
            (StatusCode::INTERNAL_SERVER_ERROR, reason).into_response()
        }
    }
}
