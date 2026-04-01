use std::path::PathBuf;

use async_trait::async_trait;
use unripe_core::tool::{Tool, ToolContext, ToolResult};

pub struct EditFileTool;

#[async_trait]
impl Tool for EditFileTool {
    fn name(&self) -> &str {
        "edit_file"
    }

    fn description(&self) -> &str {
        "Edit a file by replacing an exact string match with new content. \
         The old_string must be unique in the file unless replace_all is true."
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "The file path to edit (relative to cwd or absolute)"
                },
                "old_string": {
                    "type": "string",
                    "description": "The exact string to find and replace"
                },
                "new_string": {
                    "type": "string",
                    "description": "The replacement string"
                },
                "replace_all": {
                    "type": "boolean",
                    "description": "Replace all occurrences (default: false, requires unique match)"
                }
            },
            "required": ["path", "old_string", "new_string"]
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

        let old_string = input
            .get("old_string")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'old_string' parameter"))?;

        let new_string = input
            .get("new_string")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'new_string' parameter"))?;

        let replace_all = input
            .get("replace_all")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if old_string.is_empty() {
            return Ok(ToolResult::Failure("old_string must not be empty".into()));
        }

        if old_string == new_string {
            return Ok(ToolResult::Failure(
                "old_string and new_string are identical".into(),
            ));
        }

        let path = resolve_path(path_str, &ctx.cwd);

        // Read existing file
        let content = match tokio::fs::read_to_string(&path).await {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(ToolResult::Failure(format!(
                    "File not found: {}",
                    path.display()
                )));
            }
            Err(e) => {
                return Ok(ToolResult::Failure(format!(
                    "Failed to read {}: {e}",
                    path.display()
                )));
            }
        };

        // Count matches
        let match_count = content.matches(old_string).count();

        if match_count == 0 {
            return Ok(ToolResult::Failure(format!(
                "old_string not found in {}",
                path.display()
            )));
        }

        if match_count > 1 && !replace_all {
            return Ok(ToolResult::Failure(format!(
                "Found {match_count} matches in {}. Use replace_all: true or provide a more specific string.",
                path.display()
            )));
        }

        // Find affected line numbers (before replacement)
        let first_offset = content.find(old_string).unwrap_or(0);
        let start_line = content[..first_offset].lines().count().max(1);
        let old_line_count = old_string.lines().count().max(1);
        let end_line = start_line + old_line_count - 1;

        // Perform replacement
        let new_content = if replace_all {
            content.replace(old_string, new_string)
        } else {
            content.replacen(old_string, new_string, 1)
        };

        // Write back
        match tokio::fs::write(&path, &new_content).await {
            Ok(()) => {
                let msg = if replace_all && match_count > 1 {
                    format!("Edited {} ({match_count} replacements)", path.display())
                } else if start_line == end_line {
                    format!("Edited {}, line {start_line}", path.display())
                } else {
                    format!("Edited {}, lines {start_line}-{end_line}", path.display())
                };
                Ok(ToolResult::Success(msg))
            }
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
    async fn test_edit_single_replacement() {
        let dir = std::env::temp_dir().join("unripe-test-edit-single");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("test.txt"), "hello world\ngoodbye world\n").unwrap();

        let tool = EditFileTool;
        let result = tool
            .execute(
                serde_json::json!({
                    "path": "test.txt",
                    "old_string": "hello",
                    "new_string": "hi"
                }),
                &test_ctx(&dir),
            )
            .await
            .unwrap();

        match &result {
            ToolResult::Success(msg) => assert!(msg.contains("Edited")),
            other => panic!("expected Success, got {other:?}"),
        }

        let content = std::fs::read_to_string(dir.join("test.txt")).unwrap();
        assert_eq!(content, "hi world\ngoodbye world\n");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn test_edit_not_found() {
        let dir = std::env::temp_dir().join("unripe-test-edit-notfound");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("test.txt"), "hello world\n").unwrap();

        let tool = EditFileTool;
        let result = tool
            .execute(
                serde_json::json!({
                    "path": "test.txt",
                    "old_string": "nonexistent",
                    "new_string": "replacement"
                }),
                &test_ctx(&dir),
            )
            .await
            .unwrap();

        assert!(matches!(result, ToolResult::Failure(msg) if msg.contains("not found")));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn test_edit_multiple_matches_fails() {
        let dir = std::env::temp_dir().join("unripe-test-edit-multi");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("test.txt"), "foo bar foo baz foo\n").unwrap();

        let tool = EditFileTool;
        let result = tool
            .execute(
                serde_json::json!({
                    "path": "test.txt",
                    "old_string": "foo",
                    "new_string": "qux"
                }),
                &test_ctx(&dir),
            )
            .await
            .unwrap();

        assert!(matches!(result, ToolResult::Failure(msg) if msg.contains("3 matches")));

        // File should be unchanged
        let content = std::fs::read_to_string(dir.join("test.txt")).unwrap();
        assert_eq!(content, "foo bar foo baz foo\n");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn test_edit_replace_all() {
        let dir = std::env::temp_dir().join("unripe-test-edit-replall");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("test.txt"), "foo bar foo baz foo\n").unwrap();

        let tool = EditFileTool;
        let result = tool
            .execute(
                serde_json::json!({
                    "path": "test.txt",
                    "old_string": "foo",
                    "new_string": "qux",
                    "replace_all": true
                }),
                &test_ctx(&dir),
            )
            .await
            .unwrap();

        match &result {
            ToolResult::Success(msg) => assert!(msg.contains("3 replacements")),
            other => panic!("expected Success, got {other:?}"),
        }

        let content = std::fs::read_to_string(dir.join("test.txt")).unwrap();
        assert_eq!(content, "qux bar qux baz qux\n");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn test_edit_file_not_exists() {
        let dir = std::env::temp_dir().join("unripe-test-edit-nofile");
        std::fs::create_dir_all(&dir).unwrap();

        let tool = EditFileTool;
        let result = tool
            .execute(
                serde_json::json!({
                    "path": "nonexistent.txt",
                    "old_string": "a",
                    "new_string": "b"
                }),
                &test_ctx(&dir),
            )
            .await
            .unwrap();

        assert!(matches!(result, ToolResult::Failure(msg) if msg.contains("not found")));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn test_edit_empty_old_string() {
        let dir = std::env::temp_dir().join("unripe-test-edit-empty");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("test.txt"), "hello\n").unwrap();

        let tool = EditFileTool;
        let result = tool
            .execute(
                serde_json::json!({
                    "path": "test.txt",
                    "old_string": "",
                    "new_string": "x"
                }),
                &test_ctx(&dir),
            )
            .await
            .unwrap();

        assert!(matches!(result, ToolResult::Failure(msg) if msg.contains("must not be empty")));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn test_edit_identical_strings() {
        let dir = std::env::temp_dir().join("unripe-test-edit-identical");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("test.txt"), "hello\n").unwrap();

        let tool = EditFileTool;
        let result = tool
            .execute(
                serde_json::json!({
                    "path": "test.txt",
                    "old_string": "hello",
                    "new_string": "hello"
                }),
                &test_ctx(&dir),
            )
            .await
            .unwrap();

        assert!(matches!(result, ToolResult::Failure(msg) if msg.contains("identical")));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn test_edit_multiline() {
        let dir = std::env::temp_dir().join("unripe-test-edit-multiline");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("test.txt"), "line1\nline2\nline3\nline4\nline5\n").unwrap();

        let tool = EditFileTool;
        let result = tool
            .execute(
                serde_json::json!({
                    "path": "test.txt",
                    "old_string": "line2\nline3",
                    "new_string": "replaced2\nreplaced3\nextra"
                }),
                &test_ctx(&dir),
            )
            .await
            .unwrap();

        assert!(matches!(result, ToolResult::Success(_)));

        let content = std::fs::read_to_string(dir.join("test.txt")).unwrap();
        assert_eq!(
            content,
            "line1\nreplaced2\nreplaced3\nextra\nline4\nline5\n"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_tool_definition() {
        let tool = EditFileTool;
        let def = tool.to_definition();
        assert_eq!(def.name, "edit_file");
        let required = def.input_schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "old_string"));
        assert!(required.iter().any(|v| v == "new_string"));
    }

    #[tokio::test]
    async fn test_edit_missing_params() {
        let dir = std::env::temp_dir().join("unripe-test-edit-noparam");
        std::fs::create_dir_all(&dir).unwrap();

        let tool = EditFileTool;

        let result = tool
            .execute(serde_json::json!({"path": "foo.txt"}), &test_ctx(&dir))
            .await;
        assert!(result.is_err());

        let result = tool
            .execute(
                serde_json::json!({"path": "foo.txt", "old_string": "a"}),
                &test_ctx(&dir),
            )
            .await;
        assert!(result.is_err());

        std::fs::remove_dir_all(&dir).ok();
    }
}
