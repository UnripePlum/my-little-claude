use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::Mutex;
use unripe_core::tool::{Tool, ToolContext, ToolResult};

use crate::client::McpConnection;

/// Wraps an MCP server tool as an unripe-core Tool
pub struct McpTool {
    pub server_name: String,
    pub tool_name: String,
    pub tool_description: String,
    pub tool_schema: serde_json::Value,
    connection: Arc<Mutex<McpConnection>>,
}

impl McpTool {
    pub fn new(
        server_name: String,
        tool_name: String,
        tool_description: String,
        tool_schema: serde_json::Value,
        connection: Arc<Mutex<McpConnection>>,
    ) -> Self {
        Self {
            server_name,
            tool_name,
            tool_description,
            tool_schema,
            connection,
        }
    }
}

#[async_trait]
impl Tool for McpTool {
    fn name(&self) -> &str {
        &self.tool_name
    }

    fn description(&self) -> &str {
        &self.tool_description
    }

    fn schema(&self) -> serde_json::Value {
        self.tool_schema.clone()
    }

    async fn execute(
        &self,
        input: serde_json::Value,
        _ctx: &ToolContext,
    ) -> anyhow::Result<ToolResult> {
        let conn = self.connection.lock().await;
        match conn.call_tool(&self.tool_name, input).await {
            Ok(output) => Ok(ToolResult::Success(output)),
            Err(e) => Ok(ToolResult::Failure(format!(
                "MCP tool '{}' error: {e}",
                self.tool_name
            ))),
        }
    }
}

/// Convert MCP connections into a list of boxed Tools
pub fn connections_to_tools(connections: Vec<McpConnection>) -> Vec<Box<dyn Tool>> {
    let mut tools: Vec<Box<dyn Tool>> = Vec::new();

    for conn in connections {
        let tool_infos = conn.tools.clone();
        let shared = Arc::new(Mutex::new(conn));

        for info in tool_infos {
            tools.push(Box::new(McpTool::new(
                info.server_name,
                info.name,
                info.description,
                info.input_schema,
                Arc::clone(&shared),
            )));
        }
    }

    tools
}
