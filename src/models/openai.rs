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
pub struct StreamOptions {
    pub include_usage: bool,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_options: Option<StreamOptions>,
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
pub struct PromptTokensDetails {
    #[serde(default)]
    pub cached_tokens: Option<u32>,
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
    #[serde(default)]
    pub prompt_tokens_details: Option<PromptTokensDetails>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_serialize_stream_options() {
        // When Some, stream_options is serialized.
        let req = OAIChatReq {
            model: "m".into(),
            messages: vec![],
            max_tokens: None,
            temperature: None,
            top_p: None,
            top_k: None,
            stop: None,
            tools: None,
            tool_choice: None,
            thinking: None,
            reasoning_effort: None,
            parallel_tool_calls: None,
            metadata: None,
            stream: true,
            stream_options: Some(StreamOptions {
                include_usage: true,
            }),
        };
        let v = serde_json::to_value(&req).unwrap();
        assert_eq!(v["stream_options"]["include_usage"], json!(true));

        // When None, stream_options is omitted.
        let req_none = OAIChatReq {
            model: "m".into(),
            messages: vec![],
            max_tokens: None,
            temperature: None,
            top_p: None,
            top_k: None,
            stop: None,
            tools: None,
            tool_choice: None,
            thinking: None,
            reasoning_effort: None,
            parallel_tool_calls: None,
            metadata: None,
            stream: false,
            stream_options: None,
        };
        let v_none = serde_json::to_value(&req_none).unwrap();
        assert!(v_none.get("stream_options").is_none());
    }

    #[test]
    fn test_deserialize_usage_prompt_tokens_details() {
        // Cache-hit: prompt_tokens_details present with cached_tokens.
        let usage: OAIUsage = serde_json::from_value(json!({
            "prompt_tokens": 541,
            "completion_tokens": 12,
            "total_tokens": 553,
            "prompt_tokens_details": {"cached_tokens": 512}
        }))
        .unwrap();
        assert_eq!(
            usage.prompt_tokens_details.as_ref().unwrap().cached_tokens,
            Some(512)
        );

        // Cold call: prompt_tokens_details is null -> None.
        let usage_cold: OAIUsage = serde_json::from_value(json!({
            "prompt_tokens": 120,
            "completion_tokens": 37,
            "total_tokens": 157,
            "prompt_tokens_details": null
        }))
        .unwrap();
        assert!(usage_cold.prompt_tokens_details.is_none());
    }

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
