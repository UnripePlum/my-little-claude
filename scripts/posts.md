# Community Posts for my-little-claude

## Hacker News (Show HN)

**Title:** Show HN: my-little-claude, an open-source coding agent in Rust that works with any LLM

**Body:**

I built a model-agnostic coding agent harness in Rust. It works with Anthropic Claude, OpenAI, and local models via ollama. No vendor lock-in.

What it does:
- 3 LLM providers (Anthropic, OpenAI, ollama) behind a single `LlmProvider` trait
- 5 built-in tools (read_file, write_file, bash, glob, grep) with a tiered permission system
- MCP client support (reads .mcp.json, same format as Claude Code)
- `unripe setup` auto-detects your hardware and recommends the best local model from a catalog of 27 models
- Session persistence and replay (run the same prompts through different models)
- Chat-only fallback for models without tool calling support

The architecture is a workspace of 7 Rust crates. Each component (core traits, providers, tools, engine, CLI) is a separate crate you can depend on independently.

156 tests, clippy clean, pre-commit hooks.

GitHub: https://github.com/UnripePlum/my-little-claude

Built in a day with Claude Code. Happy to answer questions about the architecture or Rust patterns used.

---

## Reddit r/rust

**Title:** [Media] I built a model-agnostic coding agent in Rust: my-little-claude

**Body:**

Hey r/rust! I built an open-source coding agent that works with any LLM provider (Anthropic, OpenAI, local models via ollama).

**Why Rust?** Single binary distribution, async with tokio, trait-based provider/tool abstraction, and zero unsafe.

**Architecture:**
- 7 crates in a cargo workspace
- `LlmProvider` trait with 3 implementations (Anthropic SSE streaming, OpenAI chat completions, ollama)
- `Tool` trait with 5 built-in tools + MCP bridge for external tools
- `PermissionGate` trait with tiered security defaults
- Shared SSE parser using custom `Stream` implementation
- 156 tests, all passing

**Cool Rust patterns used:**
- `async_trait` for provider/tool abstraction (needed for `dyn Trait`)
- `stream::unfold` and custom `Stream` impl for SSE parsing
- `include_str!` for embedding the model catalog JSON at compile time
- `serde` tagged enums for Anthropic content blocks
- `tokio::process` with `kill_on_drop` for bash tool timeout

**Features:**
- `unripe setup --list` shows 27 local models with hardware compatibility
- MCP client reads `.mcp.json` (Claude Code compatible)
- Session replay: `unripe replay <id> --provider ollama --model qwen3.5:9b`
- Auto-detect chat-only mode for models without tool calling

GitHub: https://github.com/UnripePlum/my-little-claude

Feedback welcome, especially on the trait design and async patterns!

---

## Reddit r/LocalLLaMA

**Title:** Built an open-source coding agent that auto-detects your hardware and recommends the best local model

**Body:**

I built my-little-claude, a coding agent in Rust that's designed for local-first use with ollama.

**The setup experience:**

```
$ unripe setup --list

  System: RAM: 16.0GB | CPU: 10x aarch64 | GPU: Apple M2 Pro | Tier: Medium

  Available models (27 total):

  [coding]
    [T] devstral-small-2:24b    Mistral coding agent
    [T] qwen3-coder:30b-a3b     Alibaba coding MoE

  [general]
    [T] qwen3.5:9b              Good balance
    [T] qwen3.5:4b              Fast and lightweight

  [reasoning]
    [T] nemotron-cascade-2:30b  NVIDIA MoE
```

It detects your RAM/GPU, classifies your system, and recommends the best model. Then downloads it via `ollama pull`.

**Key features for local model users:**
- Auto-detects hardware and recommends models by category (coding/general/reasoning)
- Models without tool calling support auto-switch to chat-only mode
- Works completely offline, no API key needed
- Session replay lets you compare results across models
- MCP plugin support

GitHub: https://github.com/UnripePlum/my-little-claude

What local models are you running for coding tasks? Would love to update the model catalog based on community feedback.

---

## awesome-rust-llm PR description

**Title:** Add my-little-claude - model-agnostic coding agent harness

**Entry:**

- [my-little-claude](https://github.com/UnripePlum/my-little-claude) - A model-agnostic coding agent harness. 3 LLM providers (Anthropic, OpenAI, ollama), 5 built-in tools, MCP client support, tiered permission system, session replay. 7-crate workspace, 156 tests.
