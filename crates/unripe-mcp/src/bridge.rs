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
        match conn.call_tool(&self.tool_name, input.clone()).await {
            Ok(output) => Ok(ToolResult::Success(output)),
            Err(e) => {
                let err_str = e.to_string();
                // Detect disconnection and attempt reconnect
                if err_str.contains("closed")
                    || err_str.contains("broken pipe")
                    || err_str.contains("connection")
                {
                    drop(conn);
                    // Try to reconnect
                    let mut conn = self.connection.lock().await;
                    match conn.reconnect().await {
                        Ok(()) => {
                            // Retry the call once after reconnect
                            match conn.call_tool(&self.tool_name, input).await {
                                Ok(output) => return Ok(ToolResult::Success(output)),
                                Err(e2) => {
                                    return Ok(ToolResult::Failure(format!(
                                        "MCP server '{}' reconnected but tool '{}' still failed: {e2}",
                                        self.server_name, self.tool_name
                                    )));
                                }
                            }
                        }
                        Err(re) => {
                            return Ok(ToolResult::Failure(format!(
                                "MCP server '{}' disconnected and reconnect failed: {re}",
                                self.server_name
                            )));
                        }
                    }
                }
                Ok(ToolResult::Failure(format!(
                    "MCP tool '{}' on server '{}' error: {e}",
                    self.tool_name, self.server_name
                )))
            }
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
