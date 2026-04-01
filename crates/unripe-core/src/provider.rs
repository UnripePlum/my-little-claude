use std::pin::Pin;

use async_trait::async_trait;
use futures::Stream;
use serde::{Deserialize, Serialize};

use crate::message::Message;
use crate::tool::{ToolCall, ToolDefinition};

/// Configuration for a single LLM turn
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnConfig {
    pub max_tokens: u32,
    pub temperature: Option<f64>,
    pub stream: bool,
    #[serde(default)]
    pub stop_sequences: Vec<String>,
}

impl Default for TurnConfig {
    fn default() -> Self {
        Self {
            max_tokens: 4096,
            temperature: None,
            stream: true,
            stop_sequences: Vec::new(),
        }
    }
}

/// Response from a non-streaming LLM turn
#[derive(Debug, Clone)]
pub enum TurnResponse {
    Text(String),
    ToolCalls(Vec<ToolCall>),
    Mixed {
        text: String,
        tool_calls: Vec<ToolCall>,
    },
}

impl TurnResponse {
    /// Check if the response contains tool calls
    pub fn has_tool_calls(&self) -> bool {
        matches!(
            self,
            TurnResponse::ToolCalls(_) | TurnResponse::Mixed { .. }
        )
    }

    /// Extract tool calls from the response
    pub fn tool_calls(&self) -> &[ToolCall] {
        match self {
            TurnResponse::ToolCalls(calls) => calls,
            TurnResponse::Mixed { tool_calls, .. } => tool_calls,
            TurnResponse::Text(_) => &[],
        }
    }

    /// Extract text from the response
    pub fn text(&self) -> Option<&str> {
        match self {
            TurnResponse::Text(t) => Some(t),
            TurnResponse::Mixed { text, .. } => Some(text),
            TurnResponse::ToolCalls(_) => None,
        }
    }
}

/// Events emitted during streaming
#[derive(Debug)]
pub enum StreamEvent {
    TextDelta(String),
    ToolCallStart { id: String, name: String },
    ToolCallDelta { id: String, input_json: String },
    ToolCallEnd { id: String },
    Done,
    Error(anyhow::Error),
}

/// Trait for LLM providers (Anthropic, ollama, etc.)
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Provider name for display/config
    fn name(&self) -> &str;

    /// Non-streaming: wait for full response
    async fn send_turn(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
        config: &TurnConfig,
    ) -> anyhow::Result<TurnResponse>;

    /// Streaming: yield events as they arrive
    async fn stream_turn(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
        config: &TurnConfig,
    ) -> anyhow::Result<Pin<Box<dyn Stream<Item = StreamEvent> + Send>>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_turn_config_default() {
        let config = TurnConfig::default();
        assert_eq!(config.max_tokens, 4096);
        assert!(config.temperature.is_none());
        assert!(config.stream);
        assert!(config.stop_sequences.is_empty());
    }

    #[test]
    fn test_turn_response_text() {
        let resp = TurnResponse::Text("Hello".into());
        assert!(!resp.has_tool_calls());
        assert_eq!(resp.text(), Some("Hello"));
        assert!(resp.tool_calls().is_empty());
    }

    #[test]
    fn test_turn_response_tool_calls() {
        let resp = TurnResponse::ToolCalls(vec![ToolCall {
            id: "call_1".into(),
            name: "bash".into(),
            input: serde_json::json!({"command": "ls"}),
        }]);
        assert!(resp.has_tool_calls());
        assert!(resp.text().is_none());
        assert_eq!(resp.tool_calls().len(), 1);
        assert_eq!(resp.tool_calls()[0].name, "bash");
    }

    #[test]
    fn test_turn_response_mixed() {
        let resp = TurnResponse::Mixed {
            text: "Let me check".into(),
            tool_calls: vec![ToolCall {
                id: "call_2".into(),
                name: "read_file".into(),
                input: serde_json::json!({"path": "src/main.rs"}),
            }],
        };
        assert!(resp.has_tool_calls());
        assert_eq!(resp.text(), Some("Let me check"));
        assert_eq!(resp.tool_calls().len(), 1);
    }

    #[test]
    fn test_turn_config_serialization() {
        let config = TurnConfig {
            max_tokens: 2048,
            temperature: Some(0.7),
            stream: false,
            stop_sequences: vec!["STOP".into()],
        };
        let json = serde_json::to_string(&config).unwrap();
        let deserialized: TurnConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.max_tokens, 2048);
        assert_eq!(deserialized.temperature, Some(0.7));
        assert!(!deserialized.stream);
        assert_eq!(deserialized.stop_sequences, vec!["STOP"]);
    }
}
