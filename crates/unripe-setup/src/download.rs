use std::path::PathBuf;

use unripe_core::config::UnripeConfig;

use crate::recommend::ModelRecommendation;

/// Check if ollama is installed and reachable
pub fn check_ollama() -> OllamaStatus {
    match std::process::Command::new("ollama")
        .arg("--version")
        .output()
    {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
            OllamaStatus::Installed(version)
        }
        Ok(_) => OllamaStatus::Installed("unknown version".into()),
        Err(_) => OllamaStatus::NotInstalled,
    }
}

#[derive(Debug, Clone)]
pub enum OllamaStatus {
    Installed(String),
    NotInstalled,
}

impl OllamaStatus {
    pub fn is_installed(&self) -> bool {
        matches!(self, OllamaStatus::Installed(_))
    }
}

/// Check if a model is already pulled in ollama
pub fn is_model_available(model: &str) -> bool {
    match std::process::Command::new("ollama").arg("list").output() {
        Ok(output) if output.status.success() => {
            let text = String::from_utf8_lossy(&output.stdout);
            parse_ollama_list(&text, model)
        }
        _ => false,
    }
}

/// Parse `ollama list` output and check if the model name matches exactly
/// Format: "NAME:TAG    SIZE    MODIFIED"
fn parse_ollama_list(output: &str, model: &str) -> bool {
    output
        .lines()
        .skip(1)
        .any(|line| line.split_whitespace().next() == Some(model))
}

/// Pull a model via ollama. Returns the child process for progress tracking.
pub async fn pull_model(model: &str) -> anyhow::Result<PullResult> {
    let output = tokio::process::Command::new("ollama")
        .arg("pull")
        .arg(model)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await?;

    if output.status.success() {
        Ok(PullResult::Success)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        Ok(PullResult::Failed(stderr))
    }
}

#[derive(Debug)]
pub enum PullResult {
    Success,
    Failed(String),
}

/// Save the setup results to config.toml, including system detection info
pub fn save_setup_config(
    sys: &crate::sysinfo_detect::SystemInfo,
    pref: &crate::recommend::PerformancePreference,
    rec: &ModelRecommendation,
) -> anyhow::Result<PathBuf> {
    let config_dir = dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?
        .join(".unripe");
    std::fs::create_dir_all(&config_dir)?;
    let config_path = config_dir.join("config.toml");

    // Load existing or default
    let mut config = UnripeConfig::load_from_path(&config_path);

    // Update with setup results
    config.provider.default_provider = "ollama".into();
    config.provider.default_model = rec.model.clone();

    // Persist system detection info
    config.setup.ram_gb = Some(sys.ram_gb);
    config.setup.cpu_cores = Some(sys.cpu_cores);
    config.setup.gpu = sys.gpu.as_ref().map(|g| g.name.clone());
    config.setup.tier = Some(format!("{:?}", sys.tier()));
    config.setup.performance = Some(pref.to_string());

    config.save_to_path(&config_path)?;

    Ok(config_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_ollama() {
        // This test just verifies it doesn't panic. Result depends on environment.
        let status = check_ollama();
        match &status {
            OllamaStatus::Installed(v) => assert!(!v.is_empty()),
            OllamaStatus::NotInstalled => {} // fine on CI
        }
    }

    #[test]
    fn test_ollama_status_is_installed() {
        assert!(OllamaStatus::Installed("0.5.0".into()).is_installed());
        assert!(!OllamaStatus::NotInstalled.is_installed());
    }

    #[test]
    fn test_parse_ollama_list_exact_match() {
        let output = "NAME                ID              SIZE      MODIFIED\n\
                       llama3.2:3b         a80c4f17acd5    2.0 GB    5 days ago\n\
                       qwen3.5:9b    abc123def456    4.7 GB    2 days ago\n";
        assert!(parse_ollama_list(output, "qwen3.5:9b"));
        assert!(parse_ollama_list(output, "llama3.2:3b"));
        assert!(!parse_ollama_list(output, "qwen3.5:9b-instruct"));
        assert!(!parse_ollama_list(output, "qwen2.5-coder:14b"));
        assert!(!parse_ollama_list(output, "nonexistent:latest"));
    }

    #[test]
    fn test_parse_ollama_list_empty() {
        let output = "NAME                ID              SIZE      MODIFIED\n";
        assert!(!parse_ollama_list(output, "any-model:latest"));
    }

    #[test]
    fn test_save_setup_config() {
        let dir = std::env::temp_dir().join("unripe-test-setup-config");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // We can't easily mock home_dir, so test the config serialization part
        let sys = crate::sysinfo_detect::SystemInfo {
            ram_gb: 16.0,
            cpu_cores: 8,
            cpu_arch: "aarch64".into(),
            os: "macos 15.0".into(),
            gpu: None,
        };
        let pref = crate::recommend::PerformancePreference::Medium;
        let rec = crate::recommend::ModelRecommendation {
            model: "qwen3.5:9b".into(),
            size_label: "9B".into(),
            category: crate::recommend::ModelCategory::General,
            tool_calling: true,
            description: "test".into(),
            estimated_ram_gb: 6.0,
        };

        // Test that we can create the config object (save_setup_config uses home_dir which we can't mock easily)
        let mut config = UnripeConfig::default();
        config.provider.default_provider = "ollama".into();
        config.provider.default_model = rec.model.clone();

        let path = dir.join("config.toml");
        config.save_to_path(&path).unwrap();

        let loaded = UnripeConfig::load_from_path(&path);
        assert_eq!(loaded.provider.default_provider, "ollama");
        assert_eq!(loaded.provider.default_model, "qwen3.5:9b");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn test_pull_model_nonexistent() {
        // Skip if ollama not installed
        if !check_ollama().is_installed() {
            return;
        }
        // Try pulling a model that doesn't exist
        let result = pull_model("nonexistent-model-xyz:latest").await.unwrap();
        assert!(matches!(result, PullResult::Failed(_)));
    }
}
