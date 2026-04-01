use std::collections::HashMap;

use rmcp::model::CallToolRequestParams;
use rmcp::service::RunningService;
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use rmcp::ServiceExt;

use crate::config::McpServerConfig;

/// A connected MCP server with its available tools
pub struct McpConnection {
    pub server_name: String,
    pub service: RunningService<rmcp::RoleClient, ()>,
    pub tools: Vec<McpToolInfo>,
    config: McpServerConfig,
}

/// Info about a tool exposed by an MCP server
#[derive(Debug, Clone)]
pub struct McpToolInfo {
    pub server_name: String,
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

impl McpConnection {
    /// Connect to an MCP server and discover its tools
    pub async fn connect(server_name: &str, config: &McpServerConfig) -> anyhow::Result<Self> {
        let transport = TokioChildProcess::new(
            tokio::process::Command::new(&config.command).configure(|cmd| {
                cmd.args(&config.args);
                for (k, v) in &config.env {
                    cmd.env(k, v);
                }
            }),
        )?;

        let service = ().serve(transport).await?;

        // Discover tools using paginated list_all_tools
        let tool_list = service.list_all_tools().await?;

        let tools: Vec<McpToolInfo> = tool_list
            .into_iter()
            .map(|t| McpToolInfo {
                server_name: server_name.to_string(),
                name: t.name.to_string(),
                description: t.description.map(|d| d.to_string()).unwrap_or_default(),
                input_schema: serde_json::to_value(&t.input_schema).unwrap_or_default(),
            })
            .collect();

        Ok(Self {
            server_name: server_name.to_string(),
            service,
            tools,
            config: config.clone(),
        })
    }

    /// Reconnect to the MCP server after a disconnect
    pub async fn reconnect(&mut self) -> anyhow::Result<()> {
        eprintln!(
            "\x1b[33m[mcp] Reconnecting to '{}'...\x1b[0m",
            self.server_name
        );

        let transport = TokioChildProcess::new(
            tokio::process::Command::new(&self.config.command).configure(|cmd| {
                cmd.args(&self.config.args);
                for (k, v) in &self.config.env {
                    cmd.env(k, v);
                }
            }),
        )?;

        self.service = ().serve(transport).await?;

        eprintln!("\x1b[32m[mcp] Reconnected to '{}'\x1b[0m", self.server_name);
        Ok(())
    }

    /// Call a tool on this MCP server
    pub async fn call_tool(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> anyhow::Result<String> {
        let result = self
            .service
            .call_tool(CallToolRequestParams {
                meta: None,
                name: name.to_string().into(),
                arguments: arguments.as_object().cloned(),
                task: None,
            })
            .await?;

        // Collect text content from the result
        let output: Vec<String> = result
            .content
            .into_iter()
            .filter_map(|c| match c.raw {
                rmcp::model::RawContent::Text(text) => Some(text.text),
                _ => None,
            })
            .collect();

        Ok(output.join("\n"))
    }

    /// Shutdown the MCP server
    pub async fn shutdown(self) -> anyhow::Result<()> {
        self.service.cancel().await?;
        Ok(())
    }
}

/// Connect to all configured MCP servers
pub async fn connect_all(servers: &HashMap<String, McpServerConfig>) -> Vec<McpConnection> {
    let mut connections = Vec::new();

    for (name, config) in servers {
        match McpConnection::connect(name, config).await {
            Ok(conn) => {
                let tool_count = conn.tools.len();
                eprintln!(
                    "\x1b[36m[mcp] Connected to '{}': {} tools\x1b[0m",
                    name, tool_count
                );
                connections.push(conn);
            }
            Err(e) => {
                eprintln!(
                    "\x1b[33m[mcp] Failed to connect to '{}': {}\x1b[0m",
                    name, e
                );
            }
        }
    }

    connections
}
