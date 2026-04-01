use std::pin::Pin;

use async_trait::async_trait;
use futures::stream::{self, Stream, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};

use unripe_core::message::{ContentBlock, Message, Role};
use unripe_core::provider::{LlmProvider, StreamEvent, TurnConfig, TurnResponse};
use unripe_core::tool::{ToolCall, ToolDefinition};

/// Anthropic Messages API provider
pub struct AnthropicProvider {
    client: Client,
    api_key: String,
    model: String,
    base_url: String,
}

impl AnthropicProvider {
    pub fn new(api_key: String, model: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
            model,
            base_url: "https://api.anthropic.com".into(),
        }
    }

    pub fn with_base_url(mut self, url: String) -> Self {
        self.base_url = url;
        self
    }

    /// Convert our Message format to Anthropic API format
    fn to_api_messages(messages: &[Message]) -> (Option<String>, Vec<ApiMessage>) {
        let mut system_text = None;
        let mut api_messages = Vec::new();

        for msg in messages {
            match msg.role {
                Role::System => {
                    system_text = Some(msg.text_content());
                }
                Role::User => {
                    api_messages.push(ApiMessage {
                        role: "user".into(),
                        content: Self::to_api_content(&msg.content),
                    });
                }
                Role::Assistant => {
                    api_messages.push(ApiMessage {
                        role: "assistant".into(),
                        content: Self::to_api_content(&msg.content),
                    });
                }
                Role::Tool => {
                    // Tool results go as user messages in Anthropic API
                    api_messages.push(ApiMessage {
                        role: "user".into(),
                        content: Self::to_api_content(&msg.content),
                    });
                }
            }
        }

        (system_text, api_messages)
    }

    fn to_api_content(blocks: &[ContentBlock]) -> Vec<ApiContentBlock> {
        blocks
            .iter()
            .map(|block| match block {
                ContentBlock::Text { text } => ApiContentBlock::Text { text: text.clone() },
                ContentBlock::ToolUse { id, name, input } => ApiContentBlock::ToolUse {
                    id: id.clone(),
                    name: name.clone(),
                    input: input.clone(),
                },
                ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    is_error,
                } => ApiContentBlock::ToolResult {
                    tool_use_id: tool_use_id.clone(),
                    content: content.clone(),
                    is_error: *is_error,
                },
            })
            .collect()
    }

    fn to_api_tools(tools: &[ToolDefinition]) -> Vec<ApiTool> {
        tools
            .iter()
            .map(|t| ApiTool {
                name: t.name.clone(),
                description: t.description.clone(),
                input_schema: t.input_schema.clone(),
            })
            .collect()
    }

    /// Parse API response into our TurnResponse
    fn parse_response(resp: ApiResponse) -> TurnResponse {
        let mut text_parts = Vec::new();
        let mut tool_calls = Vec::new();

        for block in resp.content {
            match block {
                ApiResponseBlock::Text { text, .. } => {
                    text_parts.push(text);
                }
                ApiResponseBlock::ToolUse {
                    id, name, input, ..
                } => {
                    tool_calls.push(ToolCall { id, name, input });
                }
            }
        }

        let text = if text_parts.is_empty() {
            None
        } else {
            Some(text_parts.join(""))
        };

        match (text, tool_calls.is_empty()) {
            (Some(t), true) => TurnResponse::Text(t),
            (None, false) => TurnResponse::ToolCalls(tool_calls),
            (Some(t), false) => TurnResponse::Mixed {
                text: t,
                tool_calls,
            },
            (None, true) => TurnResponse::Text(String::new()),
        }
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    fn name(&self) -> &str {
        "anthropic"
    }

    async fn send_turn(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
        config: &TurnConfig,
    ) -> anyhow::Result<TurnResponse> {
        let (system, api_messages) = Self::to_api_messages(messages);

        let mut body = serde_json::json!({
            "model": self.model,
            "max_tokens": config.max_tokens,
            "messages": api_messages,
        });

        if let Some(sys) = system {
            body["system"] = serde_json::Value::String(sys);
        }

        if let Some(temp) = config.temperature {
            body["temperature"] = serde_json::json!(temp);
        }

        if !tools.is_empty() {
            body["tools"] = serde_json::to_value(Self::to_api_tools(tools))?;
        }

        if !config.stop_sequences.is_empty() {
            body["stop_sequences"] = serde_json::to_value(&config.stop_sequences)?;
        }

        let response = self
            .client
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            match status.as_u16() {
                401 => anyhow::bail!("Invalid API key. Check your ANTHROPIC_API_KEY."),
                429 => {
                    anyhow::bail!("Rate limited by Anthropic API. Try again shortly.\n{error_text}")
                }
                _ => anyhow::bail!("Anthropic API error (HTTP {status}): {error_text}"),
            }
        }

        let api_resp: ApiResponse = response.json().await?;
        Ok(Self::parse_response(api_resp))
    }

    async fn stream_turn(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
        config: &TurnConfig,
    ) -> anyhow::Result<Pin<Box<dyn Stream<Item = StreamEvent> + Send>>> {
        let (system, api_messages) = Self::to_api_messages(messages);

        let mut body = serde_json::json!({
            "model": self.model,
            "max_tokens": config.max_tokens,
            "messages": api_messages,
            "stream": true,
        });

        if let Some(sys) = system {
            body["system"] = serde_json::Value::String(sys);
        }

        if let Some(temp) = config.temperature {
            body["temperature"] = serde_json::json!(temp);
        }

        if !tools.is_empty() {
            body["tools"] = serde_json::to_value(Self::to_api_tools(tools))?;
        }

        let response = self
            .client
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            match status.as_u16() {
                401 => anyhow::bail!("Invalid API key. Check your ANTHROPIC_API_KEY."),
                429 => {
                    anyhow::bail!("Rate limited by Anthropic API. Try again shortly.\n{error_text}")
                }
                _ => anyhow::bail!("Anthropic API error (HTTP {status}): {error_text}"),
            }
        }

        let byte_stream = response.bytes_stream();
        let event_stream = parse_sse_stream(byte_stream);

        Ok(Box::pin(event_stream))
    }
}

/// Parse SSE byte stream into StreamEvents
fn parse_sse_stream(
    byte_stream: impl Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Send + 'static,
) -> impl Stream<Item = StreamEvent> + Send {
    let buffer = String::new();

    stream::unfold(
        (Box::pin(byte_stream), buffer),
        |(mut byte_stream, mut buffer)| async move {
            loop {
                // Check if buffer has a complete SSE event
                if let Some(pos) = buffer.find("\n\n") {
                    let event_text = buffer[..pos].to_string();
                    buffer = buffer[pos + 2..].to_string();

                    if let Some(evt) = parse_sse_event(&event_text) {
                        return Some((evt, (byte_stream, buffer)));
                    }
                    continue;
                }

                // Read more bytes
                match byte_stream.next().await {
                    Some(Ok(bytes)) => {
                        buffer.push_str(&String::from_utf8_lossy(&bytes));
                    }
                    Some(Err(e)) => {
                        return Some((
                            StreamEvent::Error(anyhow::anyhow!("Stream error: {e}")),
                            (byte_stream, buffer),
                        ));
                    }
                    None => {
                        // Stream ended
                        return if buffer.trim().is_empty() {
                            None
                        } else {
                            // Process remaining buffer
                            if let Some(evt) = parse_sse_event(&buffer) {
                                buffer.clear();
                                Some((evt, (byte_stream, buffer)))
                            } else {
                                None
                            }
                        };
                    }
                }
            }
        },
    )
}

fn parse_sse_event(event_text: &str) -> Option<StreamEvent> {
    let mut event_type = String::new();
    let mut data = String::new();

    for line in event_text.lines() {
        if let Some(rest) = line.strip_prefix("event: ") {
            event_type = rest.trim().to_string();
        } else if let Some(rest) = line.strip_prefix("data: ") {
            data = rest.trim().to_string();
        }
    }

    if data.is_empty() {
        return None;
    }

    match event_type.as_str() {
        "content_block_delta" => {
            let v: serde_json::Value = serde_json::from_str(&data).ok()?;
            let delta = v.get("delta")?;
            let delta_type = delta.get("type")?.as_str()?;

            match delta_type {
                "text_delta" => {
                    let text = delta.get("text")?.as_str()?;
                    Some(StreamEvent::TextDelta(text.to_string()))
                }
                "input_json_delta" => {
                    let json = delta.get("partial_json")?.as_str()?;
                    let index = v.get("index")?.as_u64()?;
                    Some(StreamEvent::ToolCallDelta {
                        id: format!("idx_{index}"),
                        input_json: json.to_string(),
                    })
                }
                _ => None,
            }
        }
        "content_block_start" => {
            let v: serde_json::Value = serde_json::from_str(&data).ok()?;
            let block = v.get("content_block")?;
            let block_type = block.get("type")?.as_str()?;

            if block_type == "tool_use" {
                let id = block.get("id")?.as_str()?.to_string();
                let name = block.get("name")?.as_str()?.to_string();
                Some(StreamEvent::ToolCallStart { id, name })
            } else {
                None
            }
        }
        "content_block_stop" => {
            let v: serde_json::Value = serde_json::from_str(&data).ok()?;
            let index = v.get("index")?.as_u64()?;
            Some(StreamEvent::ToolCallEnd {
                id: format!("idx_{index}"),
            })
        }
        "message_stop" => Some(StreamEvent::Done),
        "message_delta" | "message_start" | "ping" => None,
        _ => None,
    }
}

// ═══════════════════════════════════════════════════════
// Anthropic API wire types (internal)
// ═══════════════════════════════════════════════════════

#[derive(Debug, Serialize, Deserialize)]
struct ApiMessage {
    role: String,
    content: Vec<ApiContentBlock>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
enum ApiContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(default)]
        is_error: bool,
    },
}

#[derive(Debug, Serialize, Deserialize)]
struct ApiTool {
    name: String,
    description: String,
    input_schema: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct ApiResponse {
    content: Vec<ApiResponseBlock>,
    #[allow(dead_code)]
    stop_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum ApiResponseBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_api_messages_with_system() {
        let messages = vec![
            Message::text(Role::System, "You are helpful."),
            Message::text(Role::User, "Hello"),
        ];
        let (system, api_msgs) = AnthropicProvider::to_api_messages(&messages);
        assert_eq!(system, Some("You are helpful.".into()));
        assert_eq!(api_msgs.len(), 1);
        assert_eq!(api_msgs[0].role, "user");
    }

    #[test]
    fn test_to_api_messages_without_system() {
        let messages = vec![Message::text(Role::User, "Hello")];
        let (system, api_msgs) = AnthropicProvider::to_api_messages(&messages);
        assert!(system.is_none());
        assert_eq!(api_msgs.len(), 1);
    }

    #[test]
    fn test_to_api_messages_tool_result_becomes_user() {
        let messages = vec![Message::tool_result("call_1", "file contents", false)];
        let (_, api_msgs) = AnthropicProvider::to_api_messages(&messages);
        assert_eq!(api_msgs.len(), 1);
        assert_eq!(api_msgs[0].role, "user");
    }

    #[test]
    fn test_to_api_tools() {
        let tools = vec![ToolDefinition {
            name: "read_file".into(),
            description: "Read a file".into(),
            input_schema: serde_json::json!({"type": "object"}),
        }];
        let api_tools = AnthropicProvider::to_api_tools(&tools);
        assert_eq!(api_tools.len(), 1);
        assert_eq!(api_tools[0].name, "read_file");
    }

    #[test]
    fn test_parse_response_text_only() {
        let resp = ApiResponse {
            content: vec![ApiResponseBlock::Text {
                text: "Hello!".into(),
            }],
            stop_reason: Some("end_turn".into()),
        };
        match AnthropicProvider::parse_response(resp) {
            TurnResponse::Text(t) => assert_eq!(t, "Hello!"),
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn test_parse_response_tool_calls() {
        let resp = ApiResponse {
            content: vec![ApiResponseBlock::ToolUse {
                id: "call_1".into(),
                name: "bash".into(),
                input: serde_json::json!({"command": "ls"}),
            }],
            stop_reason: Some("tool_use".into()),
        };
        match AnthropicProvider::parse_response(resp) {
            TurnResponse::ToolCalls(calls) => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].name, "bash");
            }
            _ => panic!("expected ToolCalls"),
        }
    }

    #[test]
    fn test_parse_response_mixed() {
        let resp = ApiResponse {
            content: vec![
                ApiResponseBlock::Text {
                    text: "I'll run this:".into(),
                },
                ApiResponseBlock::ToolUse {
                    id: "call_2".into(),
                    name: "bash".into(),
                    input: serde_json::json!({"command": "cargo test"}),
                },
            ],
            stop_reason: Some("tool_use".into()),
        };
        match AnthropicProvider::parse_response(resp) {
            TurnResponse::Mixed { text, tool_calls } => {
                assert_eq!(text, "I'll run this:");
                assert_eq!(tool_calls.len(), 1);
            }
            _ => panic!("expected Mixed"),
        }
    }

    #[test]
    fn test_parse_sse_text_delta() {
        let event = r#"event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}"#;
        let result = parse_sse_event(event);
        match result {
            Some(StreamEvent::TextDelta(t)) => assert_eq!(t, "Hello"),
            _ => panic!("expected TextDelta, got {result:?}"),
        }
    }

    #[test]
    fn test_parse_sse_tool_call_start() {
        let event = r#"event: content_block_start
data: {"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_01","name":"read_file","input":{}}}"#;
        let result = parse_sse_event(event);
        match result {
            Some(StreamEvent::ToolCallStart { id, name }) => {
                assert_eq!(id, "toolu_01");
                assert_eq!(name, "read_file");
            }
            _ => panic!("expected ToolCallStart, got {result:?}"),
        }
    }

    #[test]
    fn test_parse_sse_tool_call_delta() {
        let event = r#"event: content_block_delta
data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\"main.rs\"}"}}"#;
        let result = parse_sse_event(event);
        match result {
            Some(StreamEvent::ToolCallDelta { input_json, .. }) => {
                assert!(input_json.contains("main.rs"));
            }
            _ => panic!("expected ToolCallDelta, got {result:?}"),
        }
    }

    #[test]
    fn test_parse_sse_message_stop() {
        let event = "event: message_stop\ndata: {}";
        let result = parse_sse_event(event);
        assert!(matches!(result, Some(StreamEvent::Done)));
    }

    #[test]
    fn test_parse_sse_ping_ignored() {
        let event = "event: ping\ndata: {}";
        let result = parse_sse_event(event);
        assert!(result.is_none());
    }

    #[test]
    fn test_api_content_block_serialization() {
        let blocks = vec![
            ApiContentBlock::Text {
                text: "hello".into(),
            },
            ApiContentBlock::ToolUse {
                id: "call_1".into(),
                name: "bash".into(),
                input: serde_json::json!({"command": "ls"}),
            },
        ];
        let json = serde_json::to_string(&blocks).unwrap();
        assert!(json.contains("\"type\":\"text\""));
        assert!(json.contains("\"type\":\"tool_use\""));
    }

    #[test]
    fn test_api_message_with_tool_result() {
        let msg = ApiMessage {
            role: "user".into(),
            content: vec![ApiContentBlock::ToolResult {
                tool_use_id: "call_1".into(),
                content: "file contents".into(),
                is_error: false,
            }],
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("tool_result"));
        assert!(json.contains("call_1"));
    }
}
