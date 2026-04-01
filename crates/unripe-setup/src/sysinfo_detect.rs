use serde::{Deserialize, Serialize};
use sysinfo::System;

/// Detected system hardware information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemInfo {
    pub ram_gb: f64,
    pub cpu_cores: usize,
    pub cpu_arch: String,
    pub os: String,
    pub gpu: Option<GpuInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuInfo {
    pub name: String,
    pub vram_gb: Option<f64>,
}

impl SystemInfo {
    /// Detect current system hardware
    pub fn detect() -> Self {
        let mut sys = System::new_all();
        sys.refresh_all();

        let ram_gb = sys.total_memory() as f64 / 1_073_741_824.0; // bytes to GB
        let cpu_cores = sys.cpus().len();
        let cpu_arch = std::env::consts::ARCH.to_string();
        let os = format!(
            "{} {}",
            std::env::consts::OS,
            System::os_version().unwrap_or_default()
        );

        let gpu = detect_gpu();

        Self {
            ram_gb,
            cpu_cores,
            cpu_arch,
            os,
            gpu,
        }
    }

    /// Estimate available VRAM or usable memory for models
    pub fn effective_model_memory_gb(&self) -> f64 {
        if let Some(gpu) = &self.gpu {
            if let Some(vram) = gpu.vram_gb {
                return vram;
            }
        }
        // Apple Silicon uses unified memory — estimate 75% available for models
        if self.cpu_arch == "aarch64" && self.os.contains("macos") {
            return self.ram_gb * 0.75;
        }
        // CPU-only inference: use ~50% of RAM
        self.ram_gb * 0.5
    }

    /// Classify system into tiers
    pub fn tier(&self) -> SystemTier {
        let mem = self.effective_model_memory_gb();
        if mem >= 16.0 {
            SystemTier::High
        } else if mem >= 8.0 {
            SystemTier::Medium
        } else {
            SystemTier::Low
        }
    }

    /// Human-readable summary
    pub fn summary(&self) -> String {
        let gpu_str = match &self.gpu {
            Some(g) => match g.vram_gb {
                Some(v) => format!("{} ({:.1}GB VRAM)", g.name, v),
                None => g.name.clone(),
            },
            None => "none detected".into(),
        };
        format!(
            "RAM: {:.1}GB | CPU: {}x {} | GPU: {} | OS: {} | Tier: {:?}",
            self.ram_gb,
            self.cpu_cores,
            self.cpu_arch,
            gpu_str,
            self.os,
            self.tier()
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SystemTier {
    High,
    Medium,
    Low,
}

/// Try to detect GPU info via platform-specific methods
fn detect_gpu() -> Option<GpuInfo> {
    // Try nvidia-smi first (Linux/Windows with NVIDIA GPU)
    if let Ok(output) = std::process::Command::new("nvidia-smi")
        .arg("--query-gpu=name,memory.total")
        .arg("--format=csv,noheader,nounits")
        .output()
    {
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout);
            let line = text.trim();
            if let Some((name, vram_str)) = line.split_once(',') {
                let vram_mb: f64 = vram_str.trim().parse().unwrap_or(0.0);
                return Some(GpuInfo {
                    name: name.trim().to_string(),
                    vram_gb: Some(vram_mb / 1024.0),
                });
            }
        }
    }

    // macOS: detect Apple Silicon GPU via system_profiler
    #[cfg(target_os = "macos")]
    {
        if let Ok(output) = std::process::Command::new("system_profiler")
            .arg("SPDisplaysDataType")
            .arg("-json")
            .output()
        {
            if output.status.success() {
                let text = String::from_utf8_lossy(&output.stdout);
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&text) {
                    if let Some(displays) = val.get("SPDisplaysDataType").and_then(|v| v.as_array())
                    {
                        for display in displays {
                            if let Some(name) = display.get("sppci_model").and_then(|v| v.as_str())
                            {
                                return Some(GpuInfo {
                                    name: name.to_string(),
                                    vram_gb: None, // Apple Silicon shares RAM
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_system_info() {
        let info = SystemInfo::detect();
        assert!(info.ram_gb > 0.0);
        assert!(info.cpu_cores > 0);
        assert!(!info.cpu_arch.is_empty());
        assert!(!info.os.is_empty());
    }

    #[test]
    fn test_effective_memory() {
        let info = SystemInfo {
            ram_gb: 16.0,
            cpu_cores: 8,
            cpu_arch: "aarch64".into(),
            os: "macos 15.0".into(),
            gpu: None,
        };
        // Apple Silicon: 75% of 16GB = 12GB
        assert!((info.effective_model_memory_gb() - 12.0).abs() < 0.1);
    }

    #[test]
    fn test_effective_memory_with_gpu() {
        let info = SystemInfo {
            ram_gb: 32.0,
            cpu_cores: 16,
            cpu_arch: "x86_64".into(),
            os: "linux".into(),
            gpu: Some(GpuInfo {
                name: "RTX 4090".into(),
                vram_gb: Some(24.0),
            }),
        };
        assert!((info.effective_model_memory_gb() - 24.0).abs() < 0.1);
    }

    #[test]
    fn test_effective_memory_cpu_only() {
        let info = SystemInfo {
            ram_gb: 8.0,
            cpu_cores: 4,
            cpu_arch: "x86_64".into(),
            os: "linux".into(),
            gpu: None,
        };
        // CPU-only: 50% of 8GB = 4GB
        assert!((info.effective_model_memory_gb() - 4.0).abs() < 0.1);
    }

    #[test]
    fn test_system_tier_high() {
        let info = SystemInfo {
            ram_gb: 64.0,
            cpu_cores: 16,
            cpu_arch: "x86_64".into(),
            os: "linux".into(),
            gpu: Some(GpuInfo {
                name: "RTX 4090".into(),
                vram_gb: Some(24.0),
            }),
        };
        assert_eq!(info.tier(), SystemTier::High);
    }

    #[test]
    fn test_system_tier_medium() {
        let info = SystemInfo {
            ram_gb: 16.0,
            cpu_cores: 8,
            cpu_arch: "aarch64".into(),
            os: "macos 15.0".into(),
            gpu: None,
        };
        assert_eq!(info.tier(), SystemTier::Medium);
    }

    #[test]
    fn test_system_tier_low() {
        let info = SystemInfo {
            ram_gb: 8.0,
            cpu_cores: 4,
            cpu_arch: "x86_64".into(),
            os: "linux".into(),
            gpu: None,
        };
        assert_eq!(info.tier(), SystemTier::Low);
    }

    #[test]
    fn test_summary() {
        let info = SystemInfo {
            ram_gb: 16.0,
            cpu_cores: 10,
            cpu_arch: "aarch64".into(),
            os: "macos 15.0".into(),
            gpu: Some(GpuInfo {
                name: "Apple M2 Pro".into(),
                vram_gb: None,
            }),
        };
        let summary = info.summary();
        assert!(summary.contains("16.0GB"));
        assert!(summary.contains("10x"));
        assert!(summary.contains("Apple M2 Pro"));
    }

    #[test]
    fn test_serialization_roundtrip() {
        let info = SystemInfo {
            ram_gb: 32.0,
            cpu_cores: 8,
            cpu_arch: "x86_64".into(),
            os: "linux".into(),
            gpu: None,
        };
        let json = serde_json::to_string(&info).unwrap();
        let parsed: SystemInfo = serde_json::from_str(&json).unwrap();
        assert!((parsed.ram_gb - 32.0).abs() < 0.1);
        assert_eq!(parsed.cpu_cores, 8);
    }
}
