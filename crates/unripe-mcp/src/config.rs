use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// MCP server configuration (matches Claude Code's .mcp.json format)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpConfig {
    #[serde(default, rename = "mcpServers")]
    pub mcp_servers: HashMap<String, McpServerConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

/// Load MCP config from .mcp.json (project-local) or ~/.claude.json (global)
pub fn load_mcp_config(project_root: &Path) -> McpConfig {
    // 1. Project-local .mcp.json (highest priority)
    let local_path = project_root.join(".mcp.json");
    if let Some(config) = try_load_mcp_file(&local_path) {
        return config;
    }

    // 2. Global ~/.claude.json
    if let Some(home) = dirs::home_dir() {
        let global_path = home.join(".claude.json");
        if let Some(config) = try_load_mcp_file(&global_path) {
            return config;
        }
    }

    // 3. Our own ~/.unripe/mcp.json
    if let Some(home) = dirs::home_dir() {
        let unripe_path = home.join(".unripe").join("mcp.json");
        if let Some(config) = try_load_mcp_file(&unripe_path) {
            return config;
        }
    }

    McpConfig {
        mcp_servers: HashMap::new(),
    }
}

fn try_load_mcp_file(path: &Path) -> Option<McpConfig> {
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

/// Get the paths that were checked for MCP config (for diagnostics)
pub fn mcp_config_paths(project_root: &Path) -> Vec<PathBuf> {
    let mut paths = vec![project_root.join(".mcp.json")];
    if let Some(home) = dirs::home_dir() {
        paths.push(home.join(".claude.json"));
        paths.push(home.join(".unripe").join("mcp.json"));
    }
    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_mcp_config() {
        let json = r#"{
            "mcpServers": {
                "filesystem": {
                    "command": "npx",
                    "args": ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"],
                    "env": {}
                },
                "git": {
                    "command": "uvx",
                    "args": ["mcp-server-git"],
                    "env": {"GIT_DIR": "/repo"}
                }
            }
        }"#;
        let config: McpConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.mcp_servers.len(), 2);
        assert!(config.mcp_servers.contains_key("filesystem"));
        assert!(config.mcp_servers.contains_key("git"));

        let fs = &config.mcp_servers["filesystem"];
        assert_eq!(fs.command, "npx");
        assert_eq!(fs.args.len(), 3);

        let git = &config.mcp_servers["git"];
        assert_eq!(git.env["GIT_DIR"], "/repo");
    }

    #[test]
    fn test_parse_empty_config() {
        let json = r#"{"mcpServers": {}}"#;
        let config: McpConfig = serde_json::from_str(json).unwrap();
        assert!(config.mcp_servers.is_empty());
    }

    #[test]
    fn test_parse_minimal_server() {
        let json = r#"{"mcpServers": {"test": {"command": "echo"}}}"#;
        let config: McpConfig = serde_json::from_str(json).unwrap();
        let server = &config.mcp_servers["test"];
        assert_eq!(server.command, "echo");
        assert!(server.args.is_empty());
        assert!(server.env.is_empty());
    }

    #[test]
    fn test_load_mcp_config_missing_files() {
        let dir = std::env::temp_dir().join("unripe-test-mcp-nofile");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let config = load_mcp_config(&dir);
        assert!(config.mcp_servers.is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_load_mcp_config_from_project() {
        let dir = std::env::temp_dir().join("unripe-test-mcp-project");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        std::fs::write(
            dir.join(".mcp.json"),
            r#"{"mcpServers": {"test": {"command": "echo", "args": ["hello"]}}}"#,
        )
        .unwrap();

        let config = load_mcp_config(&dir);
        assert_eq!(config.mcp_servers.len(), 1);
        assert_eq!(config.mcp_servers["test"].command, "echo");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_mcp_config_paths() {
        let dir = PathBuf::from("/project");
        let paths = mcp_config_paths(&dir);
        assert!(paths[0].ends_with(".mcp.json"));
        assert!(paths.len() >= 2);
    }
}
