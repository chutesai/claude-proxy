use crate::models::{App, ClaudeTokenCountRequest};
use crate::utils::content_extraction::count_input_tokens;
use axum::{extract::State, http::StatusCode, response::Result};
use serde_json::{json, Value};

/// Count tokens using tiktoken (cl100k_base encoding baseline)
pub async fn count_tokens(
    State(_app): State<App>,
    axum::Json(req): axum::Json<ClaudeTokenCountRequest>,
) -> Result<axum::Json<Value>, (StatusCode, &'static str)> {
    let token_count = tokio::task::spawn_blocking(move || {
        count_input_tokens(&req.messages, &req.system, &req.tools)
    })
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "tokenization_failed"))?;

    Ok(axum::Json(json!({ "input_tokens": token_count })))
}
