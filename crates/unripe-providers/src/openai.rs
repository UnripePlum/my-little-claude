use std::pin::Pin;

use async_trait::async_trait;
use futures::stream::{self, Stream, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};

use unripe_core::message::{ContentBlock, Message, Role};
use unripe_core::provider::{LlmProvider, StreamEvent, TurnConfig, TurnResponse};
use unripe_core::tool::{ToolCall, ToolDefinition};

/// OpenAI Chat Completions API provider
pub struct OpenAiProvider {
    client: Client,
    api_key: String,
    model: String,
    base_url: String,
}

impl OpenAiProvider {
    pub fn new(api_key: String, model: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
            model,
            base_url: "https://api.openai.com".into(),
        }
    }

    pub fn with_base_url(mut self, url: String) -> Self {
        self.base_url = url;
        self
    }

    fn to_api_messages(messages: &[Message]) -> Vec<OaiMessage> {
        messages
            .iter()
            .map(|msg| {
                let role = match msg.role {
                    Role::System => "system",
                    Role::User => "user",
                    Role::Assistant => "assistant",
                    Role::Tool => "tool",
                };

                let mut content = String::new();
                let mut tool_calls = Vec::new();
                let mut tool_call_id = None;

                for block in &msg.content {
                    match block {
                        ContentBlock::Text { text } => content.push_str(text),
                        ContentBlock::ToolUse { id, name, input } => {
                            tool_calls.push(OaiToolCall {
                                id: id.clone(),
                                r#type: "function".into(),
                                function: OaiFunctionCall {
                                    name: name.clone(),
                                    arguments: serde_json::to_string(input).unwrap_or_default(),
                                },
                            });
                        }
                        ContentBlock::ToolResult {
                            tool_use_id,
                            content: result_content,
                            ..
                        } => {
                            content.push_str(result_content);
                            tool_call_id = Some(tool_use_id.clone());
                        }
                    }
                }

                OaiMessage {
                    role: role.into(),
                    content: if content.is_empty() {
                        None
                    } else {
                        Some(content)
                    },
                    tool_calls: if tool_calls.is_empty() {
                        None
                    } else {
                        Some(tool_calls)
                    },
                    tool_call_id,
                }
            })
            .collect()
    }

    fn to_api_tools(tools: &[ToolDefinition]) -> Vec<OaiTool> {
        tools
            .iter()
            .map(|t| OaiTool {
                r#type: "function".into(),
                function: OaiFunction {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    parameters: t.input_schema.clone(),
                },
            })
            .collect()
    }

    fn parse_response(resp: OaiResponse) -> TurnResponse {
        let choice = match resp.choices.into_iter().next() {
            Some(c) => c,
            None => return TurnResponse::Text(String::new()),
        };

        let text = choice.message.content.unwrap_or_default();
        let tool_calls: Vec<ToolCall> = choice
            .message
            .tool_calls
            .unwrap_or_default()
            .into_iter()
            .map(|tc| ToolCall {
                id: tc.id,
                name: tc.function.name,
                input: serde_json::from_str(&tc.function.arguments)
                    .unwrap_or(serde_json::Value::Object(serde_json::Map::new())),
            })
            .collect();

        let has_text = !text.is_empty();
        let has_tools = !tool_calls.is_empty();

        match (has_text, has_tools) {
            (true, true) => TurnResponse::Mixed { text, tool_calls },
            (false, true) => TurnResponse::ToolCalls(tool_calls),
            (true, false) => TurnResponse::Text(text),
            (false, false) => TurnResponse::Text(String::new()),
        }
    }
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    fn name(&self) -> &str {
        "openai"
    }

    async fn send_turn(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
        config: &TurnConfig,
    ) -> anyhow::Result<TurnResponse> {
        let api_messages = Self::to_api_messages(messages);

        let mut body = serde_json::json!({
            "model": self.model,
            "messages": api_messages,
            "max_completion_tokens": config.max_tokens,
        });

        if let Some(temp) = config.temperature {
            body["temperature"] = serde_json::json!(temp);
        }

        if !tools.is_empty() {
            body["tools"] = serde_json::to_value(Self::to_api_tools(tools))?;
        }

        if !config.stop_sequences.is_empty() {
            body["stop"] = serde_json::to_value(&config.stop_sequences)?;
        }

        let response = self
            .client
            .post(format!("{}/v1/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            match status.as_u16() {
                401 => anyhow::bail!("Invalid OpenAI API key. Check your OPENAI_API_KEY."),
                429 => {
                    anyhow::bail!("Rate limited by OpenAI API. Try again shortly.\n{error_text}")
                }
                _ => anyhow::bail!("OpenAI API error (HTTP {status}): {error_text}"),
            }
        }

        let api_resp: OaiResponse = response.json().await?;
        Ok(Self::parse_response(api_resp))
    }

    async fn stream_turn(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
        config: &TurnConfig,
    ) -> anyhow::Result<Pin<Box<dyn Stream<Item = StreamEvent> + Send>>> {
        let api_messages = Self::to_api_messages(messages);

        let mut body = serde_json::json!({
            "model": self.model,
            "messages": api_messages,
            "max_completion_tokens": config.max_tokens,
            "stream": true,
        });

        if let Some(temp) = config.temperature {
            body["temperature"] = serde_json::json!(temp);
        }

        if !tools.is_empty() {
            body["tools"] = serde_json::to_value(Self::to_api_tools(tools))?;
        }

        let response = self
            .client
            .post(format!("{}/v1/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            match status.as_u16() {
                401 => anyhow::bail!("Invalid OpenAI API key. Check your OPENAI_API_KEY."),
                429 => {
                    anyhow::bail!("Rate limited by OpenAI API. Try again shortly.\n{error_text}")
                }
                _ => anyhow::bail!("OpenAI API error (HTTP {status}): {error_text}"),
            }
        }

        let byte_stream = response.bytes_stream();

        let event_stream = stream::unfold(
            (
                Box::pin(byte_stream)
                    as Pin<Box<dyn Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Send>>,
                String::new(),
            ),
            |(mut byte_stream, mut buffer)| async move {
                loop {
                    // Look for SSE data lines
                    if let Some(pos) = buffer.find("\n\n") {
                        let event_text = buffer[..pos].to_string();
                        buffer = buffer[pos + 2..].to_string();

                        if let Some(evt) = parse_sse_event(&event_text) {
                            return Some((evt, (byte_stream, buffer)));
                        }
                        continue;
                    }

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
                        None => return None,
                    }
                }
            },
        );

        Ok(Box::pin(event_stream))
    }
}

fn parse_sse_event(event_text: &str) -> Option<StreamEvent> {
    for line in event_text.lines() {
        if let Some(data) = line.strip_prefix("data: ") {
            let data = data.trim();
            if data == "[DONE]" {
                return Some(StreamEvent::Done);
            }

            let v: serde_json::Value = serde_json::from_str(data).ok()?;
            let delta = v.get("choices")?.as_array()?.first()?.get("delta")?;

            // Text content
            if let Some(content) = delta.get("content").and_then(|c| c.as_str()) {
                if !content.is_empty() {
                    return Some(StreamEvent::TextDelta(content.to_string()));
                }
            }

            // Tool calls
            if let Some(tool_calls) = delta.get("tool_calls").and_then(|tc| tc.as_array()) {
                if let Some(tc) = tool_calls.first() {
                    let index = tc.get("index").and_then(|i| i.as_u64()).unwrap_or(0);

                    if let Some(function) = tc.get("function") {
                        if let Some(name) = function.get("name").and_then(|n| n.as_str()) {
                            let id = tc
                                .get("id")
                                .and_then(|i| i.as_str())
                                .unwrap_or("")
                                .to_string();
                            return Some(StreamEvent::ToolCallStart {
                                id,
                                name: name.to_string(),
                            });
                        }
                        if let Some(args) = function.get("arguments").and_then(|a| a.as_str()) {
                            if !args.is_empty() {
                                return Some(StreamEvent::ToolCallDelta {
                                    id: format!("idx_{index}"),
                                    input_json: args.to_string(),
                                });
                            }
                        }
                    }
                }
            }

            // Finish reason
            if let Some(finish) = v
                .get("choices")
                .and_then(|c| c.as_array())
                .and_then(|c| c.first())
                .and_then(|c| c.get("finish_reason"))
                .and_then(|f| f.as_str())
            {
                if finish == "stop" || finish == "tool_calls" {
                    return Some(StreamEvent::Done);
                }
            }
        }
    }
    None
}

// ═══════════════════════════════════════════════════════
// OpenAI API wire types
// ═══════════════════════════════════════════════════════

#[derive(Debug, Serialize, Deserialize)]
struct OaiMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OaiToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OaiToolCall {
    id: String,
    r#type: String,
    function: OaiFunctionCall,
}

#[derive(Debug, Serialize, Deserialize)]
struct OaiFunctionCall {
    name: String,
    arguments: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct OaiTool {
    r#type: String,
    function: OaiFunction,
}

#[derive(Debug, Serialize, Deserialize)]
struct OaiFunction {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct OaiResponse {
    choices: Vec<OaiChoice>,
}

#[derive(Debug, Deserialize)]
struct OaiChoice {
    message: OaiResponseMessage,
}

#[derive(Debug, Deserialize)]
struct OaiResponseMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<OaiToolCall>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_api_messages_basic() {
        let messages = vec![
            Message::text(Role::System, "Be helpful"),
            Message::text(Role::User, "Hello"),
            Message::text(Role::Assistant, "Hi there"),
        ];
        let api = OpenAiProvider::to_api_messages(&messages);
        assert_eq!(api.len(), 3);
        assert_eq!(api[0].role, "system");
        assert_eq!(api[0].content.as_deref(), Some("Be helpful"));
        assert_eq!(api[1].role, "user");
        assert_eq!(api[2].role, "assistant");
    }

    #[test]
    fn test_to_api_messages_tool_result() {
        let messages = vec![Message::tool_result("call_1", "file content", false)];
        let api = OpenAiProvider::to_api_messages(&messages);
        assert_eq!(api[0].role, "tool");
        assert_eq!(api[0].content.as_deref(), Some("file content"));
        assert_eq!(api[0].tool_call_id, Some("call_1".into()));
    }

    #[test]
    fn test_to_api_messages_assistant_with_tool_use() {
        let msg = Message {
            role: Role::Assistant,
            content: vec![ContentBlock::ToolUse {
                id: "call_1".into(),
                name: "bash".into(),
                input: serde_json::json!({"command": "ls"}),
            }],
        };
        let api = OpenAiProvider::to_api_messages(&[msg]);
        assert_eq!(api[0].role, "assistant");
        assert!(api[0].content.is_none()); // no text content
        let tool_calls = api[0].tool_calls.as_ref().unwrap();
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].function.name, "bash");
        assert_eq!(tool_calls[0].function.arguments, r#"{"command":"ls"}"#);
    }

    #[test]
    fn test_to_api_tools() {
        let tools = vec![ToolDefinition {
            name: "read_file".into(),
            description: "Read a file".into(),
            input_schema: serde_json::json!({"type": "object"}),
        }];
        let api = OpenAiProvider::to_api_tools(&tools);
        assert_eq!(api.len(), 1);
        assert_eq!(api[0].r#type, "function");
        assert_eq!(api[0].function.name, "read_file");
    }

    #[test]
    fn test_parse_response_text() {
        let resp = OaiResponse {
            choices: vec![OaiChoice {
                message: OaiResponseMessage {
                    content: Some("Hello!".into()),
                    tool_calls: None,
                },
            }],
        };
        match OpenAiProvider::parse_response(resp) {
            TurnResponse::Text(t) => assert_eq!(t, "Hello!"),
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn test_parse_response_tool_calls() {
        let resp = OaiResponse {
            choices: vec![OaiChoice {
                message: OaiResponseMessage {
                    content: None,
                    tool_calls: Some(vec![OaiToolCall {
                        id: "call_1".into(),
                        r#type: "function".into(),
                        function: OaiFunctionCall {
                            name: "bash".into(),
                            arguments: r#"{"command":"ls"}"#.into(),
                        },
                    }]),
                },
            }],
        };
        match OpenAiProvider::parse_response(resp) {
            TurnResponse::ToolCalls(calls) => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].name, "bash");
                assert_eq!(calls[0].input["command"], "ls");
            }
            _ => panic!("expected ToolCalls"),
        }
    }

    #[test]
    fn test_parse_response_mixed() {
        let resp = OaiResponse {
            choices: vec![OaiChoice {
                message: OaiResponseMessage {
                    content: Some("Let me check".into()),
                    tool_calls: Some(vec![OaiToolCall {
                        id: "call_2".into(),
                        r#type: "function".into(),
                        function: OaiFunctionCall {
                            name: "read_file".into(),
                            arguments: r#"{"path":"main.rs"}"#.into(),
                        },
                    }]),
                },
            }],
        };
        match OpenAiProvider::parse_response(resp) {
            TurnResponse::Mixed { text, tool_calls } => {
                assert_eq!(text, "Let me check");
                assert_eq!(tool_calls.len(), 1);
            }
            _ => panic!("expected Mixed"),
        }
    }

    #[test]
    fn test_parse_response_empty_choices() {
        let resp = OaiResponse { choices: vec![] };
        match OpenAiProvider::parse_response(resp) {
            TurnResponse::Text(t) => assert!(t.is_empty()),
            _ => panic!("expected empty Text"),
        }
    }

    #[test]
    fn test_parse_sse_text_delta() {
        let event = r#"data: {"choices":[{"delta":{"content":"Hello"},"index":0}]}"#;
        match parse_sse_event(event) {
            Some(StreamEvent::TextDelta(t)) => assert_eq!(t, "Hello"),
            other => panic!("expected TextDelta, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_sse_done() {
        let event = "data: [DONE]";
        assert!(matches!(parse_sse_event(event), Some(StreamEvent::Done)));
    }

    #[test]
    fn test_parse_sse_tool_call_start() {
        let event = r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"bash"}}]},"index":0}]}"#;
        match parse_sse_event(event) {
            Some(StreamEvent::ToolCallStart { id, name }) => {
                assert_eq!(id, "call_1");
                assert_eq!(name, "bash");
            }
            other => panic!("expected ToolCallStart, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_sse_tool_call_delta() {
        let event = r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"cmd\":\"ls\"}"}}]},"index":0}]}"#;
        match parse_sse_event(event) {
            Some(StreamEvent::ToolCallDelta { input_json, .. }) => {
                assert!(input_json.contains("cmd"));
            }
            other => panic!("expected ToolCallDelta, got {other:?}"),
        }
    }

    #[test]
    fn test_oai_message_serialization_skips_none() {
        let msg = OaiMessage {
            role: "assistant".into(),
            content: Some("hello".into()),
            tool_calls: None,
            tool_call_id: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(!json.contains("tool_calls"));
        assert!(!json.contains("tool_call_id"));
        assert!(json.contains("hello"));
    }
}
