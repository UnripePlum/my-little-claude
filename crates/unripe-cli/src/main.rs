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
        let summary = match tool_name {
            "read_file" => format!(
                "read_file({})",
                input.get("path").and_then(|v| v.as_str()).unwrap_or("?")
            ),
            "write_file" => format!(
                "write_file({})",
                input.get("path").and_then(|v| v.as_str()).unwrap_or("?")
            ),
            "bash" => format!(
                "bash({})",
                input
                    .get("command")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?")
                    .chars()
                    .take(60)
                    .collect::<String>()
            ),
            _ => format!("{tool_name}(...)"),
        };
        eprintln!("\x1b[36m> {summary}\x1b[0m");
    }

    async fn on_tool_end(&self, tool_name: &str, _result: &str, is_error: bool) {
        if is_error {
            eprintln!("\x1b[31m> {tool_name} failed\x1b[0m");
        }
    }
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

    let prompt = match &cli.prompt {
        Some(p) => p.clone(),
        None => {
            eprintln!("Usage: unripe \"your prompt here\"");
            eprintln!(
                "       unripe setup              -- detect hardware and download a local model"
            );
            eprintln!("       unripe --provider ollama --model qwen2.5-coder:7b \"your prompt\"");
            std::process::exit(1);
        }
    };

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

    let mode_label = if chat_only { " | chat-only" } else { "" };
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

    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        eprintln!("\n\x1b[33m[Interrupted]\x1b[0m");
        std::process::exit(130);
    });

    let callbacks = TerminalCallbacks;
    let reason = engine.run(&prompt, &mut session, &callbacks).await?;

    let _path = session_store.save(&session)?;
    eprintln!(
        "\n\x1b[90mSession saved: {} ({:?})\x1b[0m",
        &session.id[..8],
        reason
    );

    Ok(())
}
