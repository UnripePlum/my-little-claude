<div align="center">

<h1>my-little-claude</h1>

<p><b>A model-agnostic coding agent harness in Rust.<br/>Plug any LLM. Run locally. Own your agent.</b></p>

<p>
  <a href="#features"><strong>Features</strong></a> ·
  <a href="#quick-start"><strong>Quick Start</strong></a> ·
  <a href="#architecture"><strong>Architecture</strong></a> ·
  <a href="#extend"><strong>Extend</strong></a> ·
  <a href="#roadmap"><strong>Roadmap</strong></a>
</p>

<p>

[![Rust](https://img.shields.io/badge/Rust-2021-orange?logo=rust)](https://www.rust-lang.org)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue)](LICENSE)
[![CI](https://img.shields.io/github/actions/workflow/status/UnripePlum/my-little-claude/ci.yml?label=CI)](https://github.com/UnripePlum/my-little-claude/actions)

</p>

</div>

---

my-little-claude is both a **working coding agent** and a **crate ecosystem** for building your own.

Swap between Anthropic Claude and local ollama models with a flag. Add custom providers, tools, or permission policies by implementing a trait. The agent runs a streaming ReAct loop with built-in safety guards.

## Features

- **Model-agnostic** -- Anthropic Claude, local ollama, or implement `LlmProvider` for anything else
- **Streaming ReAct loop** -- tool calls and text stream in real-time with `stream_turn()`
- **Tiered permission system** -- reads inside project auto-allowed, writes need approval, bash always asks
- **Session persistence** -- conversations save to disk, resume with `--resume`
- **Safety guards** -- max_turns (25), token_budget (100K), bash timeout (30s), all configurable
- **Local-first** -- run with ollama, no API key, no internet required
- **Extensible** -- custom providers, tools, and permission gates are one trait impl away

## Quick Start

```bash
# Clone and build
git clone https://github.com/UnripePlum/my-little-claude
cd my-little-claude
cargo build --release

# With Anthropic API
export ANTHROPIC_API_KEY=sk-ant-...
./target/release/unripe "describe what this repo does"

# With local ollama (no API key needed)
./target/release/unripe --provider ollama --model qwen2.5-coder:7b "fix the typo in main.rs"

# Resume previous session
./target/release/unripe --resume "continue where we left off"
```

Or install directly:

```bash
cargo install unripe-cli
unripe "hello"
```

## Architecture

```
                          ┌─────────────┐
                          │  unripe-cli │
                          │   (clap)    │
                          └──────┬──────┘
                                 │
                          ┌──────▼──────────┐
                          │  unripe-engine  │
                          │                 │
                          │  ReAct Loop:    │
                          │  prompt         │
                          │   → LLM call    │
                          │   → tool use?   │
                          │     → permit?   │
                          │     → execute   │
                          │   → repeat      │
                          │   → stream text │
                          └──┬──────────┬───┘
                             │          │
                     ┌───────▼──┐  ┌───▼───────────┐
                     │  tools   │  │   providers    │
                     │          │  │                │
                     │ read_file│  │  anthropic     │
                     │ write    │  │  ollama        │
                     │ bash     │  │  (your own)    │
                     └────┬─────┘  └────┬───────────┘
                          │             │
                     ┌────▼─────────────▼────┐
                     │      unripe-core      │
                     │                       │
                     │  LlmProvider  trait    │
                     │  Tool         trait    │
                     │  PermissionGate trait  │
                     │  Message, Session,     │
                     │  Config types          │
                     └───────────────────────┘
```

### Crate Map

| Crate | What it does |
|-------|-------------|
| **unripe-core** | Traits (`LlmProvider`, `Tool`, `PermissionGate`) and shared types |
| **unripe-engine** | Agent loop -- bootstrap, ReAct cycle, session truncation, guards |
| **unripe-providers** | Anthropic Messages API (streaming SSE) + ollama (OpenAI-compat) |
| **unripe-tools** | `read_file`, `write_file`, `bash` with timeout and error handling |
| **unripe-cli** | Binary entry point with clap, permission prompts, colored output |

## How It Works

```
User: "fix the bug in main.rs"
  │
  ▼
Bootstrap
  Load CLAUDE.md, AGENTS.md, git branch → system prompt
  │
  ▼
Agent Loop (max 25 turns)
  │
  ├─▶ Send messages + tool definitions to LLM
  │
  ├─▶ LLM returns tool_use(read_file, {path: "main.rs"})
  │     │
  │     ├─ PermissionGate: FileRead inside project → Allow
  │     ├─ Execute read_file → Success("fn main() { ... }")
  │     └─ Append tool result to messages
  │
  ├─▶ LLM returns tool_use(write_file, {path: "main.rs", content: "..."})
  │     │
  │     ├─ PermissionGate: FileWrite → Ask
  │     ├─ Terminal: "[Permission] Write file: main.rs [y/N]"
  │     ├─ User types: y
  │     ├─ Execute write_file → Success("Written 142 bytes")
  │     └─ Append tool result to messages
  │
  └─▶ LLM returns text("Fixed the bug. The issue was...")
        │
        └─ Stream to terminal, save session, done.
```

## Permission System

| Action | Inside project | Outside project |
|--------|:---:|:---:|
| `read_file` | Allow | Ask |
| `write_file` | Ask | **Deny** |
| `bash` | Ask | Ask |

Implement `PermissionGate` for custom policies:

```rust
use unripe_core::permission::{PermissionGate, Permission, ToolAction};

struct MyGate;
impl PermissionGate for MyGate {
    fn check(&self, tool_name: &str, action: &ToolAction) -> Permission {
        match action {
            ToolAction::BashExec(cmd) if cmd.contains("rm") => {
                Permission::Deny("no delete commands".into())
            }
            _ => Permission::Allow,
        }
    }
}
```

## Configuration

`~/.unripe/config.toml`:

```toml
[agent]
max_turns = 25           # Stop after N turns
token_budget = 100000    # Stop after ~N tokens
bash_timeout_secs = 30   # Kill bash after N seconds
context_files = []       # Extra files to load as context

[provider]
default_provider = "ollama"
default_model = "qwen2.5-coder:7b"

[provider.anthropic]
api_key_env = "ANTHROPIC_API_KEY"

[provider.ollama]
base_url = "http://localhost:11434"
```

## <a name="extend"></a>Extend

### Add a Provider

```rust
use unripe_core::provider::{LlmProvider, TurnConfig, TurnResponse, StreamEvent};
use unripe_core::message::Message;
use unripe_core::tool::ToolDefinition;

#[async_trait::async_trait]
impl LlmProvider for MyProvider {
    fn name(&self) -> &str { "my-provider" }

    async fn send_turn(
        &self, messages: &[Message], tools: &[ToolDefinition], config: &TurnConfig,
    ) -> anyhow::Result<TurnResponse> {
        // call your LLM API, return TurnResponse::Text or TurnResponse::ToolCalls
        todo!()
    }

    async fn stream_turn(
        &self, messages: &[Message], tools: &[ToolDefinition], config: &TurnConfig,
    ) -> anyhow::Result<Pin<Box<dyn Stream<Item = StreamEvent> + Send>>> {
        // yield StreamEvent::TextDelta, ToolCallStart, etc.
        todo!()
    }
}
```

### Add a Tool

```rust
use unripe_core::tool::{Tool, ToolContext, ToolResult};

#[async_trait::async_trait]
impl Tool for MyTool {
    fn name(&self) -> &str { "grep" }
    fn description(&self) -> &str { "Search files for a pattern" }
    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string" },
                "path": { "type": "string" }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> anyhow::Result<ToolResult> {
        let pattern = input["pattern"].as_str().unwrap_or("");
        // ... run grep logic ...
        Ok(ToolResult::Success("matching lines here".into()))
    }
}
```

## Safety

| Guard | Default | Configurable |
|-------|---------|:---:|
| Max conversation turns | 25 | `config.toml` |
| Token budget | 100,000 | `config.toml` |
| Bash timeout | 30s | `config.toml` |
| Write outside project | **Blocked** | `PermissionGate` |
| Bash execution | **Requires approval** | `PermissionGate` |
| Session truncation | Keep last 10 messages | `config.toml` |
| Ctrl+C | Kills child processes, exits | -- |

## Testing

```bash
cargo test --workspace        # Run all 93 tests
cargo test -p unripe-core     # Core traits and types (41 tests)
cargo test -p unripe-engine   # Engine loop (12 tests)
cargo test -p unripe-providers # Anthropic + ollama (23 tests)
cargo test -p unripe-tools    # read/write/bash tools (17 tests)
```

## Roadmap

### v0.1.1

- `unripe setup` -- detect hardware specs, recommend a model, download via ollama

### v0.2

- Tower-style middleware pipeline -- composable agent behaviors, "the Axum of agents"
- Deterministic session replay -- record and replay agent sessions with different models
- MCP client support
- OpenAI provider
- `glob` / `grep` tools
- Rich TUI with ratatui
- Chat-only fallback for models without tool calling

## License

[MIT](LICENSE) OR Apache-2.0
