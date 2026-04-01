use std::io::Write;
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use unripe_core::config::UnripeConfig;
use unripe_core::permission::DefaultPermissionGate;
use unripe_core::provider::LlmProvider;
use unripe_core::session::{Session, SessionStore};
use unripe_engine::engine::{AgentEngine, EngineCallbacks};

#[derive(Parser)]
#[command(
    name = "unripe",
    version,
    about = "my-little-claude -- a model-agnostic coding agent"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// The prompt to send to the agent (shorthand for `unripe run "prompt"`)
    #[arg(global = false)]
    prompt: Option<String>,

    /// LLM provider to use (anthropic, ollama)
    #[arg(long)]
    provider: Option<String>,

    /// Model name to use
    #[arg(long)]
    model: Option<String>,

    /// Resume the most recent session
    #[arg(long)]
    resume: bool,

    /// Chat-only mode (no tool calling, just conversation)
    #[arg(long)]
    chat: bool,

    /// Non-interactive mode: output only final text, no colors, no tool previews.
    /// Writes and bash auto-denied unless --yes is also set.
    #[arg(long)]
    print: bool,

    /// Auto-approve all permission prompts (use with --print for CI)
    #[arg(long)]
    yes: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Replay a saved session with a different model
    Replay {
        /// Session ID to replay (use 'list' to see available sessions)
        session_id: String,

        /// LLM provider for replay
        #[arg(long)]
        provider: Option<String>,

        /// Model for replay
        #[arg(long)]
        model: Option<String>,
    },

    /// List saved sessions
    Sessions,

    /// Detect hardware and set up a local model
    Setup {
        /// Performance preference: high, medium, light
        #[arg(long, default_value = "medium")]
        performance: String,

        /// Model category: coding, general, reasoning
        #[arg(long, default_value = "coding")]
        category: String,

        /// Install a specific model by name (e.g. qwen3.5:9b)
        #[arg(long)]
        install: Option<String>,

        /// List all available models grouped by category
        #[arg(long)]
        list: bool,

        /// Skip interactive prompts, auto-accept recommendations
        #[arg(long)]
        yes: bool,
    },
}

struct TerminalCallbacks;

#[async_trait::async_trait]
impl EngineCallbacks for TerminalCallbacks {
    async fn ask_permission(&self, prompt: &str) -> bool {
        eprint!("\x1b[33m[Permission] {prompt}\x1b[0m [y/N] ");
        std::io::stderr().flush().ok();
        let mut input = String::new();
        if std::io::stdin().read_line(&mut input).is_ok() {
            let trimmed = input.trim().to_lowercase();
            trimmed == "y" || trimmed == "yes"
        } else {
            false
        }
    }

    async fn on_text(&self, text: &str) {
        print!("{text}");
        std::io::stdout().flush().ok();
    }

    async fn on_tool_start(&self, tool_name: &str, input: &serde_json::Value) {
        match tool_name {
            "read_file" => {
                let path = input.get("path").and_then(|v| v.as_str()).unwrap_or("?");
                eprintln!("\x1b[36m  ╭─ read \x1b[1m{path}\x1b[0m");
            }
            "write_file" => {
                let path = input.get("path").and_then(|v| v.as_str()).unwrap_or("?");
                eprintln!("\x1b[33m  ╭─ write \x1b[1m{path}\x1b[0m");
                // Show a preview of what's being written
                if let Some(content) = input.get("content").and_then(|v| v.as_str()) {
                    let lines: Vec<&str> = content.lines().collect();
                    let preview = if lines.len() > 8 {
                        let head: Vec<&str> = lines[..4].to_vec();
                        let tail: Vec<&str> = lines[lines.len() - 2..].to_vec();
                        format!(
                            "{}\n\x1b[90m  │  ... ({} more lines)\x1b[0m\n{}",
                            head.iter()
                                .map(|l| format!("\x1b[32m  │ +{l}\x1b[0m"))
                                .collect::<Vec<_>>()
                                .join("\n"),
                            lines.len() - 6,
                            tail.iter()
                                .map(|l| format!("\x1b[32m  │ +{l}\x1b[0m"))
                                .collect::<Vec<_>>()
                                .join("\n")
                        )
                    } else {
                        lines
                            .iter()
                            .map(|l| format!("\x1b[32m  │ +{l}\x1b[0m"))
                            .collect::<Vec<_>>()
                            .join("\n")
                    };
                    eprintln!("{preview}");
                }
            }
            "bash" => {
                let cmd = input.get("command").and_then(|v| v.as_str()).unwrap_or("?");
                eprintln!("\x1b[35m  ╭─ bash\x1b[0m");
                eprintln!("\x1b[35m  │ $ {cmd}\x1b[0m");
            }
            "edit_file" => {
                let path = input.get("path").and_then(|v| v.as_str()).unwrap_or("?");
                eprintln!("\x1b[33m  ╭─ edit \x1b[1m{path}\x1b[0m");
                if let (Some(old), Some(new)) = (
                    input.get("old_string").and_then(|v| v.as_str()),
                    input.get("new_string").and_then(|v| v.as_str()),
                ) {
                    for line in old.lines().take(5) {
                        eprintln!("\x1b[31m  │ -{line}\x1b[0m");
                    }
                    if old.lines().count() > 5 {
                        eprintln!(
                            "\x1b[90m  │ ... ({} more lines)\x1b[0m",
                            old.lines().count() - 5
                        );
                    }
                    for line in new.lines().take(5) {
                        eprintln!("\x1b[32m  │ +{line}\x1b[0m");
                    }
                    if new.lines().count() > 5 {
                        eprintln!(
                            "\x1b[90m  │ ... ({} more lines)\x1b[0m",
                            new.lines().count() - 5
                        );
                    }
                }
            }
            "glob" => {
                let pattern = input.get("pattern").and_then(|v| v.as_str()).unwrap_or("?");
                eprintln!("\x1b[36m  ╭─ glob \x1b[1m{pattern}\x1b[0m");
            }
            "grep" => {
                let pattern = input.get("pattern").and_then(|v| v.as_str()).unwrap_or("?");
                eprintln!("\x1b[36m  ╭─ grep \x1b[1m{pattern}\x1b[0m");
            }
            "web_fetch" => {
                let url = input.get("url").and_then(|v| v.as_str()).unwrap_or("?");
                eprintln!("\x1b[34m  ╭─ fetch \x1b[1m{url}\x1b[0m");
            }
            _ => {
                eprintln!("\x1b[36m  ╭─ {tool_name}\x1b[0m");
            }
        }
    }

    async fn on_tool_end(&self, tool_name: &str, result: &str, is_error: bool) {
        if is_error {
            // Show error with red
            let preview = result.lines().take(3).collect::<Vec<_>>().join("\n  │ ");
            eprintln!("\x1b[31m  │ {preview}\x1b[0m");
            eprintln!("\x1b[31m  ╰─ {tool_name} failed\x1b[0m");
        } else {
            match tool_name {
                "read_file" => {
                    // Show first few lines of file content
                    let lines: Vec<&str> = result.lines().collect();
                    let count = lines.len();
                    for line in lines.iter().take(5) {
                        eprintln!("\x1b[90m  │ {line}\x1b[0m");
                    }
                    if count > 5 {
                        eprintln!("\x1b[90m  │ ... ({count} lines total)\x1b[0m");
                    }
                    eprintln!("\x1b[36m  ╰─ done\x1b[0m");
                }
                "bash" => {
                    // Show command output
                    let lines: Vec<&str> = result.lines().collect();
                    for line in lines.iter().take(8) {
                        eprintln!("\x1b[90m  │ {line}\x1b[0m");
                    }
                    if lines.len() > 8 {
                        eprintln!("\x1b[90m  │ ... ({} lines)\x1b[0m", lines.len());
                    }
                    eprintln!("\x1b[35m  ╰─ done\x1b[0m");
                }
                "write_file" | "edit_file" => {
                    eprintln!("\x1b[33m  ╰─ \x1b[32m{result}\x1b[0m");
                }
                _ => {
                    let preview: String = result.chars().take(80).collect();
                    if !preview.is_empty() {
                        eprintln!("\x1b[90m  │ {preview}\x1b[0m");
                    }
                    eprintln!("\x1b[36m  ╰─ done\x1b[0m");
                }
            }
        }
    }
}

struct PrintCallbacks {
    auto_yes: bool,
}

#[async_trait::async_trait]
impl EngineCallbacks for PrintCallbacks {
    async fn ask_permission(&self, _prompt: &str) -> bool {
        self.auto_yes
    }
    async fn on_text(&self, text: &str) {
        print!("{text}");
        std::io::stdout().flush().ok();
    }
    async fn on_tool_start(&self, _tool_name: &str, _input: &serde_json::Value) {}
    async fn on_tool_end(&self, _tool_name: &str, _result: &str, _is_error: bool) {}
}

fn print_banner(provider: &str, model: &str, mode: &str) {
    eprintln!(
        "\x1b[38;5;209m\x1b[1mmy-little-claude\x1b[0m v{} \x1b[90m| {} / {}{}\x1b[0m",
        env!("CARGO_PKG_VERSION"),
        provider,
        model,
        mode
    );
}

fn build_provider(
    provider_name: &str,
    model: &str,
    config: &UnripeConfig,
) -> anyhow::Result<Box<dyn LlmProvider>> {
    match provider_name {
        "anthropic" => {
            let api_key = std::env::var(&config.provider.anthropic.api_key_env).map_err(|_| {
                anyhow::anyhow!(
                    "Anthropic API key not found. Set {} environment variable.\n\
                     Or use --provider ollama for local models.",
                    config.provider.anthropic.api_key_env
                )
            })?;
            let mut provider =
                unripe_providers::anthropic::AnthropicProvider::new(api_key, model.to_string());
            if let Some(url) = &config.provider.anthropic.base_url {
                provider = provider.with_base_url(url.clone());
            }
            Ok(Box::new(provider))
        }
        "ollama" => Ok(Box::new(unripe_providers::ollama::OllamaProvider::new(
            model.to_string(),
            config.provider.ollama.base_url.clone(),
        ))),
        "openai" => {
            let api_key = std::env::var(&config.provider.openai.api_key_env).map_err(|_| {
                anyhow::anyhow!(
                    "OpenAI API key not found. Set {} environment variable.",
                    config.provider.openai.api_key_env
                )
            })?;
            let mut provider =
                unripe_providers::openai::OpenAiProvider::new(api_key, model.to_string());
            if let Some(url) = &config.provider.openai.base_url {
                provider = provider.with_base_url(url.clone());
            }
            Ok(Box::new(provider))
        }
        other => anyhow::bail!("Unknown provider: {other}. Supported: anthropic, openai, ollama"),
    }
}

async fn run_setup(
    performance: &str,
    category: &str,
    install_model: Option<&str>,
    list: bool,
    auto_yes: bool,
) -> anyhow::Result<()> {
    use unripe_setup::{
        download::{check_ollama, is_model_available, pull_model},
        recommend::{
            available_models, format_model_list, recommend_for_category, ModelCategory,
            PerformancePreference,
        },
        sysinfo_detect::SystemInfo,
    };

    eprintln!("\x1b[1m== my-little-claude setup ==\x1b[0m\n");

    let sys = SystemInfo::detect();

    // --list: show all models and exit
    if list {
        eprintln!("  System: {}\n", sys.summary());
        let models = available_models();
        eprintln!("  Available models ({} total):\n", models.len());
        eprint!("{}", format_model_list(&models, Some(&sys)));
        return Ok(());
    }

    // --install: install a specific model
    if let Some(model_name) = install_model {
        eprintln!("\x1b[36mInstalling model: {model_name}\x1b[0m");

        let ollama_status = check_ollama();
        if !ollama_status.is_installed() {
            eprintln!("\x1b[31mollama is not installed.\x1b[0m");
            eprintln!("Install it from: https://ollama.com/download");
            return Ok(());
        }

        if is_model_available(model_name) {
            eprintln!("  Model {model_name} is already available.");
        } else {
            eprintln!("  Pulling {model_name} (this may take a while)...");
            match pull_model(model_name).await? {
                unripe_setup::download::PullResult::Success => {
                    eprintln!("  \x1b[32mDownload complete.\x1b[0m");
                }
                unripe_setup::download::PullResult::Failed(err) => {
                    eprintln!("  \x1b[31mDownload failed: {err}\x1b[0m");
                    return Ok(());
                }
            }
        }

        // Find model in catalog to save config
        let rec = available_models()
            .into_iter()
            .find(|m| m.model == model_name)
            .unwrap_or(unripe_setup::ModelRecommendation {
                model: model_name.to_string(),
                size_label: "?".into(),
                category: ModelCategory::General,
                tool_calling: false,
                description: "User-specified model".into(),
                estimated_ram_gb: 0.0,
            });

        let pref = PerformancePreference::Medium;
        let config_path = unripe_setup::download::save_setup_config(&sys, &pref, &rec)?;
        eprintln!("\n\x1b[32mSetup complete!\x1b[0m Default model set to {model_name}");
        eprintln!("  Config: {}", config_path.display());
        return Ok(());
    }

    // Auto-recommend flow
    eprintln!("\x1b[36m[1/4] Detecting system hardware...\x1b[0m");
    eprintln!("  {}", sys.summary());

    // Parse preference
    let pref = match performance.to_lowercase().as_str() {
        "high" | "h" => PerformancePreference::High,
        "medium" | "med" | "m" => PerformancePreference::Medium,
        "light" | "low" | "l" => PerformancePreference::Light,
        other => {
            eprintln!("\x1b[33mUnknown preference '{other}', using medium\x1b[0m");
            PerformancePreference::Medium
        }
    };

    // Parse category
    let cat = match category.to_lowercase().as_str() {
        "coding" | "code" | "c" => ModelCategory::Coding,
        "general" | "gen" | "g" => ModelCategory::General,
        "reasoning" | "reason" | "r" => ModelCategory::Reasoning,
        other => {
            eprintln!("\x1b[33mUnknown category '{other}', using coding\x1b[0m");
            ModelCategory::Coding
        }
    };

    eprintln!(
        "\n\x1b[36m[2/4] Preference: {} | Category: {}\x1b[0m",
        pref, cat
    );

    // Recommend model
    let rec = recommend_for_category(&sys, pref, &cat);
    eprintln!("\n\x1b[36m[3/4] Recommended model:\x1b[0m");
    eprintln!("  {} ({})", rec.model, rec.size_label);
    eprintln!(
        "  Category: {} | Tool calling: {}",
        rec.category,
        if rec.tool_calling { "yes" } else { "no" }
    );
    eprintln!("  {}", rec.description);
    eprintln!("  Estimated memory: {:.1}GB", rec.estimated_ram_gb);

    // Confirm
    if !auto_yes {
        eprint!("\n\x1b[33mProceed with {}?\x1b[0m [Y/n] ", rec.model);
        std::io::stderr().flush().ok();
        let mut input = String::new();
        std::io::stdin().read_line(&mut input).ok();
        let trimmed = input.trim().to_lowercase();
        if trimmed == "n" || trimmed == "no" {
            eprintln!(
                "Setup cancelled. Use --list to see all models or --install <model> to pick one."
            );
            return Ok(());
        }
    }

    // Check ollama and pull
    eprintln!("\n\x1b[36m[4/4] Downloading model...\x1b[0m");

    let ollama_status = check_ollama();
    if !ollama_status.is_installed() {
        eprintln!("\x1b[31mollama is not installed.\x1b[0m");
        eprintln!("Install it from: https://ollama.com/download");
        eprintln!("Then run: unripe setup");
        return Ok(());
    }

    if is_model_available(&rec.model) {
        eprintln!("  Model {} is already available.", rec.model);
    } else {
        eprintln!("  Pulling {} (this may take a while)...", rec.model);
        match pull_model(&rec.model).await? {
            unripe_setup::download::PullResult::Success => {
                eprintln!("  \x1b[32mDownload complete.\x1b[0m");
            }
            unripe_setup::download::PullResult::Failed(err) => {
                eprintln!("  \x1b[31mDownload failed: {err}\x1b[0m");
                eprintln!("  Try manually: ollama pull {}", rec.model);
                return Ok(());
            }
        }
    }

    // Save config
    let config_path = unripe_setup::download::save_setup_config(&sys, &pref, &rec)?;
    eprintln!(
        "\n\x1b[32mSetup complete!\x1b[0m Config saved to {}",
        config_path.display()
    );
    eprintln!("\nRun your first prompt:\n  \x1b[1munripe \"describe this project\"\x1b[0m");

    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Handle sessions list
    if matches!(&cli.command, Some(Commands::Sessions)) {
        let store = SessionStore::new()?;
        let sessions = store.list()?;
        if sessions.is_empty() {
            eprintln!("No saved sessions found.");
        } else {
            eprintln!("\x1b[1mSaved sessions:\x1b[0m");
            for id in &sessions {
                match store.load(id) {
                    Ok(s) => {
                        eprintln!(
                            "  {} | {} / {} | {} turns | {} messages",
                            &s.id[..8],
                            s.provider,
                            s.model,
                            s.turn_count,
                            s.messages.len()
                        );
                    }
                    Err(_) => eprintln!("  {} (corrupted)", &id[..8.min(id.len())]),
                }
            }
        }
        return Ok(());
    }

    // Handle replay
    if let Some(Commands::Replay {
        session_id,
        provider: replay_provider,
        model: replay_model,
    }) = &cli.command
    {
        let store = SessionStore::new()?;

        // Load session (support prefix matching)
        let all = store.list()?;
        let matched = all
            .iter()
            .find(|id| id.starts_with(session_id))
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("No session found matching '{session_id}'"))?;

        let original = store.load(&matched)?;
        let config = UnripeConfig::load();

        let prov_name = replay_provider.as_deref().unwrap_or(&original.provider);
        let mdl = replay_model.as_deref().unwrap_or(&original.model);

        eprintln!("\x1b[1m== Session Replay ==\x1b[0m");
        eprintln!(
            "  Original: {} / {} ({} turns)",
            original.provider, original.model, original.turn_count
        );
        eprintln!("  Replay:   {} / {}", prov_name, mdl);

        // Extract the user prompts from the original session
        let user_prompts: Vec<String> = original
            .messages
            .iter()
            .filter(|m| m.role == unripe_core::message::Role::User)
            .map(|m| m.text_content())
            .filter(|t| !t.is_empty())
            .collect();

        if user_prompts.is_empty() {
            eprintln!("  No user prompts found in session.");
            return Ok(());
        }

        eprintln!("  Replaying {} user prompt(s)...\n", user_prompts.len());

        let provider = build_provider(prov_name, mdl, &config)?;
        let project_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let tools = unripe_tools::builtin_tools(config.agent.bash_timeout_secs);
        let gate = DefaultPermissionGate::new(&project_root);

        let engine = AgentEngine::new(
            provider,
            tools,
            Box::new(gate),
            config.agent.clone(),
            project_root,
        );

        let mut new_session = Session::new(prov_name, mdl);
        let callbacks = TerminalCallbacks;

        for (i, prompt) in user_prompts.iter().enumerate() {
            eprintln!(
                "\x1b[36m--- Prompt {}/{} ---\x1b[0m",
                i + 1,
                user_prompts.len()
            );
            let reason = engine.run(prompt, &mut new_session, &callbacks).await?;
            eprintln!("\n\x1b[90m(stop: {reason:?})\x1b[0m\n");
        }

        store.save(&new_session)?;
        eprintln!(
            "\n\x1b[32mReplay complete.\x1b[0m New session: {}",
            &new_session.id[..8]
        );

        return Ok(());
    }

    // Handle setup subcommand
    if let Some(Commands::Setup {
        performance,
        category,
        install,
        list,
        yes,
    }) = &cli.command
    {
        return run_setup(performance, category, install.as_deref(), *list, *yes).await;
    }

    // Agent mode
    let config = UnripeConfig::load();

    let provider_name = cli
        .provider
        .as_deref()
        .unwrap_or(&config.provider.default_provider);
    let model = cli
        .model
        .as_deref()
        .unwrap_or(&config.provider.default_model);

    let provider = build_provider(provider_name, model, &config)?;

    // Auto-detect chat-only mode from model catalog
    let mut chat_only = cli.chat;
    if !chat_only && provider_name == "ollama" {
        let catalog = unripe_setup::recommend::available_models();
        let known = catalog.iter().find(|m| m.model == model);
        match known {
            Some(m) if !m.tool_calling => {
                eprintln!(
                    "\x1b[33m[auto] {} does not support tool calling. Switching to chat-only mode.\x1b[0m",
                    model
                );
                chat_only = true;
            }
            None => {
                eprintln!(
                    "\x1b[33m[hint] Model '{}' is not in the catalog. If tool calling fails, try: --chat\x1b[0m",
                    model
                );
            }
            _ => {}
        }
    }

    let is_repl = cli.prompt.is_none();
    let mode_label = if chat_only {
        " | chat-only"
    } else if is_repl {
        " | interactive"
    } else {
        ""
    };
    print_banner(provider_name, model, mode_label);

    let project_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut tools = unripe_tools::builtin_tools(config.agent.bash_timeout_secs);

    // Load MCP tools from .mcp.json / ~/.claude.json / ~/.unripe/mcp.json
    let mcp_config = unripe_mcp::load_mcp_config(&project_root);
    if !mcp_config.mcp_servers.is_empty() {
        let connections = unripe_mcp::connect_all(&mcp_config.mcp_servers).await;
        let mcp_tools = unripe_mcp::connections_to_tools(connections);
        if !mcp_tools.is_empty() {
            eprintln!("\x1b[90m{} MCP tools loaded\x1b[0m", mcp_tools.len());
            tools.extend(mcp_tools);
        }
    }

    let gate = DefaultPermissionGate::new(&project_root);

    let engine = AgentEngine::new(
        provider,
        tools,
        Box::new(gate),
        config.agent.clone(),
        project_root,
    )
    .with_chat_only(chat_only);

    let session_store = SessionStore::new()?;
    let mut session = if cli.resume {
        match session_store.load_latest() {
            Ok(s) => {
                eprintln!("\x1b[90mResuming session {}\x1b[0m", &s.id[..8]);
                s
            }
            Err(e) => {
                eprintln!("\x1b[33mNo session to resume ({e}), starting fresh\x1b[0m");
                Session::new(provider_name, model)
            }
        }
    } else {
        Session::new(provider_name, model)
    };

    match cli.prompt {
        Some(prompt) => {
            // One-shot mode
            tokio::spawn(async move {
                tokio::signal::ctrl_c().await.ok();
                eprintln!("\n\x1b[33m[Interrupted]\x1b[0m");
                std::process::exit(130);
            });

            if cli.print {
                // Non-interactive: only output final text
                let callbacks = PrintCallbacks { auto_yes: cli.yes };
                let _reason = engine.run(&prompt, &mut session, &callbacks).await?;
                println!(); // trailing newline
            } else {
                let callbacks = TerminalCallbacks;
                let reason = engine.run(&prompt, &mut session, &callbacks).await?;

                let _path = session_store.save(&session)?;
                eprintln!(
                    "\n\x1b[90mSession saved: {} ({:?})\x1b[0m",
                    &session.id[..8],
                    reason
                );
            }
        }
        None => {
            // Interactive REPL mode
            run_repl(engine, session, session_store).await?;
        }
    }

    Ok(())
}

// ── REPL ────────────────────────────────────────────────────────────

enum ReplCommand {
    Prompt(String),
    Exit,
    Clear,
    Save,
    History,
    Help,
    Undo,
    Skill { name: String, args: String },
    ListSkills,
}

fn parse_repl_command(input: &str) -> ReplCommand {
    let trimmed = input.trim();
    match trimmed {
        "/exit" | "/quit" | "/q" => ReplCommand::Exit,
        "/clear" => ReplCommand::Clear,
        "/save" => ReplCommand::Save,
        "/history" => ReplCommand::History,
        "/help" | "/?" => ReplCommand::Help,
        "/undo" => ReplCommand::Undo,
        "/skills" => ReplCommand::ListSkills,
        s if s.starts_with('/') => {
            // Skill invocation: /skill-name optional args
            let rest = &s[1..];
            let (name, args) = match rest.split_once(' ') {
                Some((n, a)) => (n.to_string(), a.to_string()),
                None => (rest.to_string(), String::new()),
            };
            ReplCommand::Skill { name, args }
        }
        _ => ReplCommand::Prompt(trimmed.to_string()),
    }
}

/// Load a skill prompt from .unripe/skills/{name}.md or ~/.unripe/skills/{name}.md
fn load_skill(name: &str, project_root: &std::path::Path) -> Option<String> {
    // Project-local skills first
    let local = project_root.join(format!(".unripe/skills/{name}.md"));
    if let Ok(content) = std::fs::read_to_string(&local) {
        return Some(content);
    }

    // User-global skills
    if let Some(home) = dirs::home_dir() {
        let global = home.join(format!(".unripe/skills/{name}.md"));
        if let Ok(content) = std::fs::read_to_string(&global) {
            return Some(content);
        }
    }

    None
}

/// List available skills from both local and global directories
fn list_skills(project_root: &std::path::Path) -> Vec<(String, String)> {
    let mut skills = Vec::new();
    let mut seen = std::collections::HashSet::new();

    let dirs_to_check: Vec<(std::path::PathBuf, &str)> = {
        let mut v = vec![(project_root.join(".unripe/skills"), "local")];
        if let Some(home) = dirs::home_dir() {
            v.push((home.join(".unripe/skills"), "global"));
        }
        v
    };

    for (dir, source) in &dirs_to_check {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "md") {
                    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                        if seen.insert(stem.to_string()) {
                            skills.push((stem.to_string(), source.to_string()));
                        }
                    }
                }
            }
        }
    }

    skills.sort();
    skills
}

async fn run_repl(
    engine: AgentEngine,
    mut session: Session,
    session_store: SessionStore,
) -> anyhow::Result<()> {
    use rustyline::error::ReadlineError;
    use rustyline::DefaultEditor;

    eprintln!("\x1b[90mInteractive mode. Type /help for commands, Ctrl+D to exit.\x1b[0m\n");

    let callbacks = TerminalCallbacks;
    let mut turn_number: u32 = 0;

    let history_path = dirs::home_dir()
        .unwrap_or_default()
        .join(".unripe")
        .join("repl_history");

    // rustyline readline is blocking, so we run the input loop on a blocking thread
    // and communicate back via channels
    loop {
        // Read input on a blocking thread (rustyline is !Send-safe with spawn_blocking
        // for the Editor, so we create a fresh editor each time — cheap operation)
        let hist = history_path.clone();
        let input = tokio::task::spawn_blocking(move || {
            let mut rl = DefaultEditor::new().ok()?;
            let _ = rl.load_history(&hist);
            let result = rl.readline("\x1b[38;5;209munripe>\x1b[0m ");
            if let Ok(ref line) = result {
                let _ = rl.add_history_entry(line);
                let _ = rl.save_history(&hist);
            }
            Some(result)
        })
        .await?;

        let input = match input {
            Some(Ok(line)) => line,
            Some(Err(ReadlineError::Interrupted)) => {
                eprintln!("\x1b[90m(interrupted)\x1b[0m");
                continue;
            }
            Some(Err(ReadlineError::Eof)) => break,
            Some(Err(e)) => {
                eprintln!("\x1b[31mInput error: {e}\x1b[0m");
                break;
            }
            None => break,
        };

        let input = input.trim().to_string();
        if input.is_empty() {
            continue;
        }

        match parse_repl_command(&input) {
            ReplCommand::Exit => break,
            ReplCommand::Clear => {
                let provider = session.provider.clone();
                let model = session.model.clone();
                session = Session::new(&provider, &model);
                eprintln!("\x1b[90mSession cleared.\x1b[0m");
            }
            ReplCommand::Save => {
                session_store.save(&session)?;
                eprintln!("\x1b[90mSession saved: {}\x1b[0m", &session.id[..8]);
            }
            ReplCommand::History => {
                for msg in &session.messages {
                    let role = format!("{:?}", msg.role);
                    let text = msg.text_content();
                    let preview: String = text.chars().take(120).collect();
                    if !preview.is_empty() {
                        eprintln!("\x1b[90m[{role}] {preview}\x1b[0m");
                    }
                }
            }
            ReplCommand::Undo => match engine.undo() {
                Some(label) => {
                    eprintln!("\x1b[32mUndo: {label}\x1b[0m");
                    let remaining = engine.checkpoint_count();
                    if remaining > 0 {
                        eprintln!("\x1b[90m({remaining} more undo(s) available)\x1b[0m");
                    }
                }
                None => {
                    eprintln!("\x1b[90mNothing to undo.\x1b[0m");
                }
            },
            ReplCommand::ListSkills => {
                let project_root = std::env::current_dir().unwrap_or_default();
                let skills = list_skills(&project_root);
                if skills.is_empty() {
                    eprintln!("\x1b[90mNo skills found. Add .md files to .unripe/skills/ or ~/.unripe/skills/\x1b[0m");
                } else {
                    eprintln!("\x1b[1mAvailable skills:\x1b[0m");
                    for (name, source) in &skills {
                        eprintln!("  /{name} \x1b[90m({source})\x1b[0m");
                    }
                }
            }
            ReplCommand::Skill { name, args } => {
                let project_root = std::env::current_dir().unwrap_or_default();
                match load_skill(&name, &project_root) {
                    Some(template) => {
                        // Replace {{ARGUMENTS}} placeholder with args
                        let prompt = if args.is_empty() {
                            template
                        } else {
                            template.replace("{{ARGUMENTS}}", &args)
                        };
                        session.reset_turn_budget();
                        turn_number += 1;
                        match engine.run(&prompt, &mut session, &callbacks).await {
                            Ok(reason) => eprintln!("\n\x1b[90m({reason:?})\x1b[0m\n"),
                            Err(e) => eprintln!("\n\x1b[31mError: {e}\x1b[0m\n"),
                        }
                        if turn_number.is_multiple_of(3) {
                            let _ = session_store.save(&session);
                        }
                    }
                    None => {
                        eprintln!("\x1b[33mSkill '/{name}' not found. Use /skills to list available skills.\x1b[0m");
                    }
                }
            }
            ReplCommand::Help => {
                eprintln!("  /exit, /quit, /q  Exit the REPL");
                eprintln!("  /clear            Clear conversation history");
                eprintln!("  /save             Save session to disk");
                eprintln!("  /undo             Undo the last file edit");
                eprintln!("  /skills           List available skills");
                eprintln!("  /<name> [args]    Run a skill");
                eprintln!("  /history          Show conversation messages");
                eprintln!("  /help, /?         Show this help");
                eprintln!("  Ctrl+D            Exit");
                eprintln!("  Ctrl+C            Cancel current input");
            }
            ReplCommand::Prompt(prompt) => {
                session.reset_turn_budget();
                turn_number += 1;

                match engine.run(&prompt, &mut session, &callbacks).await {
                    Ok(reason) => {
                        eprintln!("\n\x1b[90m({reason:?})\x1b[0m\n");
                    }
                    Err(e) => {
                        eprintln!("\n\x1b[31mError: {e}\x1b[0m\n");
                    }
                }

                // Auto-save every 3 turns
                if turn_number.is_multiple_of(3) {
                    let _ = session_store.save(&session);
                }
            }
        }
    }

    // Final save
    let _ = session_store.save(&session);
    eprintln!(
        "\x1b[90mSession saved: {}. Goodbye.\x1b[0m",
        &session.id[..8]
    );

    Ok(())
}
