use std::collections::HashMap;
use std::path::PathBuf;

use unripe_core::config::AgentConfig;
use unripe_core::message::{ContentBlock, Message, Role};
use unripe_core::permission::{Permission, PermissionGate, ToolAction};
use unripe_core::provider::{LlmProvider, TurnConfig, TurnResponse};
use unripe_core::session::Session;
use unripe_core::tool::{Tool, ToolContext, ToolResult};

use crate::bootstrap;

/// Reason the engine stopped
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    EndTurn,
    MaxTurns,
    TokenBudget,
    ToolError(String),
    ProviderError(String),
}

/// Callback for handling permission Ask prompts and streaming output
#[async_trait::async_trait]
pub trait EngineCallbacks: Send + Sync {
    /// Called when a tool needs user permission. Return true to allow.
    async fn ask_permission(&self, prompt: &str) -> bool;

    /// Called with text deltas during streaming
    async fn on_text(&self, text: &str);

    /// Called when a tool starts executing
    async fn on_tool_start(&self, tool_name: &str, input: &serde_json::Value);

    /// Called when a tool finishes
    async fn on_tool_end(&self, tool_name: &str, result: &str, is_error: bool);
}

/// The agent engine loop
pub struct AgentEngine {
    provider: Box<dyn LlmProvider>,
    tools: Vec<Box<dyn Tool>>,
    permission_gate: Box<dyn PermissionGate>,
    config: AgentConfig,
    project_root: PathBuf,
    chat_only: bool,
}

impl AgentEngine {
    pub fn new(
        provider: Box<dyn LlmProvider>,
        tools: Vec<Box<dyn Tool>>,
        permission_gate: Box<dyn PermissionGate>,
        config: AgentConfig,
        project_root: PathBuf,
    ) -> Self {
        Self {
            provider,
            tools,
            permission_gate,
            config,
            project_root,
            chat_only: false,
        }
    }

    /// Enable chat-only mode (no tool calling, just conversation)
    pub fn with_chat_only(mut self, chat_only: bool) -> Self {
        self.chat_only = chat_only;
        self
    }

    /// Run the agent loop for a given prompt
    pub async fn run(
        &self,
        prompt: &str,
        session: &mut Session,
        callbacks: &dyn EngineCallbacks,
    ) -> anyhow::Result<StopReason> {
        // Bootstrap: load project context if session is fresh
        if session.messages.is_empty() {
            let system_prompt =
                bootstrap::build_system_prompt(&self.project_root, &self.config.context_files);
            session.add_message(Message::text(Role::System, system_prompt));
        }

        // Add user prompt
        session.add_message(Message::text(Role::User, prompt));

        // Truncate if resuming a long session
        session.truncate(self.config.truncation_keep_recent);

        // Build tool definitions (empty = chat-only mode)
        let tool_defs: Vec<_> = if self.chat_only {
            Vec::new()
        } else {
            self.tools.iter().map(|t| t.to_definition()).collect()
        };

        let turn_config = TurnConfig {
            max_tokens: 4096,
            temperature: None,
            stream: false, // Use non-streaming for the loop, stream final text separately
            stop_sequences: Vec::new(),
        };

        // Agent loop
        loop {
            // Guard: max turns
            if session.turn_count >= self.config.max_turns {
                callbacks
                    .on_text(&format!(
                        "\n[Agent stopped: max turns ({}) reached]\n",
                        self.config.max_turns
                    ))
                    .await;
                return Ok(StopReason::MaxTurns);
            }

            // Guard: token budget
            if session.token_estimate >= self.config.token_budget {
                callbacks
                    .on_text(&format!(
                        "\n[Agent stopped: token budget ({}) exceeded]\n",
                        self.config.token_budget
                    ))
                    .await;
                return Ok(StopReason::TokenBudget);
            }

            session.increment_turn();

            // Send to LLM
            let response = self
                .provider
                .send_turn(&session.messages, &tool_defs, &turn_config)
                .await;

            let response = match response {
                Ok(r) => r,
                Err(e) => {
                    let msg = format!("Provider error: {e}");
                    callbacks.on_text(&format!("\n[{msg}]\n")).await;
                    return Ok(StopReason::ProviderError(msg));
                }
            };

            match response {
                TurnResponse::Text(text) => {
                    callbacks.on_text(&text).await;
                    session.add_message(Message::text(Role::Assistant, &text));
                    return Ok(StopReason::EndTurn);
                }
                TurnResponse::ToolCalls(tool_calls) => {
                    // Add assistant message with tool use blocks
                    let blocks: Vec<ContentBlock> = tool_calls
                        .iter()
                        .map(|tc| ContentBlock::ToolUse {
                            id: tc.id.clone(),
                            name: tc.name.clone(),
                            input: tc.input.clone(),
                        })
                        .collect();
                    session.add_message(Message {
                        role: Role::Assistant,
                        content: blocks,
                    });

                    // Execute each tool call
                    for tc in &tool_calls {
                        let stop = self
                            .execute_tool_call(&tc.id, &tc.name, &tc.input, session, callbacks)
                            .await?;
                        if let Some(reason) = stop {
                            return Ok(reason);
                        }
                    }
                    // Continue loop for next LLM turn
                }
                TurnResponse::Mixed { text, tool_calls } => {
                    callbacks.on_text(&text).await;

                    // Add assistant message with mixed content
                    let mut blocks = vec![ContentBlock::Text { text: text.clone() }];
                    blocks.extend(tool_calls.iter().map(|tc| ContentBlock::ToolUse {
                        id: tc.id.clone(),
                        name: tc.name.clone(),
                        input: tc.input.clone(),
                    }));
                    session.add_message(Message {
                        role: Role::Assistant,
                        content: blocks,
                    });

                    for tc in &tool_calls {
                        let stop = self
                            .execute_tool_call(&tc.id, &tc.name, &tc.input, session, callbacks)
                            .await?;
                        if let Some(reason) = stop {
                            return Ok(reason);
                        }
                    }
                }
            }
        }
    }

    async fn execute_tool_call(
        &self,
        id: &str,
        name: &str,
        input: &serde_json::Value,
        session: &mut Session,
        callbacks: &dyn EngineCallbacks,
    ) -> anyhow::Result<Option<StopReason>> {
        // Find the tool
        let tool = match self.tools.iter().find(|t| t.name() == name) {
            Some(t) => t,
            None => {
                let msg = format!("Unknown tool: {name}");
                session.add_message(Message::tool_result(id, &msg, true));
                return Ok(None);
            }
        };

        // Show what's about to happen BEFORE permission check
        // so the user sees the preview when deciding to approve
        callbacks.on_tool_start(name, input).await;

        // Determine the action for permission checking
        let action = infer_tool_action(name, input, &self.project_root);

        // Check permission
        let permission = self.permission_gate.check(name, &action);
        match permission {
            Permission::Allow => {}
            Permission::Deny(reason) => {
                let msg = format!("Permission denied: {reason}");
                callbacks.on_tool_end(name, &msg, true).await;
                session.add_message(Message::tool_result(id, &msg, true));
                return Ok(None);
            }
            Permission::Ask(prompt) => {
                let allowed = callbacks.ask_permission(&prompt).await;
                if !allowed {
                    let msg = "User denied permission";
                    callbacks.on_tool_end(name, msg, true).await;
                    session.add_message(Message::tool_result(id, msg, true));
                    return Ok(None);
                }
            }
        }

        // Execute (preview already shown above)

        let ctx = ToolContext {
            cwd: self.project_root.clone(),
            session_id: session.id.clone(),
            env: HashMap::new(),
        };

        let result = tool.execute(input.clone(), &ctx).await?;
        let is_error = result.is_error();
        let output = result.output();

        callbacks.on_tool_end(name, &output, is_error).await;

        match result {
            ToolResult::Success(s) => {
                session.add_message(Message::tool_result(id, &s, false));
            }
            ToolResult::Failure(s) => {
                session.add_message(Message::tool_result(id, &s, true));
            }
            ToolResult::Error(e) => {
                let msg = format!("Tool error: {e}");
                session.add_message(Message::tool_result(id, &msg, true));
                return Ok(Some(StopReason::ToolError(msg)));
            }
        }

        Ok(None)
    }
}

/// Infer the ToolAction from tool name and input for permission checking
fn infer_tool_action(
    name: &str,
    input: &serde_json::Value,
    project_root: &std::path::Path,
) -> ToolAction {
    match name {
        "read_file" => {
            let path_str = input.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let path = if PathBuf::from(path_str).is_absolute() {
                PathBuf::from(path_str)
            } else {
                project_root.join(path_str)
            };
            ToolAction::FileRead(path)
        }
        "write_file" => {
            let path_str = input.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let path = if PathBuf::from(path_str).is_absolute() {
                PathBuf::from(path_str)
            } else {
                project_root.join(path_str)
            };
            ToolAction::FileWrite(path)
        }
        "bash" => {
            let cmd = input
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            ToolAction::BashExec(cmd)
        }
        "glob" | "grep" => {
            // Read-only search tools, resolve path relative to project
            let path_str = input.get("path").and_then(|v| v.as_str()).unwrap_or(".");
            let path = if PathBuf::from(path_str).is_absolute() {
                PathBuf::from(path_str)
            } else {
                project_root.join(path_str)
            };
            ToolAction::FileRead(path)
        }
        _ => ToolAction::NetworkRequest(format!("unknown tool: {name}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use unripe_core::permission::AutoApproveGate;
    use unripe_core::tool::ToolDefinition;

    // Mock provider that returns predetermined responses
    struct MockProvider {
        responses: Mutex<Vec<TurnResponse>>,
    }

    impl MockProvider {
        fn new(responses: Vec<TurnResponse>) -> Self {
            Self {
                responses: Mutex::new(responses),
            }
        }
    }

    #[async_trait::async_trait]
    impl LlmProvider for MockProvider {
        fn name(&self) -> &str {
            "mock"
        }

        async fn send_turn(
            &self,
            _messages: &[Message],
            _tools: &[ToolDefinition],
            _config: &TurnConfig,
        ) -> anyhow::Result<TurnResponse> {
            let mut responses = self.responses.lock().unwrap();
            if responses.is_empty() {
                Ok(TurnResponse::Text("(no more responses)".into()))
            } else {
                Ok(responses.remove(0))
            }
        }

        async fn stream_turn(
            &self,
            _messages: &[Message],
            _tools: &[ToolDefinition],
            _config: &TurnConfig,
        ) -> anyhow::Result<
            std::pin::Pin<
                Box<dyn futures::Stream<Item = unripe_core::provider::StreamEvent> + Send>,
            >,
        > {
            unimplemented!("mock does not support streaming")
        }
    }

    // Mock callbacks that collect output
    struct TestCallbacks {
        texts: Arc<Mutex<Vec<String>>>,
        tool_starts: Arc<Mutex<Vec<String>>>,
        tool_ends: Arc<Mutex<Vec<(String, bool)>>>,
    }

    impl TestCallbacks {
        fn new() -> Self {
            Self {
                texts: Arc::new(Mutex::new(Vec::new())),
                tool_starts: Arc::new(Mutex::new(Vec::new())),
                tool_ends: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    #[async_trait::async_trait]
    impl EngineCallbacks for TestCallbacks {
        async fn ask_permission(&self, _prompt: &str) -> bool {
            true // auto-approve in tests
        }
        async fn on_text(&self, text: &str) {
            self.texts.lock().unwrap().push(text.to_string());
        }
        async fn on_tool_start(&self, name: &str, _input: &serde_json::Value) {
            self.tool_starts.lock().unwrap().push(name.to_string());
        }
        async fn on_tool_end(&self, name: &str, _result: &str, is_error: bool) {
            self.tool_ends
                .lock()
                .unwrap()
                .push((name.to_string(), is_error));
        }
    }

    fn test_engine(responses: Vec<TurnResponse>, tools: Vec<Box<dyn Tool>>) -> AgentEngine {
        let dir = std::env::temp_dir().join("unripe-test-engine");
        std::fs::create_dir_all(&dir).unwrap();

        AgentEngine::new(
            Box::new(MockProvider::new(responses)),
            tools,
            Box::new(AutoApproveGate),
            AgentConfig::default(),
            dir,
        )
    }

    #[tokio::test]
    async fn test_single_turn_text_response() {
        let engine = test_engine(
            vec![TurnResponse::Text("Hello! I can help.".into())],
            vec![],
        );
        let mut session = Session::new("mock", "test");
        let cb = TestCallbacks::new();

        let reason = engine.run("Hi", &mut session, &cb).await.unwrap();

        assert_eq!(reason, StopReason::EndTurn);
        let texts = cb.texts.lock().unwrap();
        assert!(texts.iter().any(|t| t.contains("Hello")));
    }

    #[tokio::test]
    async fn test_tool_call_then_text() {
        let engine = test_engine(
            vec![
                TurnResponse::ToolCalls(vec![unripe_core::tool::ToolCall {
                    id: "call_1".into(),
                    name: "bash".into(),
                    input: serde_json::json!({"command": "echo hi"}),
                }]),
                TurnResponse::Text("The command output 'hi'.".into()),
            ],
            vec![Box::new(unripe_tools::BashTool::new(5))],
        );
        let mut session = Session::new("mock", "test");
        let cb = TestCallbacks::new();

        let reason = engine.run("Run echo", &mut session, &cb).await.unwrap();

        assert_eq!(reason, StopReason::EndTurn);
        let starts = cb.tool_starts.lock().unwrap();
        assert_eq!(starts[0], "bash");
        let ends = cb.tool_ends.lock().unwrap();
        assert!(!ends[0].1); // not an error
    }

    #[tokio::test]
    async fn test_max_turns_guard() {
        // Provider always returns tool calls, never text
        let mut responses = Vec::new();
        for i in 0..30 {
            responses.push(TurnResponse::ToolCalls(vec![unripe_core::tool::ToolCall {
                id: format!("call_{i}"),
                name: "bash".into(),
                input: serde_json::json!({"command": "echo turn"}),
            }]));
        }

        let mut config = AgentConfig::default();
        config.max_turns = 3;

        let dir = std::env::temp_dir().join("unripe-test-engine-max");
        std::fs::create_dir_all(&dir).unwrap();

        let engine = AgentEngine::new(
            Box::new(MockProvider::new(responses)),
            vec![Box::new(unripe_tools::BashTool::new(5))],
            Box::new(AutoApproveGate),
            config,
            dir,
        );
        let mut session = Session::new("mock", "test");
        let cb = TestCallbacks::new();

        let reason = engine.run("loop", &mut session, &cb).await.unwrap();
        assert_eq!(reason, StopReason::MaxTurns);
    }

    #[tokio::test]
    async fn test_permission_deny() {
        use unripe_core::permission::AutoDenyGate;

        let dir = std::env::temp_dir().join("unripe-test-engine-deny");
        std::fs::create_dir_all(&dir).unwrap();

        let engine = AgentEngine::new(
            Box::new(MockProvider::new(vec![
                TurnResponse::ToolCalls(vec![unripe_core::tool::ToolCall {
                    id: "call_1".into(),
                    name: "bash".into(),
                    input: serde_json::json!({"command": "rm -rf /"}),
                }]),
                TurnResponse::Text("OK, I won't do that.".into()),
            ])),
            vec![Box::new(unripe_tools::BashTool::new(5))],
            Box::new(AutoDenyGate),
            AgentConfig::default(),
            dir,
        );
        let mut session = Session::new("mock", "test");
        let cb = TestCallbacks::new();

        let reason = engine
            .run("delete everything", &mut session, &cb)
            .await
            .unwrap();
        assert_eq!(reason, StopReason::EndTurn);

        // Tool preview was shown (on_tool_start) but execution was denied
        let starts = cb.tool_starts.lock().unwrap();
        assert_eq!(starts.len(), 1); // preview shown before permission check
        let ends = cb.tool_ends.lock().unwrap();
        assert!(ends[0].1); // is_error = true (denied)
    }

    #[tokio::test]
    async fn test_unknown_tool() {
        let engine = test_engine(
            vec![
                TurnResponse::ToolCalls(vec![unripe_core::tool::ToolCall {
                    id: "call_1".into(),
                    name: "nonexistent_tool".into(),
                    input: serde_json::json!({}),
                }]),
                TurnResponse::Text("I don't have that tool.".into()),
            ],
            vec![],
        );
        let mut session = Session::new("mock", "test");
        let cb = TestCallbacks::new();

        let reason = engine
            .run("use fake tool", &mut session, &cb)
            .await
            .unwrap();
        assert_eq!(reason, StopReason::EndTurn);

        // Should have added a tool result with error
        let tool_msgs: Vec<_> = session
            .messages
            .iter()
            .filter(|m| m.role == Role::Tool)
            .collect();
        assert!(!tool_msgs.is_empty());
    }

    #[tokio::test]
    async fn test_mixed_response() {
        let engine = test_engine(
            vec![
                TurnResponse::Mixed {
                    text: "Let me read that file.".into(),
                    tool_calls: vec![unripe_core::tool::ToolCall {
                        id: "call_1".into(),
                        name: "bash".into(),
                        input: serde_json::json!({"command": "echo content"}),
                    }],
                },
                TurnResponse::Text("Here's what I found.".into()),
            ],
            vec![Box::new(unripe_tools::BashTool::new(5))],
        );
        let mut session = Session::new("mock", "test");
        let cb = TestCallbacks::new();

        let reason = engine.run("read", &mut session, &cb).await.unwrap();
        assert_eq!(reason, StopReason::EndTurn);

        let texts = cb.texts.lock().unwrap();
        assert!(texts.iter().any(|t| t.contains("Let me read")));
    }

    #[test]
    fn test_infer_tool_action() {
        let root = PathBuf::from("/project");

        let action = infer_tool_action(
            "read_file",
            &serde_json::json!({"path": "src/main.rs"}),
            &root,
        );
        assert!(matches!(action, ToolAction::FileRead(_)));

        let action = infer_tool_action(
            "write_file",
            &serde_json::json!({"path": "out.txt", "content": "x"}),
            &root,
        );
        assert!(matches!(action, ToolAction::FileWrite(_)));

        let action = infer_tool_action("bash", &serde_json::json!({"command": "ls"}), &root);
        assert!(matches!(action, ToolAction::BashExec(_)));
    }

    #[tokio::test]
    async fn test_chat_only_mode_ignores_tools() {
        // In chat-only mode, even if tools are provided, they should not be sent to the LLM
        let dir = std::env::temp_dir().join("unripe-test-engine-chat");
        std::fs::create_dir_all(&dir).unwrap();

        let engine = AgentEngine::new(
            Box::new(MockProvider::new(vec![TurnResponse::Text(
                "I'm in chat mode!".into(),
            )])),
            vec![Box::new(unripe_tools::BashTool::new(5))], // tools provided but should be ignored
            Box::new(AutoApproveGate),
            AgentConfig::default(),
            dir,
        )
        .with_chat_only(true);

        let mut session = Session::new("mock", "test");
        let cb = TestCallbacks::new();

        let reason = engine.run("hello", &mut session, &cb).await.unwrap();
        assert_eq!(reason, StopReason::EndTurn);

        // No tools should have been called
        let starts = cb.tool_starts.lock().unwrap();
        assert!(starts.is_empty());

        let texts = cb.texts.lock().unwrap();
        assert!(texts.iter().any(|t| t.contains("chat mode")));
    }
}
