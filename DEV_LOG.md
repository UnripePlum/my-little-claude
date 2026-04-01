# my-little-claude Development Log

## Day 1: unripe-core traits (2026-04-01)

### Implemented
- `message.rs`: Message, Role, ContentBlock (Text/ToolUse/ToolResult) with serde
- `tool.rs`: Tool trait, ToolDefinition, ToolCall, ToolContext, ToolResult (Success/Failure/Error)
- `permission.rs`: PermissionGate trait, DefaultPermissionGate (tiered policy), AutoApproveGate, AutoDenyGate
- `provider.rs`: LlmProvider trait (send_turn + stream_turn), TurnConfig, TurnResponse, StreamEvent
- `session.rs`: Session (with truncation), SessionStore (save/load/list/load_latest)
- `config.rs`: AgentConfig, UnripeConfig, ProviderConfig (toml serialization)

### Tests: 41 passed, 0 failed

### Errors Encountered
1. **Missing deps in Cargo.toml**: `dirs` and `toml` crates not listed in unripe-core deps. Fixed by adding them.
2. **macOS /private symlink**: `canonicalize()` returns `/private/var/...` but `temp_dir()` returns `/var/...`. Non-existent file paths couldn't be canonicalized, causing `starts_with` to fail. Fixed by canonicalizing the parent directory instead.

### Decisions Made
- Put Message in its own `message.rs` instead of `session.rs` — it grew large enough with ContentBlock enum
- Added `AutoApproveGate` and `AutoDenyGate` for testing — not in original design but needed for test ergonomics
- Used `(text.len() / 4)` as rough token estimate — good enough for guards, not precise
- Session truncation keeps system messages + last N non-system messages

## Day 2: Anthropic Provider (2026-04-01)

### Implemented
- `anthropic.rs`: Full Anthropic Messages API provider
  - `send_turn()`: non-streaming with content block round-tripping
  - `stream_turn()`: SSE parsing with unfold-based state machine
  - Wire type conversion: Message ↔ ApiMessage, ContentBlock ↔ ApiContentBlock
  - Tool definition serialization for API
  - Error handling: 401 (invalid key), 429 (rate limit), 5xx
  - SSE event parsing: text_delta, tool_call_start/delta/end, message_stop, ping

### Tests: 14 passed, 0 failed (55 total across workspace)

### Errors Encountered
1. **Missing `bytes` crate**: `reqwest::Response::bytes_stream()` returns `bytes::Bytes`, needed explicit dep
2. **Serde tagged enum conflict**: `#[serde(tag = "type")]` on `ApiContentBlock` auto-generates the `type` field, but I also had an explicit `r#type: String` field. Serde can't deserialize with both. Fixed by removing the explicit field.

### Decisions Made
- Tool results sent as `role: "user"` in Anthropic API — this matches Anthropic's expected format
- SSE parser uses `futures::stream::unfold` for stateful line buffering — cleaner than manual poll impl
- `ToolCallDelta` uses `idx_{index}` as id since Anthropic streams don't repeat the tool id in deltas — engine will need to track mapping
- Kept `parse_sse_event` as a standalone function (not method) for testability

## Day 3: Engine Loop + Tools (2026-04-01)

### Implemented
- **Tools:**
  - `read_file.rs`: reads files, handles not-found/binary/truncation (>100KB)
  - `write_file.rs`: writes files, creates parent dirs, handles permission errors
  - `bash.rs`: executes commands with configurable timeout, captures stdout+stderr, kills on timeout
  - `builtin_tools()` factory function
- **Engine:**
  - `bootstrap.rs`: loads CLAUDE.md, AGENTS.md, .unripe/context.md, custom files, git branch
  - `engine.rs`: full ReAct agent loop with max_turns guard, token_budget guard, permission checking, ToolResult 3-way handling
  - `EngineCallbacks` trait for decoupled UI (ask_permission, on_text, on_tool_start/end)
  - `infer_tool_action()` maps tool calls to ToolAction for permission gate

### Tests: 29 new (12 engine + 17 tools), 84 total across workspace, 0 failed

### Errors Encountered
1. **`child.wait_with_output()` consumes ownership**: Rust's `tokio::process::Child::wait_with_output` takes `self`, so we couldn't call `child.kill()` on timeout. Fixed by using `child.wait()` + reading stdout/stderr pipes separately, and adding `.kill_on_drop(true)`.

### Decisions Made
- Engine uses non-streaming `send_turn` for the loop, streaming only for final text output (simplifies tool call handling)
- `EngineCallbacks` trait instead of direct terminal IO — enables testing with mock callbacks and future TUI
- Unknown tools get an error tool_result sent to LLM (not a fatal error) — LLM can adapt
- `infer_tool_action` is a standalone function, not on the Tool trait — keeps Tool trait simple, action inference is engine's concern

## Day 4: CLI + Ollama Provider (2026-04-01)

### Implemented
- **Ollama provider** (`ollama.rs`): Full OpenAI-compatible /api/chat implementation
  - `send_turn()`: non-streaming with tool call parsing
  - `stream_turn()`: NDJSON line-by-line streaming
  - Auto-generated UUIDs for tool calls when ollama doesn't provide ids
  - Connection error detection ("ollama not running") and model-not-found detection
- **CLI binary** (`main.rs`):
  - clap-based CLI with --provider, --model, --resume flags
  - `TerminalCallbacks`: colored permission prompts, streaming text output, tool execution display
  - Provider factory: builds Anthropic or Ollama from config + flags
  - Session persistence: auto-save on completion, --resume loads latest
  - Ctrl+C handler with graceful exit
  - Config loading from ~/.unripe/config.toml

### Tests: 9 new ollama tests, 93 total across workspace, 0 failed

### Errors Encountered
1. **`OllamaToolCall.id` is `Option<String>`**: In `to_api_messages`, passed `String` instead of `Some(String)`. Quick fix.
2. **CLI missing `async-trait` and `serde_json` deps**: `EngineCallbacks` uses `#[async_trait]` which wasn't available in the CLI crate.
3. **Unused variable warnings**: `cargo fix` auto-applied underscore prefixes to unused vars.

### Decisions Made
- Ollama streaming uses NDJSON (one JSON object per line) not SSE — different from Anthropic
- CLI uses stderr for tool execution status and permission prompts, stdout only for LLM text output — enables piping
- Provider factory in CLI, not in core — CLI owns the provider construction logic
- Ctrl+C handler calls `process::exit(130)` — can't easily save session from the signal handler, but `kill_on_drop(true)` on bash child processes ensures cleanup

## Day 5: README + CI + Ship (2026-04-01)

### Implemented
- `README.md`: Full docs with architecture diagram, quick start, crate table, provider/tool extension examples, permission system, config, safety, roadmap
- `LICENSE`: MIT
- `.gitignore`: target/, .omc/, .DS_Store
- `.github/workflows/ci.yml`: check + test + clippy + fmt on ubuntu-latest

### Final Stats
- **93 tests passed, 0 failed** across 5 crates
  - unripe-core: 41 tests
  - unripe-engine: 12 tests
  - unripe-providers: 23 tests (14 anthropic + 9 ollama)
  - unripe-tools: 17 tests
- **cargo fmt: clean**
- **cargo clippy: 1 cosmetic warning** (filter_map → map suggestion)
- **CLI binary works**: `unripe --help` shows correct output

### Total Errors Encountered Across 5 Days
1. Missing `dirs`/`toml` deps in unripe-core Cargo.toml
2. macOS `/private` symlink breaking `canonicalize()` path comparison
3. Serde `#[serde(tag = "type")]` + explicit `r#type` field conflict
4. Missing `bytes` crate for reqwest streaming
5. `child.wait_with_output()` consumes ownership, can't kill on timeout
6. `OllamaToolCall.id` is `Option<String>`, not `String`
7. CLI missing `async-trait` and `serde_json` deps
8. Formatting diffs caught by `cargo fmt --check`

### All Decisions Made (no user intervention)
- Day 1: Message in separate file, AutoApprove/AutoDeny gates for testing, rough token estimate
- Day 2: Tool results as user role for Anthropic, unfold-based SSE parser, standalone parse functions
- Day 3: Non-streaming for loop, EngineCallbacks trait, unknown tools get error result not fatal
- Day 4: NDJSON for ollama streaming, stderr for status/stdout for text, factory pattern for providers
- Day 5: MIT license, single CI job for speed, architecture ASCII art in README
