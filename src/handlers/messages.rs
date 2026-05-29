use crate::constants::*;
use crate::models::{
    App, ClaudeContentBlock, ClaudeRequest, OAIChatReq, OAIMessage, OAIStreamChunk,
};
use crate::services::{
    build_model_list_content, extract_client_key, format_backend_error, get_available_models,
    mask_token, SseEventParser, ToolBuf, ToolsMap,
};
use crate::utils::content_extraction::{
    build_oai_tools, convert_system_content, convert_tool_choice, count_input_tokens,
    serialize_tool_result_content, translate_finish_reason,
};
use crate::utils::normalize_model_name;
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{
        sse::{Event, Sse},
        IntoResponse, Response,
    },
};
use futures::StreamExt;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    convert::Infallible,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio_stream::wrappers::ReceiverStream;

fn anthropic_error_type(status: StatusCode) -> &'static str {
    match status {
        StatusCode::BAD_REQUEST => "invalid_request_error",
        StatusCode::UNAUTHORIZED => "authentication_error",
        StatusCode::FORBIDDEN => "permission_error",
        StatusCode::NOT_FOUND => "not_found_error",
        StatusCode::PAYLOAD_TOO_LARGE => "request_too_large",
        StatusCode::TOO_MANY_REQUESTS => "rate_limit_error",
        StatusCode::SERVICE_UNAVAILABLE => "overloaded_error",
        _ if status.is_server_error() => "api_error",
        _ => "invalid_request_error",
    }
}

fn build_error_json_response(status: StatusCode, error_type: &str, message: &str) -> Response {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let body = json!({
        "type": "error",
        "error": {
            "type": error_type,
            "message": message,
        },
        "request_id": format!("req_{}", now),
    });

    let mut headers = HeaderMap::new();
    headers.insert("content-type", "application/json".parse().unwrap());
    (status, headers, axum::Json(body)).into_response()
}

fn maybe_error_response(
    client_wants_stream: bool,
    status: StatusCode,
    fallback_code: &'static str,
    error_type: &'static str,
    message: &'static str,
) -> Result<Response, (StatusCode, &'static str)> {
    if client_wants_stream {
        Err((status, fallback_code))
    } else {
        Ok(build_error_json_response(status, error_type, message))
    }
}

/// Build SSE response headers
fn sse_response_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("cache-control", "no-cache".parse().unwrap());
    headers.insert("connection", "keep-alive".parse().unwrap());
    headers.insert("x-accel-buffering", "no".parse().unwrap());
    headers
}

/// Wrap an SSE stream into an axum Response
fn into_sse_response(rx: tokio::sync::mpsc::Receiver<Event>) -> Response {
    let headers = sse_response_headers();
    let stream = ReceiverStream::new(rx).map(Ok::<Event, Infallible>);
    (headers, Sse::new(stream)).into_response()
}

/// Build a non-streaming Claude Messages API JSON response
fn build_non_streaming_response(
    model: &str,
    content_blocks: Vec<Value>,
    stop_reason: &str,
    input_tokens: u32,
    output_tokens: u32,
) -> Response {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let body = json!({
        "id": format!("msg_{}", now),
        "type": "message",
        "role": "assistant",
        "content": content_blocks,
        "model": model,
        "stop_reason": stop_reason,
        "stop_sequence": Value::Null,
        "usage": {
            "input_tokens": input_tokens,
            "output_tokens": output_tokens
        }
    });
    let mut headers = HeaderMap::new();
    headers.insert("content-type", "application/json".parse().unwrap());
    (headers, axum::Json(body)).into_response()
}

fn content_uses_cache_control(content: &Value) -> bool {
    content
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .any(|block| block.get("cache_control").is_some())
        })
        .unwrap_or(false)
}

fn extract_backend_error_message(raw: &str) -> String {
    if let Ok(parsed) = serde_json::from_str::<Value>(raw) {
        if let Some(message) = parsed
            .get("error")
            .and_then(|err| {
                err.get("message")
                    .or_else(|| err.get("type"))
                    .and_then(|value| value.as_str())
            })
            .or_else(|| parsed.get("message").and_then(|value| value.as_str()))
        {
            let message = message.trim();
            if !message.is_empty() {
                return message.to_string();
            }
        }
    }

    let message = raw.trim();
    if message.is_empty() {
        "Unknown backend error".to_string()
    } else {
        message.to_string()
    }
}

fn push_text_block(content_blocks: &mut Vec<Value>, text: &str) {
    if !text.is_empty() {
        content_blocks.push(json!({
            "type": "text",
            "text": text,
        }));
    }
}

fn append_non_stream_text_content(content_blocks: &mut Vec<Value>, content: Option<&Value>) {
    let Some(content) = content else {
        return;
    };

    match content {
        Value::String(text) => push_text_block(content_blocks, text),
        Value::Array(items) => {
            for item in items {
                if let Some(text) = item
                    .get("text")
                    .and_then(|value| value.as_str())
                    .or_else(|| item.as_str())
                {
                    push_text_block(content_blocks, text);
                }
            }
        }
        _ => {}
    }
}

fn compact_preview(text: &str, max_chars: usize) -> String {
    let squashed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if squashed.chars().count() <= max_chars {
        return squashed;
    }

    let mut truncated = squashed.chars().take(max_chars).collect::<String>();
    truncated.push_str("...");
    truncated
}

fn extract_text_preview(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Array(items) => {
            let parts = items
                .iter()
                .take(3)
                .filter_map(extract_text_preview)
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>();

            if parts.is_empty() {
                None
            } else {
                Some(parts.join(" "))
            }
        }
        Value::Object(map) => map
            .get("text")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| map.get("query").and_then(Value::as_str).map(str::to_string))
            .or_else(|| map.get("url").and_then(Value::as_str).map(str::to_string))
            .or_else(|| map.get("content").and_then(extract_text_preview)),
        _ => None,
    }
}

fn unsupported_content_block_text(item: &Value) -> String {
    let block_type = item
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or(match item {
            Value::String(_) => "text",
            Value::Array(_) => "array",
            Value::Object(_) => "object",
            Value::Null => "null",
            Value::Bool(_) => "boolean",
            Value::Number(_) => "number",
        });

    let block_name = item
        .get("name")
        .or_else(|| item.get("tool_name"))
        .and_then(Value::as_str);

    let mut text = format!("[Unsupported Claude content block: {block_type}");
    if let Some(name) = block_name {
        text.push_str(&format!(" ({name})"));
    }
    text.push(']');

    if let Some(preview) = extract_text_preview(item) {
        let preview = compact_preview(&preview, 160);
        if !preview.is_empty() {
            text.push(' ');
            text.push_str(&preview);
        }
    }

    text
}

fn parse_claude_content_blocks(content: &Value, role: &str) -> Vec<ClaudeContentBlock> {
    if let Ok(blocks) = serde_json::from_value::<Vec<ClaudeContentBlock>>(content.clone()) {
        return blocks;
    }

    let Some(items) = content.as_array() else {
        log::warn!(
            "⚠️ Content for role={} was not a string or content-block array; coercing to placeholder text",
            role
        );
        return vec![ClaudeContentBlock::Text {
            text: unsupported_content_block_text(content),
            cache_control: None,
        }];
    };

    items
        .iter()
        .map(|item| {
            if let Some(text) = item.as_str() {
                return ClaudeContentBlock::Text {
                    text: text.to_string(),
                    cache_control: None,
                };
            }

            match serde_json::from_value::<ClaudeContentBlock>(item.clone()) {
                Ok(block) => block,
                Err(err) => {
                    let block_type = item
                        .get("type")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown");
                    log::warn!(
                        "⚠️ Unsupported or malformed Claude content block for role={} (type={}): {}",
                        role,
                        block_type,
                        err
                    );
                    ClaudeContentBlock::Text {
                        text: unsupported_content_block_text(item),
                        cache_control: None,
                    }
                }
            }
        })
        .collect()
}

fn normalize_reasoning_effort(effort: Option<&str>) -> Option<String> {
    let effort = effort?.trim();
    if effort.is_empty() {
        None
    } else {
        Some(effort.to_ascii_lowercase())
    }
}

fn default_document_filename(media_type: &str) -> String {
    match media_type {
        "application/pdf" => "document.pdf".into(),
        "text/plain" => "document.txt".into(),
        "text/markdown" => "document.md".into(),
        "application/json" => "document.json".into(),
        _ => {
            let extension = media_type
                .rsplit('/')
                .next()
                .map(|part| {
                    part.chars()
                        .filter(|ch| ch.is_ascii_alphanumeric())
                        .collect::<String>()
                })
                .filter(|part| !part.is_empty())
                .unwrap_or_else(|| "bin".into());
            format!("document.{extension}")
        }
    }
}

struct StreamState {
    next_block_index: i32,
    thinking_open: bool,
    thinking_index: i32,
    text_open: bool,
    text_index: i32,
    tools: ToolsMap,
    final_stop_reason: &'static str,
    output_token_count: u32,
    backend_reported_usage: bool,
}

enum StreamPayloadOutcome {
    Continue,
    Done,
    FatalError,
    ClientDisconnected,
}

async fn send_sse_json(
    tx: &tokio::sync::mpsc::Sender<Event>,
    event_name: &str,
    payload: Value,
) -> bool {
    tx.send(Event::default().event(event_name).data(payload.to_string()))
        .await
        .is_ok()
}

async fn close_thinking_block_if_open(
    tx: &tokio::sync::mpsc::Sender<Event>,
    state: &mut StreamState,
    context: &str,
) -> bool {
    if !state.thinking_open {
        return true;
    }

    let ev = json!({ "type":"content_block_stop", "index":state.thinking_index });
    if !send_sse_json(tx, "content_block_stop", ev).await {
        log::debug!("🔌 Client disconnected while closing thinking block");
        return false;
    }

    state.thinking_open = false;
    log::info!(
        "🧠 OUTPUT: Closed thinking block {} (index={})",
        context,
        state.thinking_index
    );
    true
}

async fn close_text_block_if_open(
    tx: &tokio::sync::mpsc::Sender<Event>,
    state: &mut StreamState,
) -> bool {
    if !state.text_open {
        return true;
    }

    let ev = json!({ "type":"content_block_stop", "index":state.text_index });
    if !send_sse_json(tx, "content_block_stop", ev).await {
        log::debug!("🔌 Client disconnected while closing text block");
        return false;
    }

    state.text_open = false;
    true
}

fn add_output_tokens(state: &mut StreamState, text: &str) {
    if !state.backend_reported_usage {
        state.output_token_count += std::cmp::max(1, text.len() / CHARS_PER_TOKEN) as u32;
    }
}

async fn stream_reasoning_delta(
    tx: &tokio::sync::mpsc::Sender<Event>,
    state: &mut StreamState,
    reasoning: &str,
) -> bool {
    if reasoning.is_empty() {
        return true;
    }

    if !close_text_block_if_open(tx, state).await {
        return false;
    }

    if !state.thinking_open {
        state.thinking_index = state.next_block_index;
        state.next_block_index += 1;
        let ev = json!({
            "type":"content_block_start",
            "index":state.thinking_index,
            "content_block":{"type":"thinking","thinking":""}
        });
        if !send_sse_json(tx, "content_block_start", ev).await {
            log::debug!("🔌 Client disconnected while opening thinking block");
            return false;
        }
        state.thinking_open = true;
        log::info!(
            "🧠 OUTPUT: Opened thinking block (index={})",
            state.thinking_index
        );
    }

    let ev = json!({
        "type":"content_block_delta",
        "index":state.thinking_index,
        "delta":{"type":"thinking_delta","thinking":reasoning}
    });
    if !send_sse_json(tx, "content_block_delta", ev).await {
        log::debug!("🔌 Client disconnected during thinking delta");
        return false;
    }

    log::debug!(
        "🧠 OUTPUT: Streamed thinking delta ({} chars)",
        reasoning.len()
    );
    add_output_tokens(state, reasoning);
    true
}

async fn stream_text_delta(
    tx: &tokio::sync::mpsc::Sender<Event>,
    state: &mut StreamState,
    text: &str,
) -> bool {
    if text.is_empty() {
        return true;
    }

    if !close_thinking_block_if_open(tx, state, "before text").await {
        return false;
    }

    if !state.text_open {
        state.text_index = state.next_block_index;
        state.next_block_index += 1;
        let ev = json!({
            "type":"content_block_start",
            "index":state.text_index,
            "content_block":{"type":"text","text":""}
        });
        if !send_sse_json(tx, "content_block_start", ev).await {
            log::debug!("🔌 Client disconnected while opening text block");
            return false;
        }
        state.text_open = true;
    }

    let ev = json!({
        "type":"content_block_delta",
        "index":state.text_index,
        "delta":{"type":"text_delta","text":text}
    });
    if !send_sse_json(tx, "content_block_delta", ev).await {
        log::debug!("🔌 Client disconnected during text delta");
        return false;
    }

    add_output_tokens(state, text);
    true
}

async fn stream_tool_call_piece(
    tx: &tokio::sync::mpsc::Sender<Event>,
    state: &mut StreamState,
    idx: usize,
    id: Option<&str>,
    name: Option<&str>,
    arguments: Option<&str>,
) -> bool {
    if !close_thinking_block_if_open(tx, state, "before tool call").await {
        return false;
    }

    if !close_text_block_if_open(tx, state).await {
        return false;
    }

    let placeholder_index = state.next_block_index;
    let tb = state.tools.entry(idx).or_insert_with(|| ToolBuf {
        block_index: placeholder_index,
        id: None,
        name: None,
        pending_args: String::new(),
        has_sent_start: false,
    });

    if let Some(id) = id {
        tb.id = Some(id.to_string());
    }
    if let Some(name) = name {
        tb.name = Some(name.to_string());
    }
    if let Some(arguments) = arguments {
        tb.pending_args.push_str(arguments);
    }

    if !tb.has_sent_start {
        let (Some(tool_id), Some(tool_name)) = (tb.id.as_ref(), tb.name.as_ref()) else {
            return true;
        };

        tb.block_index = state.next_block_index;
        state.next_block_index += 1;

        let start = json!({
            "type":"content_block_start",
            "index":tb.block_index,
            "content_block":{
                "type":"tool_use",
                "id":tool_id,
                "name":tool_name,
                "input":{}
            }
        });
        if !send_sse_json(tx, "content_block_start", start).await {
            log::debug!("🔌 Client disconnected during tool start");
            return false;
        }
        tb.has_sent_start = true;
        log::info!("🔧 Tool call started: id={}, name={}", tool_id, tool_name);
    }

    if tb.has_sent_start && !tb.pending_args.is_empty() {
        let ev = json!({
            "type":"content_block_delta",
            "index": tb.block_index,
            "delta":{"type":"input_json_delta","partial_json": tb.pending_args}
        });
        if !send_sse_json(tx, "content_block_delta", ev).await {
            log::debug!("🔌 Client disconnected during tool args");
            return false;
        }
        tb.pending_args.clear();
    }

    true
}

async fn stream_tool_call_delta(
    tx: &tokio::sync::mpsc::Sender<Event>,
    state: &mut StreamState,
    tool_call: &crate::models::OAIToolCallDelta,
) -> bool {
    let idx = tool_call.index.unwrap_or(0);
    let id = tool_call.id.as_deref();
    let name = tool_call
        .function
        .as_ref()
        .and_then(|function| function.name.as_deref());
    let arguments = tool_call
        .function
        .as_ref()
        .and_then(|function| function.arguments.as_deref());

    stream_tool_call_piece(tx, state, idx, id, name, arguments).await
}

async fn stream_tool_call_value(
    tx: &tokio::sync::mpsc::Sender<Event>,
    state: &mut StreamState,
    default_idx: usize,
    tool_call: &Value,
) -> bool {
    let idx = tool_call
        .get("index")
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .unwrap_or(default_idx);
    let id = tool_call.get("id").and_then(Value::as_str);
    let name = tool_call
        .get("function")
        .and_then(|function| function.get("name"))
        .and_then(Value::as_str);
    let arguments = tool_call
        .get("function")
        .and_then(|function| function.get("arguments"))
        .and_then(Value::as_str);

    stream_tool_call_piece(tx, state, idx, id, name, arguments).await
}

async fn emit_backend_error_text(
    tx: &tokio::sync::mpsc::Sender<Event>,
    state: &mut StreamState,
    error_details: &str,
    raw_data: &str,
) -> bool {
    if !close_thinking_block_if_open(tx, state, "before error").await {
        return false;
    }
    if !close_text_block_if_open(tx, state).await {
        return false;
    }

    let error_index = state.next_block_index;
    state.next_block_index += 1;

    let start = json!({
        "type":"content_block_start",
        "index":error_index,
        "content_block":{"type":"text","text":""}
    });
    if !send_sse_json(tx, "content_block_start", start).await {
        log::debug!("🔌 Client disconnected during error start");
        return false;
    }

    let formatted_error = format_backend_error(error_details, raw_data);
    let delta = json!({
        "type":"content_block_delta",
        "index":error_index,
        "delta":{"type":"text_delta","text":formatted_error}
    });
    if !send_sse_json(tx, "content_block_delta", delta).await {
        log::debug!("🔌 Client disconnected during error delta");
        return false;
    }

    let stop = json!({ "type":"content_block_stop", "index":error_index });
    if !send_sse_json(tx, "content_block_stop", stop).await {
        log::debug!("🔌 Client disconnected during error stop");
        return false;
    }

    state.final_stop_reason = "error";
    true
}

async fn process_choice_message(
    tx: &tokio::sync::mpsc::Sender<Event>,
    state: &mut StreamState,
    message: &Value,
) -> bool {
    if let Some(reasoning) = message.get("reasoning_content").and_then(Value::as_str) {
        if !stream_reasoning_delta(tx, state, reasoning).await {
            return false;
        }
    }

    let mut content_blocks = Vec::new();
    append_non_stream_text_content(&mut content_blocks, message.get("content"));
    for block in content_blocks {
        if let Some(text) = block.get("text").and_then(Value::as_str) {
            if !stream_text_delta(tx, state, text).await {
                return false;
            }
        }
    }

    if let Some(tool_calls) = message.get("tool_calls").and_then(Value::as_array) {
        for (default_idx, tool_call) in tool_calls.iter().enumerate() {
            if !stream_tool_call_value(tx, state, default_idx, tool_call).await {
                return false;
            }
        }
    }

    true
}

async fn process_backend_sse_payload(
    tx: &tokio::sync::mpsc::Sender<Event>,
    state: &mut StreamState,
    data: &str,
) -> StreamPayloadOutcome {
    if data == "[DONE]" {
        log::debug!("🏁 Received [DONE] marker from backend");
        return StreamPayloadOutcome::Done;
    }
    if data.is_empty() {
        return StreamPayloadOutcome::Continue;
    }

    let parsed: serde_json::Result<OAIStreamChunk> = serde_json::from_str(data);
    let chunk = match parsed {
        Ok(chunk) => chunk,
        Err(err) => {
            let json_value: serde_json::Result<Value> = serde_json::from_str(data);

            if let Ok(value) = json_value {
                if let Some(error_obj) = value.get("error") {
                    let error_msg = error_obj
                        .get("message")
                        .or_else(|| error_obj.get("type"))
                        .and_then(Value::as_str)
                        .unwrap_or("Unknown error");
                    let error_details = if error_msg.is_empty() {
                        serde_json::to_string(error_obj)
                            .unwrap_or_else(|_| "Unknown backend error".into())
                    } else {
                        error_msg.to_string()
                    };

                    log::warn!("⚠️  Backend returned error in chunk: {}", error_details);
                    if emit_backend_error_text(tx, state, &error_details, data).await {
                        return StreamPayloadOutcome::FatalError;
                    }
                    return StreamPayloadOutcome::ClientDisconnected;
                }

                if value.is_object() {
                    let preview = if data.len() > 500 {
                        format!("{}...", &data[..500])
                    } else {
                        data.to_string()
                    };
                    log::warn!(
                        "⚠️  Chunk missing 'choices' field ({} chars), structure: {}",
                        data.len(),
                        preview
                    );
                    return StreamPayloadOutcome::Continue;
                }
            }

            let preview = if data.len() > 500 {
                format!("{}...", &data[..500])
            } else {
                data.to_string()
            };
            log::warn!(
                "⚠️  JSON parse failed ({} chars): {}\nResponse preview: {}",
                data.len(),
                err,
                preview
            );
            return StreamPayloadOutcome::Continue;
        }
    };

    if let Some(error_val) = &chunk.error {
        let error_msg = error_val
            .get("message")
            .or_else(|| error_val.get("type"))
            .and_then(Value::as_str)
            .unwrap_or("Unknown error");
        let error_details = if error_msg.is_empty() {
            serde_json::to_string(error_val).unwrap_or_else(|_| "Unknown backend error".into())
        } else {
            error_msg.to_string()
        };

        log::warn!("⚠️  Backend returned error: {}", error_details);
        if emit_backend_error_text(tx, state, &error_details, data).await {
            return StreamPayloadOutcome::FatalError;
        }
        return StreamPayloadOutcome::ClientDisconnected;
    }

    if chunk.choices.is_empty() {
        log::debug!("⚠️  Chunk has no choices, skipping");
        return StreamPayloadOutcome::Continue;
    }

    let choice = &chunk.choices[0];

    if let Some(reason) = &choice.finish_reason {
        state.final_stop_reason = translate_finish_reason(Some(reason));
        log::debug!(
            "📍 Backend finish_reason: {} → Claude stop_reason: {}",
            reason,
            state.final_stop_reason
        );
    }

    if let Some(usage) = &chunk.usage {
        if let Some(prompt_tokens) = usage.prompt_tokens {
            log::debug!("📊 Backend reported prompt tokens: {}", prompt_tokens);
        }
        if let Some(completion_tokens) = usage.completion_tokens {
            state.output_token_count = completion_tokens;
            state.backend_reported_usage = true;
            log::debug!(
                "📊 Backend reported completion tokens: {}",
                completion_tokens
            );
        }
    }

    if let Some(message) = &choice.message {
        log::debug!("📦 Received non-streaming complete response, converting to SSE");
        if process_choice_message(tx, state, message).await {
            return StreamPayloadOutcome::Continue;
        }
        return StreamPayloadOutcome::ClientDisconnected;
    }

    let Some(delta) = &choice.delta else {
        log::debug!("⚠️  Chunk has no delta or message, skipping");
        return StreamPayloadOutcome::Continue;
    };

    if let Some(reasoning) = &delta.reasoning_content {
        if !stream_reasoning_delta(tx, state, reasoning).await {
            return StreamPayloadOutcome::ClientDisconnected;
        }
    }

    if let Some(content) = &delta.content {
        if !stream_text_delta(tx, state, content).await {
            return StreamPayloadOutcome::ClientDisconnected;
        }
    }

    if let Some(tool_calls) = &delta.tool_calls {
        for tool_call in tool_calls {
            if !stream_tool_call_delta(tx, state, tool_call).await {
                return StreamPayloadOutcome::ClientDisconnected;
            }
        }
    }

    StreamPayloadOutcome::Continue
}

pub async fn messages(
    State(app): State<App>,
    headers: HeaderMap,
    axum::Json(cr): axum::Json<ClaudeRequest>,
) -> Result<Response, (StatusCode, &'static str)> {
    let request_start = SystemTime::now();

    // Count input tokens
    let input_token_count = count_input_tokens(&cr.messages, &cr.system, &cr.tools);
    log::debug!("📊 Input tokens: {}", input_token_count);

    // Determine streaming mode — default to true (Claude Code always streams)
    let client_wants_stream = cr.stream.unwrap_or(true);
    if !client_wants_stream {
        log::info!("📦 Non-streaming mode requested");
    }

    // Circuit breaker check
    {
        let mut cb = app.circuit_breaker.write().await;
        if !cb.should_allow_request() {
            log::error!("🔴 Circuit breaker is open - rejecting request");
            return maybe_error_response(
                client_wants_stream,
                StatusCode::SERVICE_UNAVAILABLE,
                "backend_unavailable_circuit_open",
                "overloaded_error",
                "Backend temporarily unavailable. Please retry shortly.",
            );
        }
    }

    // Request validation
    if cr.messages.is_empty() {
        log::warn!("❌ Validation failed: empty messages");
        return maybe_error_response(
            client_wants_stream,
            StatusCode::BAD_REQUEST,
            "empty_messages",
            "invalid_request_error",
            "messages must not be empty",
        );
    }

    if cr.messages.len() > MAX_MESSAGES_PER_REQUEST {
        log::warn!(
            "❌ Validation failed: too many messages ({})",
            cr.messages.len()
        );
        return maybe_error_response(
            client_wants_stream,
            StatusCode::BAD_REQUEST,
            "too_many_messages",
            "invalid_request_error",
            "request exceeds the maximum number of messages",
        );
    }

    // Validate message size (rough check)
    let total_content_size: usize = cr
        .messages
        .iter()
        .map(|m| {
            if let Some(s) = m.content.as_str() {
                s.len()
            } else {
                serde_json::to_string(&m.content).unwrap_or_default().len()
            }
        })
        .sum();

    if total_content_size > MAX_TOTAL_CONTENT_SIZE {
        log::warn!(
            "❌ Validation failed: content too large ({} bytes)",
            total_content_size
        );
        return maybe_error_response(
            client_wants_stream,
            StatusCode::PAYLOAD_TOO_LARGE,
            "content_too_large",
            "request_too_large",
            "request content exceeds the maximum allowed size",
        );
    }

    // Validate max_tokens if provided
    if let Some(max_tokens) = cr.max_tokens {
        if !(MIN_TOKENS_LIMIT..=MAX_TOKENS_LIMIT).contains(&max_tokens) {
            log::warn!(
                "❌ Validation failed: max_tokens out of range ({})",
                max_tokens
            );
            return maybe_error_response(
                client_wants_stream,
                StatusCode::BAD_REQUEST,
                "invalid_max_tokens",
                "invalid_request_error",
                "max_tokens is outside the supported range",
            );
        }
    }

    // Validate system prompt length if provided
    if let Some(ref system) = cr.system {
        let system_size = match system {
            serde_json::Value::String(s) => s.len(),
            other => serde_json::to_string(other).unwrap_or_default().len(),
        };
        if system_size > MAX_SYSTEM_PROMPT_SIZE {
            log::warn!(
                "❌ Validation failed: system prompt too large ({} bytes)",
                system_size
            );
            return maybe_error_response(
                client_wants_stream,
                StatusCode::BAD_REQUEST,
                "system_prompt_too_large",
                "invalid_request_error",
                "system prompt exceeds the maximum allowed size",
            );
        }
    }

    // Log warning for service_tier (accepted for compatibility but not forwarded)
    if cr.service_tier.is_some() {
        log::debug!("ℹ️  'service_tier' accepted for compatibility but not forwarded to backend");
    }

    // Debug: Log incoming headers (names only)
    log::debug!("📥 Incoming headers:");
    for (name, _) in headers.iter() {
        log::debug!("   {}", name);
    }

    // Auth extraction: Authorization or x-api-key
    let client_key = extract_client_key(&headers);

    if let Some(key) = &client_key {
        log::info!("🔑 Client API Key: Bearer {}", mask_token(key));
    } else {
        log::info!("🔑 No client API key (no 'authorization' or 'x-api-key' header)");
    }

    let has_client_auth = client_key.is_some();
    log::info!(
        "📨 Request: model={}, client_auth={}, backend={}",
        cr.model,
        has_client_auth,
        app.backend_url
    );

    // Normalize model name (case-correction only)
    let backend_model = normalize_model_name(&cr.model, &app.models_cache).await;
    let backend_model_for_metrics = backend_model.clone();

    // Auto-enable thinking for reasoning models if not explicitly provided
    let thinking_config = if cr.thinking.is_some() {
        cr.thinking
    } else {
        // Check if this is a reasoning model by querying model cache
        let is_reasoning_model = {
            let cache = app.models_cache.read().await;
            cache
                .as_ref()
                .and_then(|models| {
                    // Look for model in cache
                    models
                        .iter()
                        .find(|m| m.id.eq_ignore_ascii_case(&backend_model))
                        .map(|model_info| {
                            // Check if model supports thinking features
                            model_info.supported_features.iter().any(|f| {
                                f.eq_ignore_ascii_case("thinking")
                                    || f.eq_ignore_ascii_case("extended_thinking")
                            })
                        })
                })
                .unwrap_or(false) // Default to false if model not found
        };

        if is_reasoning_model {
            log::info!(
                "🧠 Auto-enabling thinking for reasoning model: {}",
                backend_model
            );
            Some(crate::models::ThinkingConfig::Enabled {
                budget_tokens: DEFAULT_THINKING_BUDGET_TOKENS,
            })
        } else {
            None
        }
    };

    let mut msgs = Vec::with_capacity(cr.messages.len() + 1);
    let mut warned_cache_control = false;

    if let Some(system) = cr.system.as_ref() {
        if content_uses_cache_control(system) {
            log::warn!("⚠️ Request uses cache_control (Anthropic prompt caching) — this is silently dropped in translation to OpenAI format. Prompt caching will not be effective.");
            warned_cache_control = true;
        }
    }

    if let Some(sys) = cr.system {
        let system_content = convert_system_content(&sys);
        msgs.push(OAIMessage {
            role: "system".into(),
            content: system_content,
            tool_call_id: None,
            tool_calls: None,
        });
    }

    let original_message_count = cr.messages.len();

    // Convert Claude messages → OpenAI messages
    for m in cr.messages {
        if m.content.is_string() {
            // Simple string passthrough
            log::debug!("📝 Simple string message (role={})", m.role);
            msgs.push(OAIMessage {
                role: m.role,
                content: m.content,
                tool_call_id: None,
                tool_calls: None,
            });
            continue;
        }

        // Detect cache_control usage (Anthropic prompt caching) — warn once
        if !warned_cache_control && content_uses_cache_control(&m.content) {
            log::warn!("⚠️ Request uses cache_control (Anthropic prompt caching) — this is silently dropped in translation to OpenAI format. Prompt caching will not be effective.");
            warned_cache_control = true;
        }

        // Parse content blocks
        log::debug!("🔍 Parsing content blocks (role={})", m.role);
        let blocks = parse_claude_content_blocks(&m.content, &m.role);

        // tool_result blocks require separate "tool" messages
        let has_tool_results = blocks
            .iter()
            .any(|b| matches!(b, ClaudeContentBlock::ToolResult { .. }));

        if has_tool_results && m.role == "user" {
            // Split tool_result → OpenAI tool messages
            for block in &blocks {
                if let ClaudeContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    ..
                } = block
                {
                    let tool_content = serialize_tool_result_content(content);
                    msgs.push(OAIMessage {
                        role: "tool".into(),
                        content: json!(tool_content),
                        tool_call_id: Some(tool_use_id.clone()),
                        tool_calls: None,
                    });
                }
            }

            // Also pass any user text (if present) after tool results
            let text_parts: Vec<&str> = blocks
                .iter()
                .filter_map(|b| match b {
                    ClaudeContentBlock::Text { text, .. } => Some(text.as_str()),
                    _ => None,
                })
                .collect();

            if !text_parts.is_empty() {
                msgs.push(OAIMessage {
                    role: m.role,
                    content: json!(text_parts.join("\n")),
                    tool_call_id: None,
                    tool_calls: None,
                });
            }
        } else if m.role == "assistant" {
            // Assistant messages may include tool_use blocks → OpenAI tool_calls
            let mut thinking_parts = Vec::new();
            let mut has_redacted_thinking = false;
            let mut text_parts = Vec::new();
            let mut tool_calls = Vec::new();

            for block in &blocks {
                match block {
                    ClaudeContentBlock::Thinking {
                        thinking,
                        signature,
                        ..
                    } => {
                        thinking_parts.push(thinking.as_str());
                        if signature.is_some() {
                            log::debug!("🧠 INPUT: Thinking block has signature (cannot be preserved through OAI backend)");
                        }
                        log::info!(
                            "🧠 INPUT: Extracted thinking block ({} chars) from assistant message",
                            thinking.len()
                        );
                    }
                    ClaudeContentBlock::RedactedThinking { .. } => {
                        has_redacted_thinking = true;
                        log::warn!("🧠 INPUT: Redacted thinking block — preserving as marker for multi-turn continuity");
                    }
                    ClaudeContentBlock::Text { text, .. } => text_parts.push(text.as_str()),
                    ClaudeContentBlock::ToolUse {
                        id, name, input, ..
                    } => {
                        tool_calls.push(json!({
                            "id": id,
                            "type": "function",
                            "function": {
                                "name": name,
                                "arguments": serde_json::to_string(input).unwrap_or_else(|_| "{}".into())
                            }
                        }));
                    }
                    _ => {}
                }
            }

            // Interleave thinking: prepend thinking blocks as <think> tags
            // This is the standard format for reasoning-capable OpenAI-compatible backends
            // (DeepSeek-R1, QwQ, etc. on vLLM/SGLang).
            //
            // NOTE: Anthropic's extended thinking requires signatures for multi-turn continuation.
            // Those signatures cannot survive an OAI backend roundtrip, so multi-turn reasoning
            // continuity is best-effort only. Redacted thinking blocks are preserved as markers
            // so the backend at least sees that reasoning occurred.
            let mut combined = String::new();

            if !thinking_parts.is_empty() || has_redacted_thinking {
                combined.push_str("<think>");
                if !thinking_parts.is_empty() {
                    combined.push_str(&thinking_parts.join("\n"));
                }
                if has_redacted_thinking {
                    // Preserve a marker so the backend knows redacted reasoning existed
                    combined.push_str("\n[redacted reasoning]");
                    log::info!("🧠 INPUT: Included redacted thinking marker in <think> block");
                }
                combined.push_str("</think>\n");
                log::info!(
                    "🧠 INPUT: Converted {} thinking block(s) + {} redacted to <think> format",
                    thinking_parts.len(),
                    if has_redacted_thinking { 1 } else { 0 }
                );
            }

            // Add regular text content
            if !text_parts.is_empty() {
                combined.push_str(&text_parts.join("\n"));
            }

            // Use empty string instead of null for tool-only messages (better compatibility)
            let content = json!(combined);

            msgs.push(OAIMessage {
                role: m.role,
                content,
                tool_call_id: None,
                tool_calls: if tool_calls.is_empty() {
                    None
                } else {
                    Some(tool_calls)
                },
            });
        } else {
            // User messages with possible images/documents
            let mut has_multimodal = false;
            let mut oai_content_blocks = Vec::new();

            for block in &blocks {
                match block {
                    ClaudeContentBlock::Text { text, .. } => {
                        oai_content_blocks.push(json!({ "type": "text", "text": text }));
                    }
                    ClaudeContentBlock::Image { source, .. } => {
                        has_multimodal = true;
                        log::info!(
                            "🖼️ Processing image: media_type={}, size={} bytes",
                            source.media_type,
                            source.data.len()
                        );
                        if source.data.starts_with("data:") {
                            log::warn!(
                                "⚠️ Image data already appears to be a data URI (double-encoding?)"
                            );
                        }
                        let data_uri = format!("data:{};base64,{}", source.media_type, source.data);
                        oai_content_blocks.push(json!({
                            "type": "image_url",
                            "image_url": { "url": data_uri }
                        }));
                    }
                    ClaudeContentBlock::Document { source, .. } => {
                        let media_type = source.media_type.as_deref().unwrap_or("unknown");
                        let data_len = source.data.as_deref().map(str::len).unwrap_or(0);
                        match source.source_type.as_str() {
                            "base64" => {
                                match (source.media_type.as_deref(), source.data.as_deref()) {
                                    (Some(media_type), Some(data)) => {
                                        has_multimodal = true;
                                        let data_uri =
                                            format!("data:{};base64,{}", media_type, data);
                                        oai_content_blocks.push(json!({
                                            "type": "file",
                                            "file": {
                                                "filename": default_document_filename(media_type),
                                                "file_data": data_uri
                                            }
                                        }));
                                        log::info!(
                                        "📄 Forwarded inline document as OpenAI file input (media_type={}, size={} bytes)",
                                        media_type,
                                        data_len
                                    );
                                    }
                                    _ => {
                                        log::error!(
                                        "❌ Base64 document is missing media_type or data — degrading to text placeholder."
                                    );
                                        oai_content_blocks.push(json!({
                                        "type": "text",
                                        "text": "[Unsupported document: missing media_type or data]"
                                    }));
                                    }
                                }
                            }
                            "file" => {
                                if let Some(file_id) = source.file_id.as_deref() {
                                    has_multimodal = true;
                                    oai_content_blocks.push(json!({
                                        "type": "file",
                                        "file": {
                                            "file_id": file_id
                                        }
                                    }));
                                    log::info!(
                                        "📄 Forwarded document file reference as OpenAI file input (file_id={})",
                                        file_id
                                    );
                                } else {
                                    log::error!(
                                        "❌ File document source is missing file_id — degrading to text placeholder."
                                    );
                                    oai_content_blocks.push(json!({
                                        "type": "text",
                                        "text": "[Unsupported document: missing file_id]"
                                    }));
                                }
                            }
                            "url" => {
                                let url = source.url.as_deref().unwrap_or("unknown");
                                log::warn!(
                                    "⚠️ Document URL inputs are not supported by OpenAI Chat Completions file blocks; degrading to text reference: {}",
                                    url
                                );
                                oai_content_blocks.push(json!({
                                    "type": "text",
                                    "text": format!("Attached document URL ({}): {}", media_type, url)
                                }));
                            }
                            other => {
                                let location_hint = source
                                    .url
                                    .as_deref()
                                    .or(source.file_id.as_deref())
                                    .unwrap_or("unknown");
                                log::error!(
                                    "❌ Unsupported document source type '{}' (source={}) — degrading to text placeholder.",
                                    other,
                                    location_hint,
                                );
                                oai_content_blocks.push(json!({
                                    "type": "text",
                                    "text": format!(
                                        "[Unsupported document: source_type={}, media_type={}]",
                                        other, media_type
                                    )
                                }));
                            }
                        }
                    }
                    _ => {}
                }
            }

            let content = if has_multimodal {
                json!(oai_content_blocks)
            } else {
                let text = oai_content_blocks
                    .iter()
                    .filter_map(|v| v.get("text").and_then(|t| t.as_str()))
                    .collect::<Vec<_>>()
                    .join("\n");
                json!(text)
            };

            msgs.push(OAIMessage {
                role: m.role,
                content,
                tool_call_id: None,
                tool_calls: None,
            });
        }
    }

    log::debug!(
        "📊 Converted {} Claude messages into {} OpenAI messages",
        original_message_count,
        msgs.len()
    );

    // Claude Code sometimes adds an *empty* assistant placeholder; only remove if truly empty.
    if let Some(last_msg) = msgs.last() {
        let last_is_empty_assistant = last_msg.role == "assistant"
            && (last_msg.content.is_null()
                || (last_msg.content.is_string()
                    && last_msg.content.as_str().unwrap_or("").is_empty()))
            && last_msg
                .tool_calls
                .as_ref()
                .map(|v| v.is_empty())
                .unwrap_or(true);

        if last_is_empty_assistant {
            log::info!("🚮 Removing empty assistant placeholder message from client history.");
            let _ = msgs.pop();
            log::debug!("📊 After filtering: {} messages remaining", msgs.len());
        }
    }

    if msgs.is_empty() {
        log::error!("❌ No messages remaining after conversion!");
        return maybe_error_response(
            client_wants_stream,
            StatusCode::BAD_REQUEST,
            "no_messages",
            "invalid_request_error",
            "no usable messages remained after request normalization",
        );
    }

    let tools = build_oai_tools(cr.tools);
    let (tool_choice, parallel_tool_calls) = convert_tool_choice(cr.tool_choice);
    let reasoning_effort = normalize_reasoning_effort(
        cr.output_config
            .as_ref()
            .and_then(|cfg| cfg.effort.as_deref()),
    );

    let backend_model_for_error = backend_model.clone();

    // Limit stop sequences to 4 to avoid backend errors (OpenAI limit)
    let stop = cr.stop_sequences.map(|mut s| {
        if s.len() > 4 {
            log::warn!("⚠️  Truncating stop_sequences from {} to 4 items", s.len());
            s.truncate(4);
        }
        s
    });

    // Mirror the client's stream preference to the backend.
    let oai = OAIChatReq {
        model: backend_model,
        messages: msgs,
        // Do not hard-default; allow backend default if None (safer across models)
        max_tokens: cr.max_tokens,
        temperature: cr.temperature,
        top_p: cr.top_p,
        top_k: cr.top_k,
        stop,
        tools,
        tool_choice,
        thinking: thinking_config
            .and_then(|tc| tc.into_backend_config())
            .map(|tc| serde_json::to_value(tc).unwrap_or(Value::Null)),
        reasoning_effort,
        parallel_tool_calls,
        metadata: cr.metadata,
        stream: client_wants_stream,
    };

    let mut req = app
        .client
        .post(&app.backend_url)
        .header("content-type", "application/json");

    // Forward client headers, skipping hop-by-hop and headers we set ourselves
    for (name, value) in headers.iter() {
        match name.as_str() {
            "host" | "connection" | "keep-alive" | "transfer-encoding" | "upgrade"
            | "proxy-authenticate" | "proxy-authorization" | "te" | "trailers"
            | "content-type" | "content-length" => {}
            _ => {
                req = req.header(name, value);
            }
        }
    }

    if let Some(host) = &app.backend_host_header {
        req = req.header("host", host);
    }

    // Validate auth: reject Anthropic OAuth tokens, require some key
    if let Some(key) = &client_key {
        if key.contains("sk-ant-") {
            log::warn!("❌ Anthropic OAuth tokens (sk-ant-*) are not supported - use backend-compatible key (cpk_*)");
            return maybe_error_response(
                client_wants_stream,
                StatusCode::UNAUTHORIZED,
                "invalid_auth_token",
                "authentication_error",
                "Anthropic OAuth tokens are not supported; use a backend-compatible API key",
            );
        }
        log::info!("🔄 Auth: Forwarding client key to backend");
    } else {
        log::warn!("❌ No client API key provided");
        return maybe_error_response(
            client_wants_stream,
            StatusCode::UNAUTHORIZED,
            "missing_api_key",
            "authentication_error",
            "missing API key",
        );
    }

    // Debug request body (large inline media/file data truncated)
    if log::log_enabled!(log::Level::Debug) {
        if let Ok(mut json_body) = serde_json::to_string_pretty(&oai) {
            for needle in ["\"url\": \"data:", "\"file_data\": \"data:"] {
                if let Some(start) = json_body.find(needle) {
                    let after = &json_body[start + needle.len()..];
                    if let Some(end_quote) = after.find('"') {
                        if end_quote > 120 {
                            let replace = format!("{}{}...TRUNCATED...\"", needle, &after[..120]);
                            let end_abs = start + needle.len() + end_quote + 1;
                            json_body.replace_range(start..end_abs, &replace);
                        }
                    }
                }
            }
            if json_body.contains("\"image_url\"") || json_body.contains("\"file_data\"") {
                log::info!("📸 Request contains inline media or file data (truncated in logs)");
            }
            let auth_header_str = client_key
                .as_ref()
                .map(|k| format!("Bearer {}", mask_token(k)))
                .unwrap_or_else(|| "Not Set".into());
            log::debug!(
                "\n------------------ Request to Backend ------------------\n\
                 POST {}\n\
                 Authorization: {}\n\
                 Content-Type: application/json\n\n\
                 {}\n\
                 ------------------------------------------------------------",
                app.backend_url,
                auth_header_str,
                json_body
            );
        }
    }

    log::debug!(
        "🚀 Sending request to backend with {} messages",
        oai.messages.len()
    );
    let res = match req.json(&oai).send().await {
        Ok(res) => res,
        Err(e) => {
            log::error!("❌ Backend connection failed: {}", e);
            tokio::spawn({
                let cb = app.circuit_breaker.clone();
                async move {
                    cb.write().await.record_failure();
                }
            });

            if client_wants_stream {
                return Err((StatusCode::BAD_GATEWAY, "backend_unavailable"));
            }

            return Ok(build_error_json_response(
                StatusCode::BAD_GATEWAY,
                "api_error",
                "Failed to connect to the configured backend",
            ));
        }
    };

    let status = res.status();
    log::debug!("📥 Backend response status: {}", status);

    // Validate Content-Type for better error messages
    let content_type = res
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    log::debug!("📥 Backend Content-Type: {}", content_type);

    // Warn if unexpected content type (but don't fail - be permissive)
    if !content_type.is_empty()
        && !content_type.contains("text/event-stream")
        && !content_type.contains("application/json")
        && !content_type.contains("application/octet-stream")
    {
        log::warn!(
            "⚠️  Unexpected Content-Type: {} (expected text/event-stream or application/json)",
            content_type
        );
    }

    if !status.is_success() {
        // Record circuit breaker failure
        tokio::spawn({
            let cb = app.circuit_breaker.clone();
            async move {
                cb.write().await.record_failure();
            }
        });

        // Read error response body
        let error_body = res
            .text()
            .await
            .unwrap_or_else(|_| "Unknown error".to_string());

        log::error!(
            "❌ Backend returned error: {} {} - {}",
            status.as_u16(),
            status.canonical_reason().unwrap_or(""),
            error_body
        );

        // If 404, return synthetic Claude-like SSE with model list
        if status == StatusCode::NOT_FOUND {
            let models = get_available_models(&app).await;
            if !models.is_empty() {
                log::info!(
                    "💡 Model '{}' not found - sending model list to user",
                    backend_model_for_error
                );
                let content = build_model_list_content(&backend_model_for_error, &models);

                if client_wants_stream {
                    let (tx, rx) = tokio::sync::mpsc::channel::<Event>(SSE_CHANNEL_BUFFER_SIZE);
                    let model_name = backend_model_for_error.clone();
                    tokio::spawn(async move {
                        crate::services::send_synthetic_text_response(
                            &tx,
                            &model_name,
                            &content,
                            "end_turn",
                            input_token_count,
                            50,
                        )
                        .await;
                    });
                    return Ok(into_sse_response(rx));
                } else {
                    return Ok(build_non_streaming_response(
                        &backend_model_for_error,
                        vec![json!({"type": "text", "text": content})],
                        "end_turn",
                        input_token_count,
                        50,
                    ));
                }
            }
        }

        // For retryable errors (rate limits, server errors), pass through HTTP status
        // so Claude Code can retry automatically
        if matches!(
            status,
            StatusCode::TOO_MANY_REQUESTS |  // 429
            StatusCode::INTERNAL_SERVER_ERROR |  // 500
            StatusCode::BAD_GATEWAY |  // 502
            StatusCode::SERVICE_UNAVAILABLE |  // 503
            StatusCode::GATEWAY_TIMEOUT // 504
        ) {
            log::info!(
                "⚠️  Returning retryable error status {} for automatic retry",
                status
            );
            if client_wants_stream {
                return Err((status, "backend_error_retryable"));
            }

            let message = extract_backend_error_message(&error_body);
            return Ok(build_error_json_response(
                status,
                anthropic_error_type(status),
                &message,
            ));
        }

        let model_name = backend_model_for_error.clone();

        if client_wants_stream {
            let (tx, rx) = tokio::sync::mpsc::channel::<Event>(64);
            let error_msg = format_backend_error(&error_body, &error_body);
            tokio::spawn(async move {
                crate::services::send_synthetic_text_response(
                    &tx,
                    &model_name,
                    &error_msg,
                    "error",
                    input_token_count,
                    0,
                )
                .await;
            });
            return Ok(into_sse_response(rx));
        } else {
            let message = extract_backend_error_message(&error_body);
            return Ok(build_error_json_response(
                status,
                anthropic_error_type(status),
                &message,
            ));
        }
    }

    log::info!("✅ Backend responded successfully ({})", status);

    // ── Non-streaming path ──────────────────────────────────────────────
    if !client_wants_stream {
        // Backend returned a complete JSON response (stream: false).
        // Parse the OAI chat completion and translate to Claude Messages format.
        let body: Value = match res.json().await {
            Ok(body) => body,
            Err(e) => {
                log::error!("❌ Failed to parse non-streaming backend response: {}", e);
                return Ok(build_error_json_response(
                    StatusCode::BAD_GATEWAY,
                    "api_error",
                    "Failed to parse backend response",
                ));
            }
        };

        let model_name = body
            .get("model")
            .and_then(|m| m.as_str())
            .unwrap_or(&oai.model);

        let choice = body
            .get("choices")
            .and_then(|c| c.as_array())
            .and_then(|a| a.first());

        let message = choice.and_then(|c| c.get("message"));
        let finish_reason = choice
            .and_then(|c| c.get("finish_reason"))
            .and_then(|r| r.as_str());
        let stop_reason = translate_finish_reason(finish_reason);

        let mut content_blocks: Vec<Value> = Vec::new();

        // Extract reasoning_content → thinking block
        if let Some(reasoning) = message
            .and_then(|m| m.get("reasoning_content"))
            .and_then(|r| r.as_str())
        {
            if !reasoning.is_empty() {
                content_blocks.push(json!({
                    "type": "thinking",
                    "thinking": reasoning
                }));
            }
        }

        append_non_stream_text_content(&mut content_blocks, message.and_then(|m| m.get("content")));

        // Extract tool_calls → tool_use blocks
        if let Some(tool_calls) = message
            .and_then(|m| m.get("tool_calls"))
            .and_then(|t| t.as_array())
        {
            for tc in tool_calls {
                let id = tc.get("id").and_then(|v| v.as_str()).unwrap_or("");
                let func = tc.get("function");
                let name = func
                    .and_then(|f| f.get("name"))
                    .and_then(|n| n.as_str())
                    .unwrap_or("");
                let args_str = func
                    .and_then(|f| f.get("arguments"))
                    .and_then(|a| a.as_str())
                    .unwrap_or("{}");
                let input: Value = serde_json::from_str(args_str).unwrap_or(json!({}));
                content_blocks.push(json!({
                    "type": "tool_use",
                    "id": id,
                    "name": name,
                    "input": input
                }));
            }
        }

        // If no content at all, add empty text block
        if content_blocks.is_empty() {
            content_blocks.push(json!({"type": "text", "text": ""}));
        }

        // Extract usage
        let usage = body.get("usage");
        let prompt_tokens = usage
            .and_then(|u| u.get("prompt_tokens"))
            .and_then(|t| t.as_u64())
            .unwrap_or(input_token_count as u64) as u32;
        let completion_tokens = usage
            .and_then(|u| u.get("completion_tokens"))
            .and_then(|t| t.as_u64())
            .unwrap_or(0) as u32;

        // Record circuit breaker success
        {
            let mut cb = app.circuit_breaker.write().await;
            cb.record_success();
        }

        return Ok(build_non_streaming_response(
            model_name,
            content_blocks,
            stop_reason,
            prompt_tokens,
            completion_tokens,
        ));
    }

    // ── Streaming path ──────────────────────────────────────────────────
    let (tx, rx) = tokio::sync::mpsc::channel::<Event>(64);

    // Per-request ephemeral state for re-chunking.
    let model_for_header = oai.model.clone();

    tokio::spawn(async move {
        log::debug!("🎬 Streaming task started");

        // Emit Claude "message_start" - ensure content is always an array
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let message_obj = serde_json::json!({
            "id": format!("msg_{now}"),
            "type": "message",
            "role": "assistant",
            "content": serde_json::json!([]),  // Explicitly create empty array
            "model": model_for_header,
            "stop_reason": serde_json::Value::Null,
            "stop_sequence": serde_json::Value::Null,
            "usage": {
                "input_tokens": input_token_count,
                "output_tokens": 0
            }
        });

        let start = json!({
            "type": "message_start",
            "message": message_obj
        });

        // If we can't send message_start, client is gone - no point continuing
        if tx
            .send(
                Event::default()
                    .event("message_start")
                    .data(start.to_string()),
            )
            .await
            .is_err()
        {
            log::debug!("🔌 Client disconnected before message_start - aborting stream");
            return;
        }

        // Emit ping event per Anthropic SSE spec (keeps connection alive, signals stream health)
        let ping = json!({"type": "ping"});
        if tx
            .send(Event::default().event("ping").data(ping.to_string()))
            .await
            .is_err()
        {
            log::debug!("🔌 Client disconnected before ping");
            return;
        }

        let mut bytes_stream = res.bytes_stream();

        let mut stream_state = StreamState {
            next_block_index: 0,
            thinking_open: false,
            thinking_index: -1,
            text_open: false,
            text_index: -1,
            tools: HashMap::new(),
            final_stop_reason: "end_turn",
            output_token_count: 0,
            backend_reported_usage: false,
        };
        let mut sse_parser = SseEventParser::new();
        let mut done = false;
        let mut fatal_error = false;

        log::debug!("🌊 Begin processing SSE from backend");
        while let Some(item) = bytes_stream.next().await {
            let chunk = match item {
                Ok(chunk) => chunk,
                Err(_) => {
                    log::debug!("❌ Error reading chunk from stream");
                    break;
                }
            };

            for payload in sse_parser.push_and_drain_events(&chunk) {
                let data = payload.trim();
                match process_backend_sse_payload(&tx, &mut stream_state, data).await {
                    StreamPayloadOutcome::Continue => {}
                    StreamPayloadOutcome::Done => {
                        done = true;
                        break;
                    }
                    StreamPayloadOutcome::FatalError => {
                        done = true;
                        fatal_error = true;
                        break;
                    }
                    StreamPayloadOutcome::ClientDisconnected => {
                        done = true;
                        break;
                    }
                }
            }

            if fatal_error {
                break;
            }

            if done {
                break;
            }
        }

        // Flush any trailing event if backend didn't send final blank line
        if !done {
            if let Some(payload) = sse_parser.flush() {
                let data = payload.trim();
                match process_backend_sse_payload(&tx, &mut stream_state, data).await {
                    StreamPayloadOutcome::Continue | StreamPayloadOutcome::Done => {}
                    StreamPayloadOutcome::FatalError => {
                        fatal_error = true;
                    }
                    StreamPayloadOutcome::ClientDisconnected => {}
                }
            }
        }

        // Close any open blocks and finish message
        if stream_state.thinking_open {
            let ev = json!({ "type":"content_block_stop", "index":stream_state.thinking_index });
            let _ = tx
                .send(
                    Event::default()
                        .event("content_block_stop")
                        .data(ev.to_string()),
                )
                .await;
            log::info!(
                "🧠 OUTPUT: Closed thinking block at end (index={})",
                stream_state.thinking_index
            );
        }
        if stream_state.text_open {
            let ev = json!({ "type":"content_block_stop", "index":stream_state.text_index });
            let _ = tx
                .send(
                    Event::default()
                        .event("content_block_stop")
                        .data(ev.to_string()),
                )
                .await;
        }
        for tb in stream_state.tools.values() {
            let stop = json!({ "type":"content_block_stop", "index":tb.block_index });
            let _ = tx
                .send(
                    Event::default()
                        .event("content_block_stop")
                        .data(stop.to_string()),
                )
                .await;
        }

        let md = json!({
            "type":"message_delta",
            "delta":{"stop_reason":stream_state.final_stop_reason,"stop_sequence":null},
            "usage":{"output_tokens":stream_state.output_token_count}
        });
        // Critical: if these final events fail, stream is incomplete - but log it
        if tx
            .send(Event::default().event("message_delta").data(md.to_string()))
            .await
            .is_err()
        {
            log::debug!("🔌 Client disconnected before message_delta");
            return;
        }

        if tx
            .send(
                Event::default()
                    .event("message_stop")
                    .data(json!({"type":"message_stop"}).to_string()),
            )
            .await
            .is_err()
        {
            log::debug!("🔌 Client disconnected before message_stop");
            return;
        }

        log::debug!("🏁 Streaming task completed");

        // Drain any remaining bytes from backend stream to avoid cancelling the request
        // This ensures the backend doesn't see a connection reset/cancellation
        log::debug!("🔄 Draining remaining backend stream...");
        let mut drained_bytes = 0;
        while let Some(item) = bytes_stream.next().await {
            if let Ok(chunk) = item {
                drained_bytes += chunk.len();
            }
        }
        if drained_bytes > 0 {
            log::debug!(
                "🔄 Drained {} additional bytes from backend stream",
                drained_bytes
            );
        } else {
            log::debug!("✅ Backend stream was already fully consumed");
        }

        // Record circuit breaker success if no fatal error
        if !fatal_error {
            let cb_clone = app.circuit_breaker.clone();
            tokio::spawn(async move {
                cb_clone.write().await.record_success();
            });
        }
    });

    // Log structured metrics
    if let Ok(elapsed) = request_start.elapsed() {
        log::info!(target: "metrics",
            "request_completed: model={}, duration_ms={}, messages={}, status=success",
            backend_model_for_metrics, elapsed.as_millis(), original_message_count
        );
    }

    Ok(into_sse_response(rx))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{App, CircuitBreakerState};
    use axum::{
        body::to_bytes,
        extract::State,
        http::{
            header::{AUTHORIZATION, CONTENT_TYPE},
            HeaderMap, HeaderValue, StatusCode,
        },
        response::{IntoResponse, Response},
        routing::{get, post},
        Json, Router,
    };
    use serde_json::json;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    #[test]
    fn test_content_uses_cache_control_detects_block_marker() {
        assert!(content_uses_cache_control(&json!([
            {"type": "text", "text": "cached", "cache_control": {"type": "ephemeral"}}
        ])));
        assert!(!content_uses_cache_control(&json!([
            {"type": "text", "text": "plain"}
        ])));
    }

    #[test]
    fn test_extract_backend_error_message_prefers_nested_error_message() {
        let message = extract_backend_error_message(
            r#"{"error":{"message":"upstream quota exceeded","type":"insufficient_quota"}}"#,
        );
        assert_eq!(message, "upstream quota exceeded");
    }

    #[test]
    fn test_append_non_stream_text_content_supports_array_blocks() {
        let mut content_blocks = Vec::new();
        append_non_stream_text_content(
            &mut content_blocks,
            Some(&json!([
                {"type": "text", "text": "first"},
                {"type": "text", "text": "second"}
            ])),
        );

        assert_eq!(
            content_blocks,
            vec![
                json!({"type": "text", "text": "first"}),
                json!({"type": "text", "text": "second"}),
            ]
        );
    }

    fn auth_headers() -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer test"));
        headers
    }

    fn test_app(backend_url: String, circuit_open: bool) -> App {
        App {
            client: reqwest::Client::new(),
            backend_url,
            backend_host_header: None,
            models_cache: Arc::new(RwLock::new(None)),
            circuit_breaker: Arc::new(RwLock::new(CircuitBreakerState {
                consecutive_failures: if circuit_open { 5 } else { 0 },
                last_failure_time: None,
                is_open: circuit_open,
                enabled: circuit_open,
            })),
        }
    }

    async fn spawn_mock_backend() -> String {
        async fn models_handler() -> Json<Value> {
            Json(json!({
                "data": [
                    {
                        "id": "zai-org/GLM-4.5-Air",
                        "price": { "input": { "usd": 0.1 }, "output": { "usd": 0.2 } },
                        "supported_features": []
                    },
                    {
                        "id": "deepseek-r1",
                        "price": { "input": { "usd": 0.2 }, "output": { "usd": 0.4 } },
                        "supported_features": ["thinking", "extended_thinking"]
                    }
                ]
            }))
        }

        async fn chat_handler(Json(payload): Json<Value>) -> Response {
            let model = payload
                .get("model")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            let stream = payload
                .get("stream")
                .and_then(|value| value.as_bool())
                .unwrap_or(false);
            let has_tools = payload
                .get("tools")
                .and_then(|value| value.as_array())
                .map(|tools| !tools.is_empty())
                .unwrap_or(false);
            let include_reasoning = model == "deepseek-r1";

            if model == "missing-model" {
                return (
                    StatusCode::NOT_FOUND,
                    Json(json!({
                        "error": {
                            "message": format!("model '{model}' not found"),
                            "type": "not_found"
                        }
                    })),
                )
                    .into_response();
            }

            if stream {
                let mut chunks = vec![json!({
                    "id": "chatcmpl-mock",
                    "object": "chat.completion.chunk",
                    "model": model,
                    "choices": [{ "index": 0, "delta": { "role": "assistant" }, "finish_reason": Value::Null }]
                })];

                if include_reasoning {
                    chunks.push(json!({
                        "id": "chatcmpl-mock",
                        "object": "chat.completion.chunk",
                        "model": model,
                        "choices": [{ "index": 0, "delta": { "reasoning_content": "I reasoned briefly." }, "finish_reason": Value::Null }]
                    }));
                }

                if has_tools {
                    chunks.push(json!({
                        "id": "chatcmpl-mock",
                        "object": "chat.completion.chunk",
                        "model": model,
                        "choices": [{
                            "index": 0,
                            "delta": {
                                "tool_calls": [{
                                    "index": 0,
                                    "id": "call_mock_1",
                                    "type": "function",
                                    "function": { "name": "read_file" }
                                }]
                            },
                            "finish_reason": Value::Null
                        }]
                    }));
                    chunks.push(json!({
                        "id": "chatcmpl-mock",
                        "object": "chat.completion.chunk",
                        "model": model,
                        "choices": [{
                            "index": 0,
                            "delta": {
                                "tool_calls": [{
                                    "index": 0,
                                    "function": { "arguments": "{\"path\":\"README.md\"}" }
                                }]
                            },
                            "finish_reason": Value::Null
                        }]
                    }));
                } else {
                    chunks.push(json!({
                        "id": "chatcmpl-mock",
                        "object": "chat.completion.chunk",
                        "model": model,
                        "choices": [{ "index": 0, "delta": { "content": "Mock backend response." }, "finish_reason": Value::Null }]
                    }));
                }

                chunks.push(json!({
                    "id": "chatcmpl-mock",
                    "object": "chat.completion.chunk",
                    "model": model,
                    "choices": [{ "index": 0, "delta": {}, "finish_reason": if has_tools { "tool_calls" } else { "stop" } }],
                    "usage": { "prompt_tokens": 12, "completion_tokens": 8, "total_tokens": 20 }
                }));

                let body = format!(
                    "{}data: [DONE]\n\n",
                    chunks
                        .into_iter()
                        .map(|chunk| format!("data: {chunk}\n\n"))
                        .collect::<String>()
                );
                let mut headers = HeaderMap::new();
                headers.insert(CONTENT_TYPE, HeaderValue::from_static("text/event-stream"));
                return (headers, body).into_response();
            }

            let mut message = json!({
                "role": "assistant",
                "content": "Mock backend response."
            });

            if include_reasoning {
                message["reasoning_content"] = json!("I reasoned briefly.");
            }

            if has_tools {
                message = json!({
                    "role": "assistant",
                    "content": "",
                    "tool_calls": [{
                        "id": "call_mock_1",
                        "type": "function",
                        "function": {
                            "name": "read_file",
                            "arguments": "{\"path\":\"README.md\"}"
                        }
                    }]
                });
            }

            Json(json!({
                "id": "chatcmpl-mock",
                "object": "chat.completion",
                "model": model,
                "choices": [{
                    "index": 0,
                    "message": message,
                    "finish_reason": if has_tools { "tool_calls" } else { "stop" }
                }],
                "usage": { "prompt_tokens": 12, "completion_tokens": 8, "total_tokens": 20 }
            }))
            .into_response()
        }

        let app = Router::new()
            .route("/v1/models", get(models_handler))
            .route("/v1/chat/completions", post(chat_handler));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let models_url = format!("http://{addr}/v1/models");
        let client = reqwest::Client::new();
        for _ in 0..20 {
            if client.get(&models_url).send().await.is_ok() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }

        format!("http://{addr}/v1/chat/completions")
    }

    async fn spawn_capturing_backend() -> (String, Arc<RwLock<Vec<Value>>>) {
        async fn models_handler() -> Json<Value> {
            Json(json!({
                "data": [
                    {
                        "id": "zai-org/GLM-4.5-Air",
                        "price": { "input": { "usd": 0.1 }, "output": { "usd": 0.2 } },
                        "supported_features": []
                    },
                    {
                        "id": "deepseek-r1",
                        "price": { "input": { "usd": 0.2 }, "output": { "usd": 0.4 } },
                        "supported_features": ["thinking", "extended_thinking"]
                    }
                ]
            }))
        }

        let captured = Arc::new(RwLock::new(Vec::<Value>::new()));
        let captured_for_route = captured.clone();

        let app = Router::new()
            .route("/v1/models", get(models_handler))
            .route(
                "/v1/chat/completions",
                post(move |Json(payload): Json<Value>| {
                    let captured = captured_for_route.clone();
                    async move {
                        captured.write().await.push(payload.clone());

                        if payload
                            .get("stream")
                            .and_then(|value| value.as_bool())
                            .unwrap_or(false)
                        {
                            let body = format!(
                                "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
                                json!({
                                    "id": "chatcmpl-capture",
                                    "object": "chat.completion.chunk",
                                    "model": payload["model"],
                                    "choices": [{ "index": 0, "delta": { "role": "assistant" }, "finish_reason": Value::Null }]
                                }),
                                json!({
                                    "id": "chatcmpl-capture",
                                    "object": "chat.completion.chunk",
                                    "model": payload["model"],
                                    "choices": [{ "index": 0, "delta": { "content": "captured" }, "finish_reason": "stop" }],
                                    "usage": { "prompt_tokens": 2, "completion_tokens": 1, "total_tokens": 3 }
                                })
                            );
                            let mut headers = HeaderMap::new();
                            headers.insert(
                                CONTENT_TYPE,
                                HeaderValue::from_static("text/event-stream"),
                            );
                            (headers, body).into_response()
                        } else {
                            Json(json!({
                                "id": "chatcmpl-capture",
                                "object": "chat.completion",
                                "model": payload["model"],
                                "choices": [{
                                    "index": 0,
                                    "message": { "role": "assistant", "content": "captured" },
                                    "finish_reason": "stop"
                                }],
                                "usage": { "prompt_tokens": 2, "completion_tokens": 1, "total_tokens": 3 }
                            }))
                            .into_response()
                        }
                    }
                }),
            );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let models_url = format!("http://{addr}/v1/models");
        let client = reqwest::Client::new();
        for _ in 0..20 {
            if client.get(&models_url).send().await.is_ok() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }

        (format!("http://{addr}/v1/chat/completions"), captured)
    }

    async fn spawn_static_backend(
        chat_status: StatusCode,
        chat_content_type: &'static str,
        chat_body: String,
        models_status: StatusCode,
        models_body: Value,
    ) -> String {
        let app = Router::new()
            .route(
                "/v1/models",
                get({
                    let models_body = models_body.clone();
                    move || {
                        let models_body = models_body.clone();
                        async move { (models_status, Json(models_body)).into_response() }
                    }
                }),
            )
            .route(
                "/v1/chat/completions",
                post({
                    let chat_body = chat_body.clone();
                    move || {
                        let chat_body = chat_body.clone();
                        async move {
                            let mut response = Response::new(axum::body::Body::from(chat_body));
                            *response.status_mut() = chat_status;
                            response
                                .headers_mut()
                                .insert(CONTENT_TYPE, HeaderValue::from_static(chat_content_type));
                            response
                        }
                    }
                }),
            );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        format!("http://{addr}/v1/chat/completions")
    }

    async fn unused_backend_url() -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        format!("http://{addr}/v1/chat/completions")
    }

    async fn response_text(response: Response) -> (StatusCode, String) {
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, String::from_utf8(bytes.to_vec()).unwrap())
    }

    fn request_from_value(value: Value) -> ClaudeRequest {
        serde_json::from_value(value).unwrap()
    }

    #[tokio::test]
    async fn test_messages_streaming_success_returns_claude_sse() {
        let app = test_app(spawn_mock_backend().await, false);
        let request = request_from_value(json!({
            "model": "zai-org/GLM-4.5-Air",
            "messages": [{ "role": "user", "content": "hi" }],
            "max_tokens": 32,
            "stream": true
        }));

        let response = messages(State(app), auth_headers(), axum::Json(request))
            .await
            .unwrap();
        let (status, body) = response_text(response).await;

        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("event: message_start"));
        assert!(body.contains("event: content_block_delta"));
        assert!(body.contains("Mock backend response."));
        assert!(body.contains("event: message_stop"));
    }

    #[tokio::test]
    async fn test_messages_streaming_flushes_trailing_reasoning_without_final_blank_line() {
        let body = format!(
            "data: {}\n\ndata: {}",
            json!({
                "id": "chatcmpl-tail",
                "object": "chat.completion.chunk",
                "model": "deepseek-r1",
                "choices": [{ "index": 0, "delta": { "role": "assistant" }, "finish_reason": Value::Null }]
            }),
            json!({
                "id": "chatcmpl-tail",
                "object": "chat.completion.chunk",
                "model": "deepseek-r1",
                "choices": [{ "index": 0, "delta": { "reasoning_content": "tail reasoning" }, "finish_reason": "stop" }],
                "usage": { "prompt_tokens": 3, "completion_tokens": 2, "total_tokens": 5 }
            })
        );
        let backend_url = spawn_static_backend(
            StatusCode::OK,
            "text/event-stream",
            body,
            StatusCode::OK,
            json!({ "data": [] }),
        )
        .await;
        let app = test_app(backend_url, false);
        let request = request_from_value(json!({
            "model": "deepseek-r1",
            "messages": [{ "role": "user", "content": "hi" }],
            "max_tokens": 32,
            "stream": true
        }));

        let response = messages(State(app), auth_headers(), axum::Json(request))
            .await
            .unwrap();
        let (status, body) = response_text(response).await;

        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("\"type\":\"thinking_delta\""));
        assert!(body.contains("tail reasoning"));
        assert!(body.contains("event: message_stop"));
    }

    #[tokio::test]
    async fn test_messages_streaming_flushes_trailing_tool_arguments_without_final_blank_line() {
        let body = format!(
            "data: {}\n\ndata: {}\n\ndata: {}",
            json!({
                "id": "chatcmpl-tail-tools",
                "object": "chat.completion.chunk",
                "model": "zai-org/GLM-4.5-Air",
                "choices": [{ "index": 0, "delta": { "role": "assistant" }, "finish_reason": Value::Null }]
            }),
            json!({
                "id": "chatcmpl-tail-tools",
                "object": "chat.completion.chunk",
                "model": "zai-org/GLM-4.5-Air",
                "choices": [{
                    "index": 0,
                    "delta": {
                        "tool_calls": [{
                            "index": 0,
                            "id": "call_tail_1",
                            "type": "function",
                            "function": { "name": "read_file" }
                        }]
                    },
                    "finish_reason": Value::Null
                }]
            }),
            json!({
                "id": "chatcmpl-tail-tools",
                "object": "chat.completion.chunk",
                "model": "zai-org/GLM-4.5-Air",
                "choices": [{
                    "index": 0,
                    "delta": {
                        "tool_calls": [{
                            "index": 0,
                            "function": { "arguments": "{\"path\":\"README.md\"}" }
                        }]
                    },
                    "finish_reason": "tool_calls"
                }],
                "usage": { "prompt_tokens": 4, "completion_tokens": 2, "total_tokens": 6 }
            })
        );
        let backend_url = spawn_static_backend(
            StatusCode::OK,
            "text/event-stream",
            body,
            StatusCode::OK,
            json!({ "data": [] }),
        )
        .await;
        let app = test_app(backend_url, false);
        let request = request_from_value(json!({
            "model": "zai-org/GLM-4.5-Air",
            "messages": [{ "role": "user", "content": "hi" }],
            "max_tokens": 32,
            "stream": true
        }));

        let response = messages(State(app), auth_headers(), axum::Json(request))
            .await
            .unwrap();
        let (status, body) = response_text(response).await;

        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("\"type\":\"tool_use\""));
        assert!(body.contains("\"type\":\"input_json_delta\""));
        assert!(body.contains("{\\\"path\\\":\\\"README.md\\\"}"));
        assert!(body.contains("event: message_stop"));
    }

    #[tokio::test]
    async fn test_messages_non_streaming_success_returns_claude_json() {
        let app = test_app(spawn_mock_backend().await, false);
        let request = request_from_value(json!({
            "model": "deepseek-r1",
            "messages": [{ "role": "user", "content": "hi" }],
            "max_tokens": 32,
            "stream": false
        }));

        let response = messages(State(app), auth_headers(), axum::Json(request))
            .await
            .unwrap();
        let (status, body) = response_text(response).await;
        let parsed: Value = serde_json::from_str(&body).unwrap();

        assert_eq!(status, StatusCode::OK);
        assert_eq!(parsed["type"], "message");
        assert_eq!(parsed["role"], "assistant");
        assert_eq!(parsed["content"][0]["type"], "thinking");
        assert_eq!(parsed["content"][1]["type"], "text");
    }

    #[tokio::test]
    async fn test_messages_non_streaming_tool_calls_become_tool_use_blocks() {
        let app = test_app(spawn_mock_backend().await, false);
        let request = request_from_value(json!({
            "model": "zai-org/GLM-4.5-Air",
            "messages": [{ "role": "user", "content": "read a file" }],
            "tools": [{
                "name": "read_file",
                "description": "Read file",
                "input_schema": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" }
                    }
                }
            }],
            "max_tokens": 32,
            "stream": false
        }));

        let response = messages(State(app), auth_headers(), axum::Json(request))
            .await
            .unwrap();
        let (_, body) = response_text(response).await;
        let parsed: Value = serde_json::from_str(&body).unwrap();

        assert_eq!(parsed["content"][0]["type"], "tool_use");
        assert_eq!(parsed["content"][0]["name"], "read_file");
        assert_eq!(parsed["content"][0]["input"]["path"], "README.md");
        assert_eq!(parsed["stop_reason"], "tool_use");
    }

    #[tokio::test]
    async fn test_messages_streaming_missing_model_returns_synthetic_response() {
        let app = test_app(spawn_mock_backend().await, false);
        let request = request_from_value(json!({
            "model": "missing-model",
            "messages": [{ "role": "user", "content": "hi" }],
            "max_tokens": 32,
            "stream": true
        }));

        let response = messages(State(app), auth_headers(), axum::Json(request))
            .await
            .unwrap();
        let (_, body) = response_text(response).await;

        assert!(body.contains("Model `missing-model` not found"));
        assert!(body.contains("zai-org/GLM-4.5-Air"));
        assert!(body.contains("event: message_stop"));
    }

    #[tokio::test]
    async fn test_messages_non_streaming_missing_model_returns_model_list_message() {
        let app = test_app(spawn_mock_backend().await, false);
        let request = request_from_value(json!({
            "model": "missing-model",
            "messages": [{ "role": "user", "content": "hi" }],
            "max_tokens": 32,
            "stream": false
        }));

        let response = messages(State(app), auth_headers(), axum::Json(request))
            .await
            .unwrap();
        let (status, body) = response_text(response).await;
        let parsed: Value = serde_json::from_str(&body).unwrap();

        assert_eq!(status, StatusCode::OK);
        assert_eq!(parsed["type"], "message");
        assert!(parsed["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Model `missing-model` not found"));
    }

    #[tokio::test]
    async fn test_messages_non_streaming_missing_auth_returns_json_error() {
        let app = test_app(spawn_mock_backend().await, false);
        let request = request_from_value(json!({
            "model": "zai-org/GLM-4.5-Air",
            "messages": [{ "role": "user", "content": "hi" }],
            "max_tokens": 32,
            "stream": false
        }));

        let response = messages(State(app), HeaderMap::new(), axum::Json(request))
            .await
            .unwrap();
        let (status, body) = response_text(response).await;
        let parsed: Value = serde_json::from_str(&body).unwrap();

        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(parsed["type"], "error");
        assert_eq!(parsed["error"]["type"], "authentication_error");
    }

    #[tokio::test]
    async fn test_messages_non_streaming_empty_messages_returns_json_error() {
        let app = test_app(spawn_mock_backend().await, false);
        let request = request_from_value(json!({
            "model": "zai-org/GLM-4.5-Air",
            "messages": [],
            "max_tokens": 32,
            "stream": false
        }));

        let response = messages(State(app), auth_headers(), axum::Json(request))
            .await
            .unwrap();
        let (status, body) = response_text(response).await;
        let parsed: Value = serde_json::from_str(&body).unwrap();

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(parsed["error"]["type"], "invalid_request_error");
        assert_eq!(parsed["error"]["message"], "messages must not be empty");
    }

    #[tokio::test]
    async fn test_messages_non_streaming_open_circuit_breaker_returns_json_error() {
        let app = test_app(spawn_mock_backend().await, true);
        let request = request_from_value(json!({
            "model": "zai-org/GLM-4.5-Air",
            "messages": [{ "role": "user", "content": "hi" }],
            "max_tokens": 32,
            "stream": false
        }));

        let response = messages(State(app), auth_headers(), axum::Json(request))
            .await
            .unwrap();
        let (status, body) = response_text(response).await;
        let parsed: Value = serde_json::from_str(&body).unwrap();

        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(parsed["error"]["type"], "overloaded_error");
    }

    #[tokio::test]
    async fn test_messages_streaming_connection_failure_returns_bad_gateway_tuple() {
        let app = test_app(unused_backend_url().await, false);
        let request = request_from_value(json!({
            "model": "zai-org/GLM-4.5-Air",
            "messages": [{ "role": "user", "content": "hi" }],
            "max_tokens": 32,
            "stream": true
        }));

        let error = messages(State(app), auth_headers(), axum::Json(request))
            .await
            .unwrap_err();

        assert_eq!(error, (StatusCode::BAD_GATEWAY, "backend_unavailable"));
    }

    #[tokio::test]
    async fn test_messages_non_streaming_connection_failure_returns_json_error() {
        let app = test_app(unused_backend_url().await, false);
        let request = request_from_value(json!({
            "model": "zai-org/GLM-4.5-Air",
            "messages": [{ "role": "user", "content": "hi" }],
            "max_tokens": 32,
            "stream": false
        }));

        let response = messages(State(app), auth_headers(), axum::Json(request))
            .await
            .unwrap();
        let (status, body) = response_text(response).await;
        let parsed: Value = serde_json::from_str(&body).unwrap();

        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert_eq!(parsed["error"]["type"], "api_error");
        assert_eq!(
            parsed["error"]["message"],
            "Failed to connect to the configured backend"
        );
    }

    #[tokio::test]
    async fn test_messages_streaming_retryable_backend_error_returns_status_tuple() {
        let backend_url = spawn_static_backend(
            StatusCode::TOO_MANY_REQUESTS,
            "application/json",
            json!({ "error": { "message": "slow down" } }).to_string(),
            StatusCode::OK,
            json!({ "data": [] }),
        )
        .await;
        let app = test_app(backend_url, false);
        let request = request_from_value(json!({
            "model": "zai-org/GLM-4.5-Air",
            "messages": [{ "role": "user", "content": "hi" }],
            "max_tokens": 32,
            "stream": true
        }));

        let error = messages(State(app), auth_headers(), axum::Json(request))
            .await
            .unwrap_err();

        assert_eq!(
            error,
            (StatusCode::TOO_MANY_REQUESTS, "backend_error_retryable")
        );
    }

    #[tokio::test]
    async fn test_messages_non_streaming_retryable_backend_error_returns_json_error() {
        let backend_url = spawn_static_backend(
            StatusCode::TOO_MANY_REQUESTS,
            "application/json",
            json!({ "error": { "message": "slow down" } }).to_string(),
            StatusCode::OK,
            json!({ "data": [] }),
        )
        .await;
        let app = test_app(backend_url, false);
        let request = request_from_value(json!({
            "model": "zai-org/GLM-4.5-Air",
            "messages": [{ "role": "user", "content": "hi" }],
            "max_tokens": 32,
            "stream": false
        }));

        let response = messages(State(app), auth_headers(), axum::Json(request))
            .await
            .unwrap();
        let (status, body) = response_text(response).await;
        let parsed: Value = serde_json::from_str(&body).unwrap();

        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(parsed["error"]["type"], "rate_limit_error");
        assert_eq!(parsed["error"]["message"], "slow down");
    }

    #[tokio::test]
    async fn test_messages_streaming_non_retryable_backend_error_returns_sse_error() {
        let backend_url = spawn_static_backend(
            StatusCode::FORBIDDEN,
            "application/json",
            json!({ "error": { "message": "access denied" } }).to_string(),
            StatusCode::OK,
            json!({ "data": [] }),
        )
        .await;
        let app = test_app(backend_url, false);
        let request = request_from_value(json!({
            "model": "zai-org/GLM-4.5-Air",
            "messages": [{ "role": "user", "content": "hi" }],
            "max_tokens": 32,
            "stream": true
        }));

        let response = messages(State(app), auth_headers(), axum::Json(request))
            .await
            .unwrap();
        let (status, body) = response_text(response).await;

        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("access denied"));
        assert!(body.contains("event: message_stop"));
    }

    #[tokio::test]
    async fn test_messages_non_streaming_backend_parse_failure_returns_json_error() {
        let backend_url = spawn_static_backend(
            StatusCode::OK,
            "application/json",
            "{not-json".into(),
            StatusCode::OK,
            json!({ "data": [] }),
        )
        .await;
        let app = test_app(backend_url, false);
        let request = request_from_value(json!({
            "model": "zai-org/GLM-4.5-Air",
            "messages": [{ "role": "user", "content": "hi" }],
            "max_tokens": 32,
            "stream": false
        }));

        let response = messages(State(app), auth_headers(), axum::Json(request))
            .await
            .unwrap();
        let (status, body) = response_text(response).await;
        let parsed: Value = serde_json::from_str(&body).unwrap();

        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert_eq!(parsed["error"]["type"], "api_error");
        assert_eq!(
            parsed["error"]["message"],
            "Failed to parse backend response"
        );
    }

    #[tokio::test]
    async fn test_messages_translates_assistant_thinking_and_tool_results_for_backend() {
        let (backend_url, captured) = spawn_capturing_backend().await;
        let app = test_app(backend_url, false);
        let request = request_from_value(json!({
            "model": "deepseek-r1",
            "thinking": { "type": "enabled", "budget_tokens": 256 },
            "messages": [
                {
                    "role": "assistant",
                    "content": [
                        { "type": "thinking", "thinking": "step one", "signature": "sig_123" },
                        { "type": "redacted_thinking", "data": "redacted" },
                        { "type": "text", "text": "I will use a tool" },
                        { "type": "tool_use", "id": "call_1", "name": "read_file", "input": { "path": "README.md" } }
                    ]
                },
                {
                    "role": "user",
                    "content": [
                        { "type": "tool_result", "tool_use_id": "call_1", "content": "contents" },
                        { "type": "text", "text": "thanks" }
                    ]
                }
            ],
            "max_tokens": 32,
            "stream": false
        }));

        messages(State(app), auth_headers(), axum::Json(request))
            .await
            .unwrap();

        let captured = captured.read().await;
        let payload = &captured[0];
        let messages = payload["messages"].as_array().unwrap();

        assert_eq!(messages[0]["role"], "assistant");
        assert!(messages[0]["content"]
            .as_str()
            .unwrap()
            .contains("<think>step one\n[redacted reasoning]</think>"));
        assert_eq!(
            messages[0]["tool_calls"][0]["function"]["name"],
            json!("read_file")
        );
        assert_eq!(messages[1]["role"], "tool");
        assert_eq!(messages[1]["tool_call_id"], json!("call_1"));
        assert_eq!(messages[1]["content"], json!("contents"));
        assert_eq!(messages[2]["role"], "user");
        assert_eq!(messages[2]["content"], json!("thanks"));
        assert_eq!(payload["thinking"]["type"], json!("enabled"));
    }

    #[tokio::test]
    async fn test_messages_auto_enables_thinking_for_reasoning_model_from_cache() {
        let (backend_url, captured) = spawn_capturing_backend().await;
        let app = test_app(backend_url, false);
        *app.models_cache.write().await = Some(vec![crate::models::ModelInfo {
            id: "deepseek-r1".into(),
            input_price_usd: None,
            output_price_usd: None,
            supported_features: vec!["thinking".into()],
        }]);

        let request = request_from_value(json!({
            "model": "deepseek-r1",
            "messages": [{ "role": "user", "content": "reason this through" }],
            "max_tokens": 32,
            "stream": false
        }));

        messages(State(app), auth_headers(), axum::Json(request))
            .await
            .unwrap();

        let captured = captured.read().await;
        let payload = &captured[0];

        assert_eq!(payload["thinking"]["type"], json!("enabled"));
        assert_eq!(
            payload["thinking"]["budget_tokens"],
            json!(crate::constants::DEFAULT_THINKING_BUDGET_TOKENS)
        );
    }

    #[tokio::test]
    async fn test_messages_accepts_claude_code_adaptive_thinking() {
        let (backend_url, captured) = spawn_capturing_backend().await;
        let app = test_app(backend_url, false);

        let request = request_from_value(json!({
            "model": "zai-org/GLM-5-TEE",
            "thinking": { "type": "adaptive" },
            "output_config": { "effort": "low" },
            "messages": [{ "role": "user", "content": "hello" }],
            "max_tokens": 32,
            "stream": false
        }));

        messages(State(app), auth_headers(), axum::Json(request))
            .await
            .unwrap();

        let captured = captured.read().await;
        let payload = &captured[0];

        assert_eq!(payload["thinking"]["type"], json!("enabled"));
        assert_eq!(
            payload["thinking"]["budget_tokens"],
            json!(crate::constants::DEFAULT_THINKING_BUDGET_TOKENS)
        );
        assert_eq!(payload["reasoning_effort"], json!("low"));
    }

    #[tokio::test]
    async fn test_messages_disabled_thinking_skips_backend_thinking_field() {
        let (backend_url, captured) = spawn_capturing_backend().await;
        let app = test_app(backend_url, false);
        *app.models_cache.write().await = Some(vec![crate::models::ModelInfo {
            id: "deepseek-r1".into(),
            input_price_usd: None,
            output_price_usd: None,
            supported_features: vec!["thinking".into()],
        }]);

        let request = request_from_value(json!({
            "model": "deepseek-r1",
            "thinking": { "type": "disabled" },
            "messages": [{ "role": "user", "content": "hello" }],
            "max_tokens": 32,
            "stream": false
        }));

        messages(State(app), auth_headers(), axum::Json(request))
            .await
            .unwrap();

        let captured = captured.read().await;
        let payload = &captured[0];

        assert!(payload.get("thinking").is_none());
    }

    #[tokio::test]
    async fn test_messages_unknown_content_blocks_degrade_to_text_placeholders() {
        let (backend_url, captured) = spawn_capturing_backend().await;
        let app = test_app(backend_url, false);
        let request = request_from_value(json!({
            "model": "zai-org/GLM-4.5-Air",
            "messages": [
                {
                    "role": "assistant",
                    "content": [
                        {
                            "type": "server_tool_use",
                            "name": "bash",
                            "input": { "command": "ls" }
                        },
                        { "type": "text", "text": "ran a command" }
                    ]
                },
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "search_result",
                            "query": "weather in New York",
                            "content": [{ "type": "text", "text": "Sunny and 68F" }]
                        },
                        { "type": "text", "text": "what do you think?" }
                    ]
                }
            ],
            "max_tokens": 32,
            "stream": false
        }));

        messages(State(app), auth_headers(), axum::Json(request))
            .await
            .unwrap();

        let captured = captured.read().await;
        let payload = &captured[0];
        let messages = payload["messages"].as_array().unwrap();

        assert!(messages[0]["content"].is_string());
        assert!(messages[0]["content"]
            .as_str()
            .unwrap()
            .contains("Unsupported Claude content block: server_tool_use (bash)"));
        assert!(messages[0]["content"]
            .as_str()
            .unwrap()
            .contains("ran a command"));

        assert!(messages[1]["content"].is_string());
        assert!(messages[1]["content"]
            .as_str()
            .unwrap()
            .contains("Unsupported Claude content block: search_result"));
        assert!(messages[1]["content"]
            .as_str()
            .unwrap()
            .contains("what do you think?"));
    }

    #[tokio::test]
    async fn test_messages_truncates_stop_sequences_for_backend_payload() {
        let (backend_url, captured) = spawn_capturing_backend().await;
        let app = test_app(backend_url, false);
        let request = request_from_value(json!({
            "model": "zai-org/GLM-4.5-Air",
            "messages": [{ "role": "user", "content": "hi" }],
            "stop_sequences": ["a", "b", "c", "d", "e"],
            "max_tokens": 32,
            "stream": false
        }));

        messages(State(app), auth_headers(), axum::Json(request))
            .await
            .unwrap();

        let captured = captured.read().await;
        let stop = captured[0]["stop"].as_array().unwrap();

        assert_eq!(stop.len(), 4);
        assert_eq!(stop[0], json!("a"));
        assert_eq!(stop[3], json!("d"));
    }

    #[tokio::test]
    async fn test_messages_translates_documents_for_backend_payload() {
        let (backend_url, captured) = spawn_capturing_backend().await;
        let app = test_app(backend_url, false);
        let request = request_from_value(json!({
            "model": "zai-org/GLM-4.5-Air",
            "messages": [{
                "role": "user",
                "content": [
                    { "type": "text", "text": "see attached" },
                    {
                        "type": "document",
                        "source": {
                            "type": "base64",
                            "media_type": "application/pdf",
                            "data": "ZmFrZQ=="
                        }
                    },
                    {
                        "type": "document",
                        "source": {
                            "type": "file",
                            "file_id": "file_123"
                        }
                    },
                    {
                        "type": "document",
                        "source": {
                            "type": "url",
                            "url": "https://example.com/file.pdf"
                        }
                    }
                ]
            }],
            "max_tokens": 32,
            "stream": false
        }));

        messages(State(app), auth_headers(), axum::Json(request))
            .await
            .unwrap();

        let captured = captured.read().await;
        let payload = &captured[0];
        let content = payload["messages"][0]["content"].as_array().unwrap();

        assert_eq!(payload["messages"][0]["role"], json!("user"));
        assert_eq!(content[0]["type"], json!("text"));
        assert_eq!(content[1]["type"], json!("file"));
        assert_eq!(
            content[1]["file"]["file_data"],
            json!("data:application/pdf;base64,ZmFrZQ==")
        );
        assert_eq!(content[1]["file"]["filename"], json!("document.pdf"));
        assert_eq!(content[2]["type"], json!("file"));
        assert_eq!(content[2]["file"]["file_id"], json!("file_123"));
        assert_eq!(content[3]["type"], json!("text"));
        assert!(content[3]["text"]
            .as_str()
            .unwrap()
            .contains("Attached document URL (unknown): https://example.com/file.pdf"));
    }
}
