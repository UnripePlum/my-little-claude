# Community Posts — my-little-claude

---

## Hacker News (Show HN)

**Title:** Show HN: I built an open-source Claude Code clone in Rust that runs on any LLM

**Body:**

I got tired of paying for Claude Code and being locked into one provider. So I built my own.

my-little-claude is a coding agent in Rust that works with Anthropic, OpenAI, or any local model via ollama. No API key needed. It auto-detects your hardware and picks the best model for your machine.

What made me build this: I wanted to use local models for coding tasks but every agent was either Python (slow to start, heavy dependencies) or locked to one provider. Rust gives me a 5MB binary that starts instantly.

It reads your code, finds bugs, fixes them, and asks permission before writing. Like Claude Code, but you own it.

Demo GIF: https://github.com/UnripePlum/my-little-claude

What I shipped:
- 3 LLM providers behind one trait (swap with a flag)
- MCP support (same .mcp.json as Claude Code)
- 27 local models categorized by coding/general/reasoning
- Permission system that actually asks before writing
- 156 tests, 7 Rust crates

Built the whole thing in one day with Claude Code. The irony is not lost on me.

GitHub: https://github.com/UnripePlum/my-little-claude

---

## Reddit r/rust

**Title:** I replaced Claude Code with 5MB of Rust. Here's the architecture.

**Body:**

Claude Code is great but it's $20/month and you can't use your own models. I built an open-source alternative in Rust.

**The result: 5MB binary, 156 tests, works with any LLM.**

Here's what was interesting from an engineering perspective:

**The trait design:**
```rust
#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn send_turn(&self, messages: &[Message], tools: &[ToolDefinition], config: &TurnConfig) -> Result<TurnResponse>;
    async fn stream_turn(...) -> Result<Pin<Box<dyn Stream<Item = StreamEvent> + Send>>>;
}
```
One trait, three implementations (Anthropic SSE, OpenAI chat completions, ollama). Swap provider with `--provider ollama`. The abstraction held up perfectly across all three.

**The hard part:** Anthropic and OpenAI have completely different wire formats for tool calls. Anthropic uses nested content blocks with `tool_use_id` echo-back. OpenAI uses `tool_calls` array with stringified JSON arguments. ollama expects `arguments` as a JSON object, not a string. Each provider converts internally.

**What I'm proud of:**
- Custom `Stream` impl for SSE parsing (not just `unfold`)
- `include_str!` for embedding model catalog at compile time
- `kill_on_drop(true)` on bash child process + 30s timeout
- Permission gate is sync (policy decision), Ask handling is async (user input) — clean separation

**What I learned:** The `async_trait` crate is still needed in 2026 if you want `dyn Trait`. Native async traits work for static dispatch but not `Box<dyn LlmProvider>`.

7 crates, zero unsafe, tokio runtime. Demo: https://github.com/UnripePlum/my-little-claude

Would love feedback on the trait design. Is there a better way to handle the provider abstraction?

---

## Reddit r/LocalLLaMA

**Title:** I built a coding agent that works offline with ollama. No API key, no cloud, no telemetry. Just your code and your model.

**Body:**

Every coding agent I tried either required an API key or phoned home. I wanted something that runs 100% on my machine.

So I built my-little-claude. It's a Rust binary that:

1. **Detects your hardware** (RAM, GPU, CPU arch)
2. **Recommends a model** from 27 options categorized by use case
3. **Downloads it** via ollama
4. **Runs a coding agent** that reads files, finds bugs, and fixes them

The whole thing works offline after the initial model download.

**The killer feature for local model users:** If your model doesn't support tool calling (like llama3.2:3b), it auto-detects that and switches to chat-only mode. No crash, no cryptic error. Just works.

```
$ unripe setup --list

  System: RAM: 64.0GB | CPU: 18x aarch64 | GPU: Apple M5 Pro | Tier: High

  [coding]
    [T] devstral-small-2:24b    Mistral coding agent
    [T] qwen3-coder:30b-a3b     Alibaba coding MoE
  [general]
    [T] qwen3.5:9b              Good balance
  [reasoning]
    [T] nemotron-cascade-2:30b  NVIDIA MoE
```

**What models are you using for coding?** I'd love to add more to the catalog. Currently 27 models but I know I'm missing some good ones.

Also supports session replay — run the same prompts through different models and compare outputs. Good for benchmarking local models against each other.

GitHub: https://github.com/UnripePlum/my-little-claude

---

## X/Twitter

**Post 1 (main):**

I built an open-source Claude Code clone in Rust.

5MB binary. Works with any LLM. No API key needed.

- Anthropic, OpenAI, or ollama
- 27 local models, auto hardware detection
- MCP plugin support
- Permission system that asks before writing
- 156 tests

Built in one day.

github.com/UnripePlum/my-little-claude

[attach demo.gif]

**Post 2 (reply thread):**

The irony: I built a Claude Code replacement using Claude Code.

The whole thing — 7 Rust crates, 156 tests, 3 LLM providers, MCP client — was designed, implemented, reviewed, and tested in a single conversation.

AI building AI tools. We're in the recursion now.

---

## awesome-rust-llm PR

**Title:** Add my-little-claude — model-agnostic coding agent harness

**Entry:**

- [my-little-claude](https://github.com/UnripePlum/my-little-claude) — Open-source Claude Code alternative in Rust. 3 LLM providers (Anthropic, OpenAI, ollama), 5 tools, MCP client, tiered permissions, 27-model catalog with hardware auto-detection, session replay. 7 crates, 156 tests. Works offline with local models.
