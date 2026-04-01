use serde::{Deserialize, Serialize};

use crate::sysinfo_detect::{SystemInfo, SystemTier};

/// User's performance preference
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PerformancePreference {
    High,
    Medium,
    Light,
}

impl std::fmt::Display for PerformancePreference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::High => write!(f, "high"),
            Self::Medium => write!(f, "medium"),
            Self::Light => write!(f, "light"),
        }
    }
}

/// A recommended model with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRecommendation {
    pub model: String,
    pub size_label: String,
    pub description: String,
    pub estimated_ram_gb: f64,
}

/// Load model catalog from embedded JSON (models.json)
fn load_model_catalog() -> Vec<ModelRecommendation> {
    let json = include_str!("../models.json");
    serde_json::from_str(json).expect("models.json must be valid JSON")
}

/// Model recommendation matrix
/// Rows: performance preference, Columns: system tier
const MATRIX: [[&str; 3]; 3] = [
    // [High tier, Medium tier, Low tier]
    // High preference
    ["qwen2.5-coder:32b", "qwen2.5-coder:14b", "qwen2.5-coder:7b"],
    // Medium preference
    ["qwen2.5-coder:14b", "qwen2.5-coder:7b", "qwen2.5-coder:3b"],
    // Light preference
    ["qwen2.5-coder:7b", "qwen2.5-coder:3b", "qwen2.5-coder:1.5b"],
];

/// Get model details for a given model name from the catalog
fn model_details(model: &str) -> ModelRecommendation {
    let catalog = load_model_catalog();
    catalog
        .into_iter()
        .find(|m| m.model == model)
        .unwrap_or(ModelRecommendation {
            model: model.into(),
            size_label: "?".into(),
            description: "Unknown model".into(),
            estimated_ram_gb: 4.0,
        })
}

/// Recommend a model based on system info and user preference
pub fn recommend(sys: &SystemInfo, pref: PerformancePreference) -> ModelRecommendation {
    let tier_idx = match sys.tier() {
        SystemTier::High => 0,
        SystemTier::Medium => 1,
        SystemTier::Low => 2,
    };
    let pref_idx = match pref {
        PerformancePreference::High => 0,
        PerformancePreference::Medium => 1,
        PerformancePreference::Light => 2,
    };

    let model_name = MATRIX[pref_idx][tier_idx];
    let rec = model_details(model_name);

    // Verify the recommended model fits in available memory; downgrade if needed
    let available = sys.effective_model_memory_gb();
    if rec.estimated_ram_gb > available {
        // Try one size smaller
        let smaller_pref_idx = (pref_idx + 1).min(2);
        let smaller_model = MATRIX[smaller_pref_idx][tier_idx];
        let smaller_rec = model_details(smaller_model);
        if smaller_rec.estimated_ram_gb <= available {
            return smaller_rec;
        }
        // Try smallest
        let smallest = MATRIX[2][2];
        return model_details(smallest);
    }

    rec
}

/// Get all available models for display
pub fn available_models() -> Vec<ModelRecommendation> {
    load_model_catalog()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sysinfo_detect::GpuInfo;

    fn sys(ram_gb: f64, arch: &str, os: &str, gpu_vram: Option<f64>) -> SystemInfo {
        SystemInfo {
            ram_gb,
            cpu_cores: 8,
            cpu_arch: arch.into(),
            os: os.into(),
            gpu: gpu_vram.map(|v| GpuInfo {
                name: "Test GPU".into(),
                vram_gb: Some(v),
            }),
        }
    }

    #[test]
    fn test_high_tier_high_pref() {
        let s = sys(64.0, "x86_64", "linux", Some(24.0));
        let rec = recommend(&s, PerformancePreference::High);
        assert_eq!(rec.model, "qwen2.5-coder:32b");
    }

    #[test]
    fn test_medium_tier_medium_pref() {
        let s = sys(16.0, "aarch64", "macos 15.0", None);
        let rec = recommend(&s, PerformancePreference::Medium);
        assert_eq!(rec.model, "qwen2.5-coder:7b");
    }

    #[test]
    fn test_low_tier_light_pref() {
        let s = sys(8.0, "x86_64", "linux", None);
        let rec = recommend(&s, PerformancePreference::Light);
        assert_eq!(rec.model, "qwen2.5-coder:1.5b");
    }

    #[test]
    fn test_downgrade_when_model_too_large() {
        // 4GB effective memory, high pref on high tier would pick 32b (needs 20GB)
        // Should downgrade
        let s = sys(8.0, "x86_64", "linux", None); // 4GB effective
        let rec = recommend(&s, PerformancePreference::High);
        assert!(
            rec.estimated_ram_gb <= 4.0,
            "model {} needs {}GB but only 4GB available",
            rec.model,
            rec.estimated_ram_gb
        );
    }

    #[test]
    fn test_apple_silicon_medium() {
        let s = sys(16.0, "aarch64", "macos 15.0", None);
        // Effective: 12GB, Medium pref, Medium tier
        let rec = recommend(&s, PerformancePreference::Medium);
        assert!(rec.estimated_ram_gb <= 12.0);
    }

    #[test]
    fn test_available_models_count() {
        let models = available_models();
        assert_eq!(models.len(), 5);
        assert_eq!(models[0].model, "qwen2.5-coder:32b");
        assert_eq!(models[4].model, "qwen2.5-coder:1.5b");
    }

    #[test]
    fn test_performance_preference_display() {
        assert_eq!(PerformancePreference::High.to_string(), "high");
        assert_eq!(PerformancePreference::Medium.to_string(), "medium");
        assert_eq!(PerformancePreference::Light.to_string(), "light");
    }

    #[test]
    fn test_model_recommendation_serialization() {
        let rec = model_details("qwen2.5-coder:7b");
        let json = serde_json::to_string(&rec).unwrap();
        let parsed: ModelRecommendation = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.model, "qwen2.5-coder:7b");
        assert_eq!(parsed.size_label, "7B");
    }
}
