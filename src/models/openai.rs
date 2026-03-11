use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Serialize)]
pub struct OAIMessage {
    pub role: String,
    pub content: Value, // String or Array for multimodal
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<Value>>,
}

#[derive(Serialize)]
pub struct OAIFunction {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub parameters: Value,
}

#[derive(Serialize)]
pub struct OAITool {
    #[serde(rename = "type")]
    pub type_: String,
    pub function: OAIFunction,
}

#[derive(Serialize)]
pub struct OAIChatReq {
    pub model: String,
    pub messages: Vec<OAIMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<OAITool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
    pub stream: bool,
}

#[derive(Deserialize, Default, Debug)]
pub struct OAIToolFunctionDelta {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arguments: Option<String>,
}

#[derive(Deserialize, Default, Debug)]
pub struct OAIToolCallDelta {
    #[serde(default)]
    pub index: Option<usize>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default, rename = "type")]
    pub _type: Option<String>,
    #[serde(default)]
    pub function: Option<OAIToolFunctionDelta>,
}

#[derive(Deserialize, Default, Debug)]
pub struct OAIChoiceDelta {
    #[serde(default)]
    #[allow(dead_code)]
    pub role: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<OAIToolCallDelta>>,
    // Extended reasoning streams (optional in some backends)
    #[serde(default)]
    pub reasoning_content: Option<String>,
}

#[derive(Deserialize, Default, Debug)]
pub struct OAIChoice {
    #[serde(default)]
    #[allow(dead_code)]
    pub index: usize,
    // Streaming responses use 'delta', non-streaming use 'message'
    #[serde(default)]
    pub delta: Option<OAIChoiceDelta>,
    // Non-streaming complete response (fallback)
    #[serde(default)]
    pub message: Option<serde_json::Value>,
    #[serde(default)]
    pub finish_reason: Option<String>,
}

#[derive(Deserialize, Default, Debug)]
pub struct OAIStreamChunk {
    #[serde(default)]
    #[allow(dead_code)]
    pub id: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    pub object: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    pub created: Option<i64>,
    #[serde(default)]
    #[allow(dead_code)]
    pub model: Option<String>,
    #[serde(default)]
    pub choices: Vec<OAIChoice>,
    #[serde(default)]
    pub error: Option<serde_json::Value>,
    #[serde(default)]
    pub usage: Option<OAIUsage>,
}

#[derive(Deserialize, Default, Debug)]
pub struct OAIUsage {
    #[serde(default)]
    pub prompt_tokens: Option<u32>,
    #[serde(default)]
    pub completion_tokens: Option<u32>,
    #[serde(default)]
    #[allow(dead_code)]
    pub total_tokens: Option<u32>,
}
