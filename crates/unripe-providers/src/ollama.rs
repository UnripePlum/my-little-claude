use std::pin::Pin;

use async_trait::async_trait;
use futures::stream::{self, Stream, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};

use unripe_core::message::{ContentBlock, Message, Role};
use unripe_core::provider::{LlmProvider, StreamEvent, TurnConfig, TurnResponse};
use unripe_core::tool::{ToolCall, ToolDefinition};

/// Ollama provider using the OpenAI-compatible /api/chat endpoint
pub struct OllamaProvider {
    client: Client,
    model: String,
    base_url: String,
}

impl OllamaProvider {
    pub fn new(model: String, base_url: String) -> Self {
        Self {
            client: Client::new(),
            model,
            base_url,
        }
    }

    fn to_api_messages(messages: &[Message]) -> Vec<OllamaMessage> {
        messages
            .iter()
            .map(|msg| {
                let role = match msg.role {
                    Role::System => "system",
                    Role::User => "user",
                    Role::Assistant => "assistant",
                    Role::Tool => "tool",
                };

                let mut text_parts = Vec::new();
                let mut tool_calls = Vec::new();
                let mut tool_call_id = None;

                for block in &msg.content {
                    match block {
                        ContentBlock::Text { text } => text_parts.push(text.clone()),
                        ContentBlock::ToolUse { id, name, input } => {
                            tool_calls.push(OllamaToolCall {
                                id: Some(id.clone()),
                                r#type: "function".into(),
                                function: OllamaFunctionCall {
                                    name: name.clone(),
                                    arguments: input.clone(),
                                },
                            });
                        }
                        ContentBlock::ToolResult {
                            tool_use_id,
                            content,
                            ..
                        } => {
                            text_parts.push(content.clone());
                            tool_call_id = Some(tool_use_id.clone());
                        }
                    }
                }

                let content = text_parts.join("");

                OllamaMessage {
                    role: role.into(),
                    content,
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

    fn to_api_tools(tools: &[ToolDefinition]) -> Vec<OllamaTool> {
        tools
            .iter()
            .map(|t| OllamaTool {
                r#type: "function".into(),
                function: OllamaFunction {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    parameters: t.input_schema.clone(),
                },
            })
            .collect()
    }

    fn parse_response(resp: OllamaResponse) -> TurnResponse {
        let text = resp.message.content.clone();
        let tool_calls: Vec<ToolCall> = resp
            .message
            .tool_calls
            .unwrap_or_default()
            .into_iter()
            .filter_map(|tc| {
                let func = tc.function?;
                if func.name.is_empty() {
                    return None;
                }
                Some(ToolCall {
                    id: uuid::Uuid::new_v4().to_string(),
                    name: func.name,
                    input: func.arguments,
                })
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
impl LlmProvider for OllamaProvider {
    fn name(&self) -> &str {
        "ollama"
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
            "stream": false,
            "options": {
                "num_predict": config.max_tokens,
            }
        });

        if let Some(temp) = config.temperature {
            body["options"]["temperature"] = serde_json::json!(temp);
        }

        if !tools.is_empty() {
            body["tools"] = serde_json::to_value(Self::to_api_tools(tools))?;
        }

        let response = self
            .client
            .post(format!("{}/api/chat", self.base_url))
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                if e.is_connect() {
                    anyhow::anyhow!(
                        "Cannot connect to ollama at {}. Is ollama running? Start with: ollama serve",
                        self.base_url
                    )
                } else {
                    anyhow::anyhow!("Ollama request failed: {e}")
                }
            })?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            if error_text.contains("model") && error_text.contains("not found") {
                anyhow::bail!(
                    "Model '{}' not found. Pull it with: ollama pull {}",
                    self.model,
                    self.model
                );
            }
            anyhow::bail!("Ollama API error (HTTP {status}): {error_text}");
        }

        let api_resp: OllamaResponse = response.json().await?;
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
            "stream": true,
            "options": {
                "num_predict": config.max_tokens,
            }
        });

        if let Some(temp) = config.temperature {
            body["options"]["temperature"] = serde_json::json!(temp);
        }

        if !tools.is_empty() {
            body["tools"] = serde_json::to_value(Self::to_api_tools(tools))?;
        }

        let response = self
            .client
            .post(format!("{}/api/chat", self.base_url))
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                if e.is_connect() {
                    anyhow::anyhow!(
                        "Cannot connect to ollama at {}. Is ollama running? Start with: ollama serve",
                        self.base_url
                    )
                } else {
                    anyhow::anyhow!("Ollama request failed: {e}")
                }
            })?;

        let byte_stream = response.bytes_stream();

        let event_stream = stream::unfold(
            (
                Box::pin(byte_stream)
                    as Pin<Box<dyn Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Send>>,
                String::new(),
            ),
            |(mut byte_stream, mut buffer)| async move {
                loop {
                    if let Some(pos) = buffer.find('\n') {
                        let line = buffer[..pos].to_string();
                        buffer = buffer[pos + 1..].to_string();

                        if line.trim().is_empty() {
                            continue;
                        }

                        if let Ok(chunk) = serde_json::from_str::<OllamaStreamChunk>(&line) {
                            if chunk.done {
                                return Some((StreamEvent::Done, (byte_stream, buffer)));
                            }
                            if !chunk.message.content.is_empty() {
                                return Some((
                                    StreamEvent::TextDelta(chunk.message.content),
                                    (byte_stream, buffer),
                                ));
                            }
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

// ═══════════════════════════════════════════════════════
// Ollama API wire types (OpenAI-compatible)
// ═══════════════════════════════════════════════════════

#[derive(Debug, Serialize, Deserialize)]
struct OllamaMessage {
    role: String,
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OllamaToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OllamaToolCall {
    #[serde(default)]
    id: Option<String>,
    r#type: String,
    function: OllamaFunctionCall,
}

#[derive(Debug, Serialize, Deserialize)]
struct OllamaFunctionCall {
    name: String,
    arguments: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize)]
struct OllamaTool {
    r#type: String,
    function: OllamaFunction,
}

#[derive(Debug, Serialize, Deserialize)]
struct OllamaFunction {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct OllamaResponse {
    message: OllamaResponseMessage,
    #[allow(dead_code)]
    #[serde(default)]
    done: bool,
    // ollama returns many extra fields (done_reason, total_duration, etc.)
    // serde ignores them by default
}

#[derive(Debug, Deserialize)]
struct OllamaResponseMessage {
    #[serde(default)]
    content: String,
    #[serde(default)]
    tool_calls: Option<Vec<OllamaResponseToolCall>>,
}

/// Tool call format in ollama responses (different from request format)
#[derive(Debug, Deserialize)]
struct OllamaResponseToolCall {
    #[serde(default)]
    function: Option<OllamaResponseFunctionCall>,
}

#[derive(Debug, Deserialize)]
struct OllamaResponseFunctionCall {
    #[serde(default)]
    name: String,
    #[serde(default)]
    arguments: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct OllamaStreamChunk {
    message: OllamaStreamMessage,
    done: bool,
}

#[derive(Debug, Deserialize)]
struct OllamaStreamMessage {
    #[serde(default)]
    content: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_api_messages_basic() {
        let messages = vec![
            Message::text(Role::System, "Be helpful"),
            Message::text(Role::User, "Hello"),
        ];
        let api = OllamaProvider::to_api_messages(&messages);
        assert_eq!(api.len(), 2);
        assert_eq!(api[0].role, "system");
        assert_eq!(api[0].content, "Be helpful");
        assert_eq!(api[1].role, "user");
    }

    #[test]
    fn test_to_api_messages_tool_result() {
        let messages = vec![Message::tool_result("call_1", "file content", false)];
        let api = OllamaProvider::to_api_messages(&messages);
        assert_eq!(api[0].role, "tool");
        assert_eq!(api[0].content, "file content");
        assert_eq!(api[0].tool_call_id, Some("call_1".into()));
    }

    #[test]
    fn test_to_api_tools() {
        let tools = vec![ToolDefinition {
            name: "bash".into(),
            description: "Run commands".into(),
            input_schema: serde_json::json!({"type": "object"}),
        }];
        let api = OllamaProvider::to_api_tools(&tools);
        assert_eq!(api.len(), 1);
        assert_eq!(api[0].function.name, "bash");
        assert_eq!(api[0].r#type, "function");
    }

    #[test]
    fn test_parse_response_text() {
        let resp = OllamaResponse {
            message: OllamaResponseMessage {
                content: "Hello!".into(),
                tool_calls: None,
            },
            done: true, // extra fields like done_reason are ignored by serde
        };
        match OllamaProvider::parse_response(resp) {
            TurnResponse::Text(t) => assert_eq!(t, "Hello!"),
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn test_parse_response_tool_calls() {
        let resp = OllamaResponse {
            message: OllamaResponseMessage {
                content: String::new(),
                tool_calls: Some(vec![OllamaResponseToolCall {
                    function: Some(OllamaResponseFunctionCall {
                        name: "bash".into(),
                        arguments: serde_json::json!({"command": "ls"}),
                    }),
                }]),
            },
            done: true,
        };
        match OllamaProvider::parse_response(resp) {
            TurnResponse::ToolCalls(calls) => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].name, "bash");
                assert_eq!(calls[0].input["command"], "ls");
            }
            _ => panic!("expected ToolCalls"),
        }
    }

    #[test]
    fn test_parse_response_tool_call_auto_id() {
        let resp = OllamaResponse {
            message: OllamaResponseMessage {
                content: String::new(),
                tool_calls: Some(vec![OllamaResponseToolCall {
                    function: Some(OllamaResponseFunctionCall {
                        name: "read_file".into(),
                        arguments: serde_json::json!({"path": "main.rs"}),
                    }),
                }]),
            },
            done: true,
        };
        match OllamaProvider::parse_response(resp) {
            TurnResponse::ToolCalls(calls) => {
                assert!(!calls[0].id.is_empty()); // auto-generated UUID
                assert_eq!(calls[0].name, "read_file");
            }
            _ => panic!("expected ToolCalls"),
        }
    }

    #[test]
    fn test_ollama_message_serialization() {
        let msg = OllamaMessage {
            role: "assistant".into(),
            content: "text".into(),
            tool_calls: None,
            tool_call_id: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(!json.contains("tool_calls")); // skip_serializing_if None
        assert!(!json.contains("tool_call_id"));
    }

    #[test]
    fn test_stream_chunk_parsing() {
        let json = r#"{"message":{"role":"assistant","content":"Hi"},"done":false}"#;
        let chunk: OllamaStreamChunk = serde_json::from_str(json).unwrap();
        assert_eq!(chunk.message.content, "Hi");
        assert!(!chunk.done);
    }

    #[test]
    fn test_stream_chunk_done() {
        let json = r#"{"message":{"role":"assistant","content":""},"done":true}"#;
        let chunk: OllamaStreamChunk = serde_json::from_str(json).unwrap();
        assert!(chunk.done);
    }
}
