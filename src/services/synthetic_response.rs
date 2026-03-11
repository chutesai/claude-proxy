use axum::response::sse::Event;
use serde_json::{json, Value};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;

/// Send a complete synthetic Claude SSE response with a single text content block.
/// Used for 404 model-not-found, backend errors, etc.
pub async fn send_synthetic_text_response(
    tx: &mpsc::Sender<Event>,
    model: &str,
    content: &str,
    stop_reason: &str,
    input_tokens: u32,
    output_tokens: u32,
) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();

    let message_obj = json!({
        "id": format!("msg_{}", now),
        "type": "message",
        "role": "assistant",
        "content": json!([]),
        "model": model,
        "stop_reason": Value::Null,
        "stop_sequence": Value::Null,
        "usage": { "input_tokens": input_tokens, "output_tokens": 0 }
    });

    let start = json!({ "type": "message_start", "message": message_obj });
    if tx
        .send(
            Event::default()
                .event("message_start")
                .data(start.to_string()),
        )
        .await
        .is_err()
    {
        return;
    }

    let block_start = json!({
        "type": "content_block_start",
        "index": 0,
        "content_block": { "type": "text", "text": "" }
    });
    if tx
        .send(
            Event::default()
                .event("content_block_start")
                .data(block_start.to_string()),
        )
        .await
        .is_err()
    {
        return;
    }

    let delta = json!({
        "type": "content_block_delta",
        "index": 0,
        "delta": { "type": "text_delta", "text": content }
    });
    if tx
        .send(
            Event::default()
                .event("content_block_delta")
                .data(delta.to_string()),
        )
        .await
        .is_err()
    {
        return;
    }

    let block_stop = json!({ "type": "content_block_stop", "index": 0 });
    if tx
        .send(
            Event::default()
                .event("content_block_stop")
                .data(block_stop.to_string()),
        )
        .await
        .is_err()
    {
        return;
    }

    let msg_delta = json!({
        "type": "message_delta",
        "delta": { "stop_reason": stop_reason, "stop_sequence": Value::Null },
        "usage": { "output_tokens": output_tokens }
    });
    if tx
        .send(
            Event::default()
                .event("message_delta")
                .data(msg_delta.to_string()),
        )
        .await
        .is_err()
    {
        return;
    }

    let msg_stop = json!({ "type": "message_stop" });
    let _ = tx
        .send(
            Event::default()
                .event("message_stop")
                .data(msg_stop.to_string()),
        )
        .await;
}
