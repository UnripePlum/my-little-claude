use std::collections::HashMap;
use std::path::PathBuf;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Definition of a tool sent to the LLM
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// A tool call from the LLM
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: serde_json::Value,
}

/// Context passed to tool execution
#[derive(Debug, Clone)]
pub struct ToolContext {
    pub cwd: PathBuf,
    pub session_id: String,
    pub env: HashMap<String, String>,
}

/// Result of tool execution.
/// Success/Failure are forwarded to the LLM. Error stops the engine loop.
#[derive(Debug)]
pub enum ToolResult {
    /// Tool executed successfully, output forwarded to LLM
    Success(String),
    /// Tool ran but the operation failed (e.g. exit code 1, file not found).
    /// Forwarded to LLM as an error result so it can retry or adjust.
    Failure(String),
    /// Tool infrastructure error (crash, timeout). Stops the engine loop.
    Error(anyhow::Error),
}

impl ToolResult {
    /// Whether this result should stop the engine loop
    pub fn is_fatal(&self) -> bool {
        matches!(self, ToolResult::Error(_))
    }

    /// Get the output string (for Success/Failure), or the error message
    pub fn output(&self) -> String {
        match self {
            ToolResult::Success(s) | ToolResult::Failure(s) => s.clone(),
            ToolResult::Error(e) => format!("Tool error: {e}"),
        }
    }

    /// Whether this is an error result (Failure or Error)
    pub fn is_error(&self) -> bool {
        !matches!(self, ToolResult::Success(_))
    }
}

/// Trait for tools that can be executed by the agent
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn schema(&self) -> serde_json::Value;

    /// Generate a ToolDefinition from this tool's metadata
    fn to_definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: self.description().to_string(),
            input_schema: self.schema(),
        }
    }

    /// Execute the tool with the given input
    async fn execute(
        &self,
        input: serde_json::Value,
        ctx: &ToolContext,
    ) -> anyhow::Result<ToolResult>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_result_success() {
        let result = ToolResult::Success("file contents".into());
        assert!(!result.is_fatal());
        assert!(!result.is_error());
        assert_eq!(result.output(), "file contents");
    }

    #[test]
    fn test_tool_result_failure() {
        let result = ToolResult::Failure("file not found: foo.rs".into());
        assert!(!result.is_fatal());
        assert!(result.is_error());
        assert_eq!(result.output(), "file not found: foo.rs");
    }

    #[test]
    fn test_tool_result_error() {
        let result = ToolResult::Error(anyhow::anyhow!("process crashed"));
        assert!(result.is_fatal());
        assert!(result.is_error());
        assert!(result.output().contains("process crashed"));
    }

    #[test]
    fn test_tool_definition_serialization() {
        let def = ToolDefinition {
            name: "read_file".into(),
            description: "Read a file".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" }
                },
                "required": ["path"]
            }),
        };
        let json = serde_json::to_string(&def).unwrap();
        assert!(json.contains("read_file"));
        let deserialized: ToolDefinition = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.name, "read_file");
    }

    #[test]
    fn test_tool_call_serialization() {
        let call = ToolCall {
            id: "call_001".into(),
            name: "bash".into(),
            input: serde_json::json!({"command": "ls"}),
        };
        let json = serde_json::to_string(&call).unwrap();
        let deserialized: ToolCall = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.id, "call_001");
        assert_eq!(deserialized.name, "bash");
    }

    #[test]
    fn test_tool_context_creation() {
        let ctx = ToolContext {
            cwd: PathBuf::from("/tmp/project"),
            session_id: "test-session".into(),
            env: HashMap::new(),
        };
        assert_eq!(ctx.cwd, PathBuf::from("/tmp/project"));
        assert_eq!(ctx.session_id, "test-session");
    }
}
