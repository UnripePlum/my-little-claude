use std::path::PathBuf;

use async_trait::async_trait;
use unripe_core::permission::ToolAction;
use unripe_core::tool::{Tool, ToolContext, ToolResult};

pub struct WriteFileTool;

#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &str {
        "write_file"
    }

    fn description(&self) -> &str {
        "Write content to a file at the given path, creating parent directories if needed"
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "The file path to write (relative to cwd or absolute)"
                },
                "content": {
                    "type": "string",
                    "description": "The content to write to the file"
                }
            },
            "required": ["path", "content"]
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

        let content = input
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'content' parameter"))?;

        let path = resolve_path(path_str, &ctx.cwd);

        // Create parent directories if needed
        if let Some(parent) = path.parent() {
            if let Err(e) = tokio::fs::create_dir_all(parent).await {
                return Ok(ToolResult::Failure(format!(
                    "Failed to create directories for {}: {e}",
                    path.display()
                )));
            }
        }

        match tokio::fs::write(&path, content).await {
            Ok(()) => Ok(ToolResult::Success(format!(
                "Written {} bytes to {}",
                content.len(),
                path.display()
            ))),
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => Ok(ToolResult::Failure(
                format!("Permission denied: {}", path.display()),
            )),
            Err(e) => Ok(ToolResult::Failure(format!(
                "Failed to write {}: {e}",
                path.display()
            ))),
        }
    }
}

impl WriteFileTool {
    pub fn make_action(path_str: &str, cwd: &std::path::Path) -> ToolAction {
        ToolAction::FileWrite(resolve_path(path_str, cwd))
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
    async fn test_write_new_file() {
        let dir = std::env::temp_dir().join("unripe-test-write-new");
        std::fs::create_dir_all(&dir).unwrap();

        let tool = WriteFileTool;
        let result = tool
            .execute(
                serde_json::json!({"path": "output.txt", "content": "hello"}),
                &test_ctx(&dir),
            )
            .await
            .unwrap();

        match &result {
            ToolResult::Success(msg) => assert!(msg.contains("5 bytes")),
            other => panic!("expected Success, got {other:?}"),
        }

        let written = std::fs::read_to_string(dir.join("output.txt")).unwrap();
        assert_eq!(written, "hello");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn test_write_overwrite_existing() {
        let dir = std::env::temp_dir().join("unripe-test-write-overwrite");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("existing.txt"), "old content").unwrap();

        let tool = WriteFileTool;
        let result = tool
            .execute(
                serde_json::json!({"path": "existing.txt", "content": "new content"}),
                &test_ctx(&dir),
            )
            .await
            .unwrap();

        assert!(matches!(result, ToolResult::Success(_)));
        let written = std::fs::read_to_string(dir.join("existing.txt")).unwrap();
        assert_eq!(written, "new content");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn test_write_creates_parent_dirs() {
        let dir = std::env::temp_dir().join("unripe-test-write-parents");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let tool = WriteFileTool;
        let result = tool
            .execute(
                serde_json::json!({"path": "sub/dir/file.txt", "content": "nested"}),
                &test_ctx(&dir),
            )
            .await
            .unwrap();

        assert!(matches!(result, ToolResult::Success(_)));
        let written = std::fs::read_to_string(dir.join("sub/dir/file.txt")).unwrap();
        assert_eq!(written, "nested");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn test_write_missing_params() {
        let dir = std::env::temp_dir().join("unripe-test-write-noparam");
        std::fs::create_dir_all(&dir).unwrap();

        let tool = WriteFileTool;

        // Missing content
        let result = tool
            .execute(serde_json::json!({"path": "foo.txt"}), &test_ctx(&dir))
            .await;
        assert!(result.is_err());

        // Missing path
        let result = tool
            .execute(serde_json::json!({"content": "hello"}), &test_ctx(&dir))
            .await;
        assert!(result.is_err());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_tool_definition() {
        let tool = WriteFileTool;
        let def = tool.to_definition();
        assert_eq!(def.name, "write_file");
        assert!(def.input_schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "content"));
    }
}
