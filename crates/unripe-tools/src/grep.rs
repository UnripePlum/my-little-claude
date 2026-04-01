use std::path::PathBuf;

use async_trait::async_trait;
use unripe_core::tool::{Tool, ToolContext, ToolResult};

/// Grep tool: search file contents for a pattern
pub struct GrepTool;

#[async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &str {
        "grep"
    }

    fn description(&self) -> &str {
        "Search file contents for a regex pattern. Returns matching lines with file paths and line numbers."
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Regex pattern to search for"
                },
                "path": {
                    "type": "string",
                    "description": "Directory or file to search in (default: current directory)"
                },
                "file_pattern": {
                    "type": "string",
                    "description": "Glob pattern to filter files (e.g. '*.rs')"
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

        let search_path = input
            .get("path")
            .and_then(|v| v.as_str())
            .map(|p| {
                let path = PathBuf::from(p);
                if path.is_absolute() {
                    path
                } else {
                    ctx.cwd.join(path)
                }
            })
            .unwrap_or_else(|| ctx.cwd.clone());

        let file_pattern = input
            .get("file_pattern")
            .and_then(|v| v.as_str())
            .unwrap_or("*");

        // Use grep command for speed and reliability
        let mut cmd = tokio::process::Command::new("grep");
        cmd.arg("-rn") // recursive + line numbers
            .arg("--include")
            .arg(file_pattern)
            .arg("-E") // extended regex
            .arg(pattern)
            .arg(&search_path)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let output = match cmd.output().await {
            Ok(o) => o,
            Err(e) => {
                return Ok(ToolResult::Error(anyhow::anyhow!(
                    "Failed to run grep: {e}"
                )))
            }
        };

        let stdout = String::from_utf8_lossy(&output.stdout);

        if output.status.success() {
            // Make paths relative to cwd
            let cwd_str = ctx.cwd.to_string_lossy();
            let relative_output = stdout.replace(&*cwd_str, "").replace("/./", "/");
            let relative_output = relative_output.trim_start_matches('/');

            let mut result = relative_output.to_string();
            let line_count = result.lines().count();
            if line_count > 200 {
                let truncated: String = result.lines().take(200).collect::<Vec<_>>().join("\n");
                result =
                    format!("{truncated}\n\n[... {line_count} matches total, showing first 200]");
            }
            Ok(ToolResult::Success(result))
        } else if output.status.code() == Some(1) {
            // grep exit 1 = no matches (not an error)
            Ok(ToolResult::Success("No matches found.".into()))
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Ok(ToolResult::Failure(format!("grep error: {stderr}")))
        }
    }
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
    async fn test_grep_finds_matches() {
        let dir = std::env::temp_dir().join("unripe-test-grep-match");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(
            dir.join("src/main.rs"),
            "fn main() {\n    println!(\"hello\");\n}\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("src/lib.rs"),
            "pub fn hello() {}\npub fn world() {}\n",
        )
        .unwrap();

        let tool = GrepTool;
        let result = tool
            .execute(
                serde_json::json!({"pattern": "hello", "file_pattern": "*.rs"}),
                &test_ctx(&dir),
            )
            .await
            .unwrap();

        match &result {
            ToolResult::Success(output) => {
                assert!(output.contains("hello"));
                assert!(output.contains("main.rs") || output.contains("lib.rs"));
            }
            other => panic!("expected Success, got {other:?}"),
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn test_grep_no_matches() {
        let dir = std::env::temp_dir().join("unripe-test-grep-nomatch");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("test.txt"), "nothing here").unwrap();

        let tool = GrepTool;
        let result = tool
            .execute(
                serde_json::json!({"pattern": "zzzznotfound"}),
                &test_ctx(&dir),
            )
            .await
            .unwrap();

        match &result {
            ToolResult::Success(output) => assert!(output.contains("No matches")),
            other => panic!("expected Success (no matches), got {other:?}"),
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn test_grep_specific_path() {
        let dir = std::env::temp_dir().join("unripe-test-grep-path");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("a")).unwrap();
        std::fs::create_dir_all(dir.join("b")).unwrap();
        std::fs::write(dir.join("a/file.txt"), "target_word").unwrap();
        std::fs::write(dir.join("b/file.txt"), "other_content").unwrap();

        let tool = GrepTool;
        let result = tool
            .execute(
                serde_json::json!({"pattern": "target_word", "path": "a"}),
                &test_ctx(&dir),
            )
            .await
            .unwrap();

        match &result {
            ToolResult::Success(output) => {
                assert!(output.contains("target_word"));
            }
            other => panic!("expected Success, got {other:?}"),
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn test_grep_missing_pattern() {
        let dir = std::env::temp_dir().join("unripe-test-grep-noparam");
        std::fs::create_dir_all(&dir).unwrap();
        let tool = GrepTool;
        let result = tool.execute(serde_json::json!({}), &test_ctx(&dir)).await;
        assert!(result.is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn test_grep_invalid_regex() {
        let dir = std::env::temp_dir().join("unripe-test-grep-badregex");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("test.txt"), "some content").unwrap();

        let tool = GrepTool;
        let result = tool
            .execute(serde_json::json!({"pattern": "[invalid"}), &test_ctx(&dir))
            .await
            .unwrap();

        match &result {
            ToolResult::Failure(msg) => assert!(msg.contains("grep error")),
            ToolResult::Success(msg) => {
                // Some grep implementations treat invalid regex as literal
                assert!(!msg.is_empty());
            }
            other => panic!("expected Failure or Success for invalid regex, got {other:?}"),
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_tool_definition() {
        let tool = GrepTool;
        let def = tool.to_definition();
        assert_eq!(def.name, "grep");
        assert!(def.description.contains("regex"));
    }
}
