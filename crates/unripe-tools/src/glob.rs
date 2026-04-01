use std::path::PathBuf;

use async_trait::async_trait;
use unripe_core::tool::{Tool, ToolContext, ToolResult};

/// Glob tool: find files matching a pattern
pub struct GlobTool;

#[async_trait]
impl Tool for GlobTool {
    fn name(&self) -> &str {
        "glob"
    }

    fn description(&self) -> &str {
        "Find files matching a glob pattern (e.g. '**/*.rs', 'src/**/*.ts')"
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Glob pattern to match files (e.g. '**/*.rs')"
                }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(
        &self,
        input: serde_json::Value,
        ctx: &ToolContext,
    ) -> anyhow::Result<ToolResult> {
        let pattern = input
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'pattern' parameter"))?;

        let full_pattern = ctx.cwd.join(pattern).to_string_lossy().to_string();

        match glob_match(&full_pattern) {
            Ok(paths) => {
                if paths.is_empty() {
                    Ok(ToolResult::Success("No files matched.".into()))
                } else {
                    let relative: Vec<String> = paths
                        .iter()
                        .filter_map(|p| p.strip_prefix(&ctx.cwd).ok())
                        .map(|p| p.display().to_string())
                        .collect();
                    let count = relative.len();
                    let mut output = relative.join("\n");
                    if count > 200 {
                        output.truncate(output.lines().take(200).map(|l| l.len() + 1).sum());
                        output
                            .push_str(&format!("\n\n[... {count} files total, showing first 200]"));
                    }
                    Ok(ToolResult::Success(output))
                }
            }
            Err(e) => Ok(ToolResult::Failure(format!("Invalid glob pattern: {e}"))),
        }
    }
}

fn glob_match(pattern: &str) -> Result<Vec<PathBuf>, String> {
    let mut results = Vec::new();
    let entries = glob::glob(pattern).map_err(|e| e.to_string())?;
    for entry in entries {
        match entry {
            Ok(path) if path.is_file() => results.push(path),
            Ok(_) => {}  // skip directories
            Err(_) => {} // skip permission errors
        }
    }
    results.sort();
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn test_ctx(dir: &std::path::Path) -> ToolContext {
        ToolContext {
            cwd: dir.to_path_buf(),
            session_id: "test".into(),
            env: HashMap::new(),
        }
    }

    #[tokio::test]
    async fn test_glob_rs_files() {
        let dir = std::env::temp_dir().join("unripe-test-glob-rs");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/main.rs"), "fn main() {}").unwrap();
        std::fs::write(dir.join("src/lib.rs"), "pub mod foo;").unwrap();
        std::fs::write(dir.join("README.md"), "# hello").unwrap();

        let tool = GlobTool;
        let result = tool
            .execute(serde_json::json!({"pattern": "**/*.rs"}), &test_ctx(&dir))
            .await
            .unwrap();

        match &result {
            ToolResult::Success(output) => {
                assert!(output.contains("main.rs"));
                assert!(output.contains("lib.rs"));
                assert!(!output.contains("README"));
            }
            other => panic!("expected Success, got {other:?}"),
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn test_glob_no_match() {
        let dir = std::env::temp_dir().join("unripe-test-glob-empty");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let tool = GlobTool;
        let result = tool
            .execute(serde_json::json!({"pattern": "**/*.xyz"}), &test_ctx(&dir))
            .await
            .unwrap();

        match &result {
            ToolResult::Success(output) => assert!(output.contains("No files")),
            other => panic!("expected Success, got {other:?}"),
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn test_glob_missing_param() {
        let dir = std::env::temp_dir().join("unripe-test-glob-noparam");
        std::fs::create_dir_all(&dir).unwrap();
        let tool = GlobTool;
        let result = tool.execute(serde_json::json!({}), &test_ctx(&dir)).await;
        assert!(result.is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_tool_definition() {
        let tool = GlobTool;
        let def = tool.to_definition();
        assert_eq!(def.name, "glob");
    }
}
