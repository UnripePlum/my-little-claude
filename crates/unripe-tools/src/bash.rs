use std::time::Duration;

use async_trait::async_trait;
use tokio::process::Command;
use unripe_core::permission::ToolAction;
use unripe_core::tool::{Tool, ToolContext, ToolResult};

pub struct BashTool {
    pub timeout_secs: u64,
}

impl BashTool {
    pub fn new(timeout_secs: u64) -> Self {
        Self { timeout_secs }
    }
}

impl Default for BashTool {
    fn default() -> Self {
        Self { timeout_secs: 30 }
    }
}

#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        "Execute a bash command and return stdout/stderr"
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The bash command to execute"
                }
            },
            "required": ["command"]
        })
    }

    async fn execute(
        &self,
        input: serde_json::Value,
        ctx: &ToolContext,
    ) -> anyhow::Result<ToolResult> {
        let command = input
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'command' parameter"))?;

        let timeout = Duration::from_secs(self.timeout_secs);

        let mut child = match Command::new("bash")
            .arg("-c")
            .arg(command)
            .current_dir(&ctx.cwd)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                return Ok(ToolResult::Error(anyhow::anyhow!(
                    "Failed to spawn bash: {e}"
                )));
            }
        };

        // Wait with timeout. Use child.wait() so we retain ownership for kill.
        let wait_result = tokio::time::timeout(timeout, child.wait()).await;

        match wait_result {
            Ok(Ok(status)) => {
                // Process finished — read stdout/stderr from the pipes
                let mut stdout_str = String::new();
                let mut stderr_str = String::new();

                if let Some(mut stdout) = child.stdout.take() {
                    use tokio::io::AsyncReadExt;
                    let _ = stdout.read_to_string(&mut stdout_str).await;
                }
                if let Some(mut stderr) = child.stderr.take() {
                    use tokio::io::AsyncReadExt;
                    let _ = stderr.read_to_string(&mut stderr_str).await;
                }

                let mut combined = String::new();
                if !stdout_str.is_empty() {
                    combined.push_str(&stdout_str);
                }
                if !stderr_str.is_empty() {
                    if !combined.is_empty() {
                        combined.push('\n');
                    }
                    combined.push_str("STDERR:\n");
                    combined.push_str(&stderr_str);
                }

                // Truncate very long output
                if combined.len() > 50_000 {
                    combined.truncate(50_000);
                    combined.push_str("\n\n[... output truncated at 50KB]");
                }

                if status.success() {
                    Ok(ToolResult::Success(combined))
                } else {
                    let code = status.code().unwrap_or(-1);
                    Ok(ToolResult::Failure(format!(
                        "Command exited with code {code}\n{combined}"
                    )))
                }
            }
            Ok(Err(e)) => Ok(ToolResult::Error(anyhow::anyhow!(
                "Failed to execute command: {e}"
            ))),
            Err(_) => {
                // Timeout — kill the child process (we still own it)
                let _ = child.kill().await;
                Ok(ToolResult::Failure(format!(
                    "Command timed out after {}s",
                    self.timeout_secs
                )))
            }
        }
    }
}

impl BashTool {
    pub fn make_action(command: &str) -> ToolAction {
        ToolAction::BashExec(command.to_string())
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
    async fn test_bash_echo() {
        let dir = std::env::temp_dir().join("unripe-test-bash-echo");
        std::fs::create_dir_all(&dir).unwrap();

        let tool = BashTool::default();
        let result = tool
            .execute(
                serde_json::json!({"command": "echo hello"}),
                &test_ctx(&dir),
            )
            .await
            .unwrap();

        match result {
            ToolResult::Success(output) => assert!(output.trim() == "hello"),
            other => panic!("expected Success, got {other:?}"),
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn test_bash_exit_code_1() {
        let dir = std::env::temp_dir().join("unripe-test-bash-fail");
        std::fs::create_dir_all(&dir).unwrap();

        let tool = BashTool::default();
        let result = tool
            .execute(serde_json::json!({"command": "exit 1"}), &test_ctx(&dir))
            .await
            .unwrap();

        match result {
            ToolResult::Failure(msg) => assert!(msg.contains("code 1")),
            other => panic!("expected Failure, got {other:?}"),
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn test_bash_stderr_capture() {
        let dir = std::env::temp_dir().join("unripe-test-bash-stderr");
        std::fs::create_dir_all(&dir).unwrap();

        let tool = BashTool::default();
        let result = tool
            .execute(
                serde_json::json!({"command": "echo out && echo err >&2"}),
                &test_ctx(&dir),
            )
            .await
            .unwrap();

        match result {
            ToolResult::Success(output) => {
                assert!(output.contains("out"));
                assert!(output.contains("STDERR:"));
                assert!(output.contains("err"));
            }
            other => panic!("expected Success, got {other:?}"),
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn test_bash_timeout() {
        let dir = std::env::temp_dir().join("unripe-test-bash-timeout");
        std::fs::create_dir_all(&dir).unwrap();

        let tool = BashTool::new(1); // 1 second timeout
        let result = tool
            .execute(serde_json::json!({"command": "sleep 10"}), &test_ctx(&dir))
            .await
            .unwrap();

        match result {
            ToolResult::Failure(msg) => assert!(msg.contains("timed out")),
            other => panic!("expected Failure (timeout), got {other:?}"),
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn test_bash_missing_command_param() {
        let dir = std::env::temp_dir().join("unripe-test-bash-noparam");
        std::fs::create_dir_all(&dir).unwrap();

        let tool = BashTool::default();
        let result = tool.execute(serde_json::json!({}), &test_ctx(&dir)).await;
        assert!(result.is_err());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn test_bash_uses_cwd() {
        let dir = std::env::temp_dir().join("unripe-test-bash-cwd");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("marker.txt"), "found").unwrap();

        let tool = BashTool::default();
        let result = tool
            .execute(
                serde_json::json!({"command": "cat marker.txt"}),
                &test_ctx(&dir),
            )
            .await
            .unwrap();

        match result {
            ToolResult::Success(output) => assert!(output.contains("found")),
            other => panic!("expected Success, got {other:?}"),
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_tool_definition() {
        let tool = BashTool::default();
        let def = tool.to_definition();
        assert_eq!(def.name, "bash");
        assert!(def.input_schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "command"));
    }
}
