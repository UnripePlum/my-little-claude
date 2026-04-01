# unripe-claude

A model-agnostic coding agent harness written in Rust. Use any LLM — Anthropic Claude, local models via ollama, or bring your own provider.

unripe-claude is both a working coding agent **and** a crate ecosystem you can build on.

## Quick Start

```bash
# With Anthropic API
export ANTHROPIC_API_KEY=sk-...
cargo run --bin unripe -- "describe what this repo does"

# With local ollama model (no API key needed)
cargo run --bin unripe -- --provider ollama --model qwen2.5-coder:7b "fix the typo in main.rs"

# Resume previous session
cargo run --bin unripe -- --resume "continue where we left off"
```

## Install

```bash
cargo install unripe-cli
```

Or build from source:

```bash
git clone https://github.com/unripeplum/unripe-claude
cd unripe-claude
cargo build --release
# Binary at target/release/unripe
```

## Architecture

```
┌─────────────┐
│  unripe-cli │   CLI binary (clap)
└──────┬──────┘
       │
┌──────▼──────────┐
│  unripe-engine  │   Agent loop (ReAct pattern)
│                 │   max_turns + token_budget guards
│                 │   permission checking
│                 │   session management
└──┬──────────┬───┘
   │          │
┌──▼────┐  ┌─▼───────────┐
│ tools │  │  providers   │
│       │  │              │
│ read  │  │  anthropic   │
│ write │  │  ollama      │
│ bash  │  │  (your own)  │
└──┬────┘  └──┬───────────┘
   │          │
┌──▼──────────▼──┐
│   unripe-core  │   Traits: LlmProvider, Tool, PermissionGate
│                │   Types: Message, Session, Config
└────────────────┘
```

## Crates

| Crate | Description |
|-------|-------------|
| `unripe-core` | Core traits (`LlmProvider`, `Tool`, `PermissionGate`) and types |
| `unripe-providers` | Built-in LLM providers (Anthropic, ollama) |
| `unripe-tools` | Built-in tools (read_file, write_file, bash) |
| `unripe-engine` | Agent engine loop with ReAct pattern |
| `unripe-cli` | CLI binary |

## Adding Your Own Provider

Implement the `LlmProvider` trait:

```rust
use unripe_core::provider::{LlmProvider, TurnConfig, TurnResponse, StreamEvent};
use unripe_core::message::Message;
use unripe_core::tool::ToolDefinition;

#[async_trait::async_trait]
impl LlmProvider for MyProvider {
    fn name(&self) -> &str { "my-provider" }

    async fn send_turn(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
        config: &TurnConfig,
    ) -> anyhow::Result<TurnResponse> {
        // Your implementation here
    }

    async fn stream_turn(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
        config: &TurnConfig,
    ) -> anyhow::Result<Pin<Box<dyn Stream<Item = StreamEvent> + Send>>> {
        // Your streaming implementation here
    }
}
```

## Adding Your Own Tool

Implement the `Tool` trait:

```rust
use unripe_core::tool::{Tool, ToolContext, ToolResult};

#[async_trait::async_trait]
impl Tool for MyTool {
    fn name(&self) -> &str { "my_tool" }
    fn description(&self) -> &str { "Does something useful" }
    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": { "input": { "type": "string" } },
            "required": ["input"]
        })
    }

    async fn execute(
        &self,
        input: serde_json::Value,
        ctx: &ToolContext,
    ) -> anyhow::Result<ToolResult> {
        Ok(ToolResult::Success("done".into()))
    }
}
```

## Permission System

unripe-claude has a tiered permission system:

| Action | Inside project | Outside project |
|--------|---------------|-----------------|
| `read_file` | Allow | Ask |
| `write_file` | Ask | Deny |
| `bash` | Ask | Ask |

Implement `PermissionGate` for custom policies.

## Configuration

Config lives at `~/.unripe/config.toml`:

```toml
[agent]
max_turns = 25
token_budget = 100000
bash_timeout_secs = 30

[provider]
default_provider = "ollama"
default_model = "qwen2.5-coder:7b"

[provider.anthropic]
api_key_env = "ANTHROPIC_API_KEY"

[provider.ollama]
base_url = "http://localhost:11434"
```

## Session Management

Sessions are saved to `~/.unripe/sessions/`. Use `--resume` to continue the last conversation.

## Safety

- All bash commands require explicit approval
- File writes require approval
- Reads outside the project directory require approval
- 30-second timeout on bash commands (configurable)
- max_turns guard prevents infinite tool-call loops
- token_budget guard prevents runaway API costs

## Roadmap

### v0.1.1
- `unripe setup` — auto-detect hardware, recommend and download local models

### v0.2
- Tower-style middleware pipeline (the "Axum of agents")
- Deterministic session replay
- MCP client support
- OpenAI provider
- `glob`/`grep` tools
- Rich TUI with ratatui
- Tool-less model fallback (chat-only mode)

## License

MIT OR Apache-2.0
