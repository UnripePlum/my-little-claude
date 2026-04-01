use std::path::PathBuf;

use async_trait::async_trait;
use unripe_core::permission::ToolAction;
use unripe_core::tool::{Tool, ToolContext, ToolResult};

pub struct ReadFileTool;

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }

    fn description(&self) -> &str {
        "Read the contents of a file at the given path"
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "The file path to read (relative to cwd or absolute)"
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(
        &self,
        input: serde_json::Value,
        ctx: &ToolContext,
    ) -> anyhow::Result<ToolResult> {
        let path_str = input
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'path' parameter"))?;

        let path = resolve_path(path_str, &ctx.cwd);

        match tokio::fs::read_to_string(&path).await {
            Ok(content) => {
                // Truncate very large files
                if content.len() > 100_000 {
                    let truncated = &content[..100_000];
                    Ok(ToolResult::Success(format!(
                        "{truncated}\n\n[... truncated, file is {} bytes]",
                        content.len()
                    )))
                } else {
                    Ok(ToolResult::Success(content))
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(ToolResult::Failure(format!(
                "File not found: {}",
                path.display()
            ))),
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => Ok(ToolResult::Failure(
                format!("Permission denied: {}", path.display()),
            )),
            Err(e) => {
                // Could be binary file or other IO error
                Ok(ToolResult::Failure(format!(
                    "Failed to read {}: {e}",
                    path.display()
                )))
            }
        }
    }
}

impl ReadFileTool {
    pub fn make_action(path_str: &str, cwd: &std::path::Path) -> ToolAction {
        ToolAction::FileRead(resolve_path(path_str, cwd))
    }
}

fn resolve_path(path_str: &str, cwd: &std::path::Path) -> PathBuf {
    let path = PathBuf::from(path_str);
    if path.is_absolute() {
        path
    } else {
        cwd.join(path)
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
    async fn test_read_existing_file() {
        let dir = std::env::temp_dir().join("unripe-test-read-file");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("hello.txt"), "hello world").unwrap();

        let tool = ReadFileTool;
        let result = tool
            .execute(serde_json::json!({"path": "hello.txt"}), &test_ctx(&dir))
            .await
            .unwrap();

        match result {
            ToolResult::Success(content) => assert_eq!(content, "hello world"),
            other => panic!("expected Success, got {other:?}"),
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn test_read_file_not_found() {
        let dir = std::env::temp_dir().join("unripe-test-read-notfound");
        std::fs::create_dir_all(&dir).unwrap();

        let tool = ReadFileTool;
        let result = tool
            .execute(
                serde_json::json!({"path": "nonexistent.rs"}),
                &test_ctx(&dir),
            )
            .await
            .unwrap();

        match result {
            ToolResult::Failure(msg) => assert!(msg.contains("not found")),
            other => panic!("expected Failure, got {other:?}"),
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn test_read_absolute_path() {
        let dir = std::env::temp_dir().join("unripe-test-read-abs");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("abs.txt");
        std::fs::write(&file, "absolute content").unwrap();

        let tool = ReadFileTool;
        let result = tool
            .execute(
                serde_json::json!({"path": file.to_str().unwrap()}),
                &test_ctx(&dir),
            )
            .await
            .unwrap();

        match result {
            ToolResult::Success(content) => assert_eq!(content, "absolute content"),
            other => panic!("expected Success, got {other:?}"),
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn test_read_missing_path_param() {
        let dir = std::env::temp_dir().join("unripe-test-read-noparam");
        std::fs::create_dir_all(&dir).unwrap();

        let tool = ReadFileTool;
        let result = tool.execute(serde_json::json!({}), &test_ctx(&dir)).await;

        assert!(result.is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_tool_definition() {
        let tool = ReadFileTool;
        let def = tool.to_definition();
        assert_eq!(def.name, "read_file");
        assert!(def.description.contains("Read"));
        assert!(def.input_schema.get("properties").is_some());
    }
}
