//! A small orders service with one deliberate defect, used as the system
//! under test in the walkthrough (`docs/walkthrough.md`).
//!
//! - `GET /health` answers 200 `{"status":"ok"}`.
//! - `GET /orders/1` answers 200 with a shipped order.
//! - `POST /orders` answers 500 `{"error":"orders database is read-only"}`
//!   instead of creating the order: the defect a plan should find.

use axum::Router;
use axum::http::StatusCode;
use axum::routing::{get, post};
use serde_json::{Value, json};

/// The service's routes.
pub fn router() -> Router {
    Router::new()
        .route(
            "/health",
            get(|| async { axum::Json(json!({"status": "ok"})) }),
        )
        .route("/orders/1", get(|| async { axum::Json(order()) }))
        .route(
            "/orders",
            post(|| async {
                let body = json!({"error": "orders database is read-only"});
                (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(body))
            }),
        )
}

fn order() -> Value {
    json!({"id": 1, "item": "keyboard", "quantity": 2, "status": "shipped"})
}
