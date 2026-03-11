use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct ThinkingConfig {
    #[serde(rename = "type")]
    pub type_: String, // "enabled"
    pub budget_tokens: u32,
}

#[derive(Deserialize, Debug)]
pub struct ClaudeImageSource {
    #[serde(rename = "type")]
    #[allow(dead_code)]
    pub source_type: String,
    pub media_type: String,
    pub data: String,
}

#[derive(Deserialize, Debug)]
pub struct ClaudeDocumentSource {
    #[serde(rename = "type")]
    #[allow(dead_code)]
    pub source_type: String, // "base64", "url", "file"
    #[serde(default)]
    pub media_type: Option<String>,
    #[serde(default)]
    pub data: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default, alias = "id")]
    pub file_id: Option<String>,
}

#[derive(Deserialize, Debug)]
#[serde(tag = "type")]
pub enum ClaudeContentBlock {
    #[serde(rename = "text")]
    Text {
        text: String,
        #[serde(default)]
        #[allow(dead_code)]
        cache_control: Option<Value>,
    },
    #[serde(rename = "image")]
    Image {
        source: ClaudeImageSource,
        #[serde(default)]
        #[allow(dead_code)]
        cache_control: Option<Value>,
    },
    #[serde(rename = "document")]
    Document {
        source: ClaudeDocumentSource,
        #[serde(default)]
        #[allow(dead_code)]
        cache_control: Option<Value>,
    },
    #[serde(rename = "thinking")]
    Thinking {
        thinking: String,
        #[serde(default)]
        #[allow(dead_code)]
        signature: Option<String>,
    },
    #[serde(rename = "redacted_thinking")]
    RedactedThinking {
        #[allow(dead_code)]
        data: String,
    },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: Value,
        #[serde(default)]
        #[allow(dead_code)]
        cache_control: Option<Value>,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: Value,
        #[serde(default)]
        #[allow(dead_code)]
        is_error: Option<bool>,
        #[serde(default)]
        #[allow(dead_code)]
        cache_control: Option<Value>,
    },
}

#[derive(Deserialize)]
pub struct ClaudeMessage {
    pub role: String,
    pub content: Value, // String or Vec<ClaudeContentBlock>
}

#[derive(Deserialize)]
pub struct ClaudeTool {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub input_schema: Value,
}

#[derive(Deserialize)]
pub struct ClaudeRequest {
    pub model: String,
    pub messages: Vec<ClaudeMessage>,
    #[serde(default)]
    pub system: Option<Value>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub top_p: Option<f32>,
    #[serde(default)]
    pub top_k: Option<u32>,
    #[serde(default)]
    pub stop_sequences: Option<Vec<String>>,
    #[serde(default)]
    pub tools: Option<Vec<ClaudeTool>>,
    #[serde(default)]
    pub tool_choice: Option<Value>,
    #[serde(default)]
    pub thinking: Option<ThinkingConfig>,
    #[serde(default)]
    #[allow(dead_code)]
    pub stream: Option<bool>,
    // Fields for validation warnings (accepted but not used)
    #[serde(default)]
    pub metadata: Option<Value>,
    #[serde(default)]
    pub service_tier: Option<String>,
}

#[derive(Deserialize)]
pub struct ClaudeTokenCountRequest {
    #[allow(dead_code)]
    pub model: String,
    pub messages: Vec<ClaudeMessage>,
    #[serde(default)]
    pub system: Option<Value>,
    #[serde(default)]
    pub tools: Option<Vec<ClaudeTool>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_document_block_deserializes_base64_source() {
        let block: ClaudeContentBlock = serde_json::from_value(json!({
            "type": "document",
            "source": {
                "type": "base64",
                "media_type": "application/pdf",
                "data": "ZmFrZQ=="
            }
        }))
        .unwrap();

        match block {
            ClaudeContentBlock::Document { source, .. } => {
                assert_eq!(source.source_type, "base64");
                assert_eq!(source.media_type.as_deref(), Some("application/pdf"));
                assert_eq!(source.data.as_deref(), Some("ZmFrZQ=="));
                assert_eq!(source.url, None);
                assert_eq!(source.file_id, None);
            }
            _ => panic!("expected document block"),
        }
    }

    #[test]
    fn test_document_block_deserializes_url_source() {
        let block: ClaudeContentBlock = serde_json::from_value(json!({
            "type": "document",
            "source": {
                "type": "url",
                "url": "https://example.com/file.pdf"
            }
        }))
        .unwrap();

        match block {
            ClaudeContentBlock::Document { source, .. } => {
                assert_eq!(source.source_type, "url");
                assert_eq!(source.url.as_deref(), Some("https://example.com/file.pdf"));
                assert_eq!(source.data, None);
                assert_eq!(source.file_id, None);
            }
            _ => panic!("expected document block"),
        }
    }

    #[test]
    fn test_document_block_deserializes_file_source() {
        let block: ClaudeContentBlock = serde_json::from_value(json!({
            "type": "document",
            "source": {
                "type": "file",
                "file_id": "file_123"
            }
        }))
        .unwrap();

        match block {
            ClaudeContentBlock::Document { source, .. } => {
                assert_eq!(source.source_type, "file");
                assert_eq!(source.file_id.as_deref(), Some("file_123"));
                assert_eq!(source.data, None);
                assert_eq!(source.url, None);
            }
            _ => panic!("expected document block"),
        }
    }
}
