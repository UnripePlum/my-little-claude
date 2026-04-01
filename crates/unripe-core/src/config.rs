use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Agent engine configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Maximum conversation turns before stopping
    #[serde(default = "default_max_turns")]
    pub max_turns: u32,

    /// Token budget before stopping (rough estimate)
    #[serde(default = "default_token_budget")]
    pub token_budget: u64,

    /// Bash command timeout in seconds
    #[serde(default = "default_bash_timeout")]
    pub bash_timeout_secs: u64,

    /// Number of recent messages to keep on truncation
    #[serde(default = "default_truncation_keep")]
    pub truncation_keep_recent: usize,

    /// Additional context files to load during bootstrap
    #[serde(default)]
    pub context_files: Vec<String>,

    /// Hooks: shell commands triggered on tool events
    #[serde(default)]
    pub hooks: Vec<HookConfig>,
}

/// A hook that runs a shell command on a tool event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookConfig {
    /// Event type: "pre_tool_use" or "post_tool_use"
    pub event: String,
    /// Tool name to match (or "*" for all tools)
    #[serde(default = "default_hook_tool")]
    pub tool: String,
    /// Shell command to execute. Gets TOOL_NAME, TOOL_INPUT env vars.
    /// For pre_tool_use: exit 0 = proceed, non-zero = block.
    pub command: String,
}

fn default_hook_tool() -> String {
    "*".into()
}

fn default_max_turns() -> u32 {
    25
}
fn default_token_budget() -> u64 {
    100_000
}
fn default_bash_timeout() -> u64 {
    30
}
fn default_truncation_keep() -> usize {
    10
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            max_turns: default_max_turns(),
            token_budget: default_token_budget(),
            bash_timeout_secs: default_bash_timeout(),
            truncation_keep_recent: default_truncation_keep(),
            context_files: Vec::new(),
            hooks: Vec::new(),
        }
    }
}

/// Detected system info persisted during setup
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SetupInfo {
    #[serde(default)]
    pub ram_gb: Option<f64>,
    #[serde(default)]
    pub cpu_cores: Option<usize>,
    #[serde(default)]
    pub gpu: Option<String>,
    #[serde(default)]
    pub tier: Option<String>,
    #[serde(default)]
    pub performance: Option<String>,
}

/// Top-level configuration file (~/.unripe/config.toml)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UnripeConfig {
    #[serde(default)]
    pub agent: AgentConfig,

    #[serde(default)]
    pub provider: ProviderConfig,

    #[serde(default)]
    pub setup: SetupInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    #[serde(default = "default_provider")]
    pub default_provider: String,

    #[serde(default = "default_model")]
    pub default_model: String,

    #[serde(default)]
    pub anthropic: AnthropicConfig,

    #[serde(default)]
    pub ollama: OllamaConfig,

    #[serde(default)]
    pub openai: OpenAiConfig,
}

fn default_provider() -> String {
    "ollama".into()
}
fn default_model() -> String {
    "qwen3.5:9b".into()
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            default_provider: default_provider(),
            default_model: default_model(),
            anthropic: AnthropicConfig::default(),
            ollama: OllamaConfig::default(),
            openai: OpenAiConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicConfig {
    /// Environment variable name for the API key
    #[serde(default = "default_anthropic_key_env")]
    pub api_key_env: String,

    /// Base URL override
    pub base_url: Option<String>,
}

fn default_anthropic_key_env() -> String {
    "ANTHROPIC_API_KEY".into()
}

impl Default for AnthropicConfig {
    fn default() -> Self {
        Self {
            api_key_env: default_anthropic_key_env(),
            base_url: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaConfig {
    /// Ollama server URL
    #[serde(default = "default_ollama_url")]
    pub base_url: String,
}

fn default_ollama_url() -> String {
    "http://localhost:11434".into()
}

impl Default for OllamaConfig {
    fn default() -> Self {
        Self {
            base_url: default_ollama_url(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAiConfig {
    #[serde(default = "default_openai_key_env")]
    pub api_key_env: String,
    pub base_url: Option<String>,
}

fn default_openai_key_env() -> String {
    "OPENAI_API_KEY".into()
}

impl Default for OpenAiConfig {
    fn default() -> Self {
        Self {
            api_key_env: default_openai_key_env(),
            base_url: None,
        }
    }
}

impl UnripeConfig {
    /// Load config from ~/.unripe/config.toml, or return defaults
    pub fn load() -> Self {
        Self::load_from_path(
            &dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".unripe")
                .join("config.toml"),
        )
    }

    pub fn load_from_path(path: &PathBuf) -> Self {
        match std::fs::read_to_string(path) {
            Ok(content) => toml::from_str(&content).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// Save config to a path
    pub fn save_to_path(&self, path: &PathBuf) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_agent_config() {
        let config = AgentConfig::default();
        assert_eq!(config.max_turns, 25);
        assert_eq!(config.token_budget, 100_000);
        assert_eq!(config.bash_timeout_secs, 30);
        assert_eq!(config.truncation_keep_recent, 10);
        assert!(config.context_files.is_empty());
    }

    #[test]
    fn test_default_provider_config() {
        let config = ProviderConfig::default();
        assert_eq!(config.default_provider, "ollama");
        assert_eq!(config.default_model, "qwen3.5:9b");
        assert_eq!(config.ollama.base_url, "http://localhost:11434");
    }

    #[test]
    fn test_config_toml_roundtrip() {
        let config = UnripeConfig::default();
        let toml_str = toml::to_string_pretty(&config).unwrap();
        let parsed: UnripeConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.agent.max_turns, 25);
        assert_eq!(parsed.provider.default_provider, "ollama");
    }

    #[test]
    fn test_config_load_missing_file() {
        let config = UnripeConfig::load_from_path(&PathBuf::from("/nonexistent/config.toml"));
        assert_eq!(config.agent.max_turns, 25); // defaults
    }

    #[test]
    fn test_config_save_and_load() {
        let dir = std::env::temp_dir().join("unripe-test-config");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");

        let mut config = UnripeConfig::default();
        config.agent.max_turns = 50;
        config.provider.default_provider = "ollama".into();
        config.save_to_path(&path).unwrap();

        let loaded = UnripeConfig::load_from_path(&path);
        assert_eq!(loaded.agent.max_turns, 50);
        assert_eq!(loaded.provider.default_provider, "ollama");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_config_partial_toml() {
        let toml_str = r#"
[agent]
max_turns = 10

[provider]
default_provider = "ollama"
default_model = "qwen2.5-coder:7b"
"#;
        let config: UnripeConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.agent.max_turns, 10);
        assert_eq!(config.agent.bash_timeout_secs, 30); // default
        assert_eq!(config.provider.default_provider, "ollama");
        assert_eq!(config.provider.default_model, "qwen2.5-coder:7b");
    }
}
