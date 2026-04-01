use std::io::Write;
use std::path::PathBuf;

use clap::Parser;
use unripe_core::config::UnripeConfig;
use unripe_core::permission::DefaultPermissionGate;
use unripe_core::provider::LlmProvider;
use unripe_core::session::{Session, SessionStore};
use unripe_engine::engine::{AgentEngine, EngineCallbacks};

#[derive(Parser)]
#[command(
    name = "unripe",
    version,
    about = "my-little-claude — a model-agnostic coding agent"
)]
struct Cli {
    /// The prompt to send to the agent
    prompt: Option<String>,

    /// LLM provider to use (anthropic, ollama)
    #[arg(long, default_value = None)]
    provider: Option<String>,

    /// Model name to use
    #[arg(long, default_value = None)]
    model: Option<String>,

    /// Resume the most recent session
    #[arg(long)]
    resume: bool,
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
        other => anyhow::bail!("Unknown provider: {other}. Supported: anthropic, ollama"),
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
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
            eprintln!("       unripe --provider ollama --model qwen2.5-coder:7b \"your prompt\"");
            std::process::exit(1);
        }
    };

    // Build provider
    let provider = build_provider(provider_name, model, &config)?;

    eprintln!(
        "\x1b[90mmy-little-claude v{} | {} / {}\x1b[0m",
        env!("CARGO_PKG_VERSION"),
        provider_name,
        model
    );

    // Project root = current directory
    let project_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    // Tools
    let tools = unripe_tools::builtin_tools(config.agent.bash_timeout_secs);

    // Permission gate
    let gate = DefaultPermissionGate::new(&project_root);

    // Engine
    let engine = AgentEngine::new(
        provider,
        tools,
        Box::new(gate),
        config.agent.clone(),
        project_root,
    );

    // Session
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

    // Register Ctrl+C handler
    let _session_id = session.id.clone();
    let _ctrlc_store = SessionStore::new()?;
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        eprintln!("\n\x1b[33m[Interrupted]\x1b[0m");
        // Try to save session on interrupt (best effort)
        // We can't easily access the session here, but the main loop will save on normal exit
        std::process::exit(130);
    });

    // Run
    let callbacks = TerminalCallbacks;
    let reason = engine.run(&prompt, &mut session, &callbacks).await?;

    // Save session
    let _path = session_store.save(&session)?;
    eprintln!(
        "\n\x1b[90mSession saved: {} ({:?})\x1b[0m",
        &session.id[..8],
        reason
    );

    Ok(())
}
