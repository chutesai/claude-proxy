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
    pub reasoning_effort: Option<String>,
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_deserialize_stream_chunk_with_reasoning_and_tool_calls() {
        let chunk: OAIStreamChunk = serde_json::from_value(json!({
            "id": "chatcmpl-1",
            "object": "chat.completion.chunk",
            "created": 123,
            "model": "deepseek-r1",
            "choices": [
                {
                    "index": 0,
                    "delta": {
                        "role": "assistant",
                        "reasoning_content": "thinking...",
                        "tool_calls": [
                            {
                                "index": 0,
                                "id": "call_1",
                                "type": "function",
                                "function": {
                                    "name": "read_file",
                                    "arguments": "{\"path\":\"README.md\"}"
                                }
                            }
                        ]
                    },
                    "finish_reason": null
                }
            ],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 5,
                "total_tokens": 15
            }
        }))
        .unwrap();

        assert_eq!(chunk.model.as_deref(), Some("deepseek-r1"));
        assert_eq!(chunk.usage.as_ref().unwrap().total_tokens, Some(15));
        let delta = chunk.choices[0].delta.as_ref().unwrap();
        assert_eq!(delta.reasoning_content.as_deref(), Some("thinking..."));
        assert_eq!(
            delta.tool_calls.as_ref().unwrap()[0].id.as_deref(),
            Some("call_1")
        );
    }

    #[test]
    fn test_deserialize_non_streaming_chunk_message() {
        let chunk: OAIStreamChunk = serde_json::from_value(json!({
            "id": "chatcmpl-2",
            "object": "chat.completion",
            "model": "zai-org/GLM-4.5-Air",
            "choices": [
                {
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": "hello"
                    },
                    "finish_reason": "stop"
                }
            ],
            "usage": {
                "prompt_tokens": 2,
                "completion_tokens": 1
            }
        }))
        .unwrap();

        assert_eq!(chunk.choices[0].finish_reason.as_deref(), Some("stop"));
        assert_eq!(
            chunk.choices[0].message.as_ref().unwrap()["content"],
            json!("hello")
        );
        assert_eq!(chunk.usage.as_ref().unwrap().prompt_tokens, Some(2));
    }
}
