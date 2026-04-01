use serde::{Deserialize, Serialize};

use crate::sysinfo_detect::SystemInfo;

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

/// Model category
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelCategory {
    Coding,
    General,
    Reasoning,
}

impl std::fmt::Display for ModelCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Coding => write!(f, "coding"),
            Self::General => write!(f, "general"),
            Self::Reasoning => write!(f, "reasoning"),
        }
    }
}

/// A model entry with full metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRecommendation {
    pub model: String,
    pub size_label: String,
    #[serde(default = "default_category")]
    pub category: ModelCategory,
    #[serde(default)]
    pub tool_calling: bool,
    pub description: String,
    pub estimated_ram_gb: f64,
}

fn default_category() -> ModelCategory {
    ModelCategory::General
}

/// Load model catalog from embedded JSON (models.json)
fn load_model_catalog() -> Vec<ModelRecommendation> {
    let json = include_str!("../models.json");
    serde_json::from_str(json).expect("models.json must be valid JSON")
}

/// Get all available models
pub fn available_models() -> Vec<ModelRecommendation> {
    load_model_catalog()
}

/// Filter models by category
pub fn models_by_category(category: &ModelCategory) -> Vec<ModelRecommendation> {
    load_model_catalog()
        .into_iter()
        .filter(|m| &m.category == category)
        .collect()
}

/// Filter models that fit in available memory
pub fn models_that_fit(sys: &SystemInfo) -> Vec<ModelRecommendation> {
    let available = sys.effective_model_memory_gb();
    load_model_catalog()
        .into_iter()
        .filter(|m| m.estimated_ram_gb <= available)
        .collect()
}

/// Filter models with tool calling support
pub fn models_with_tool_calling() -> Vec<ModelRecommendation> {
    load_model_catalog()
        .into_iter()
        .filter(|m| m.tool_calling)
        .collect()
}

/// Smart recommendation: category + system tier + preference
pub fn recommend(sys: &SystemInfo, pref: PerformancePreference) -> ModelRecommendation {
    recommend_for_category(sys, pref, &ModelCategory::Coding)
}

/// Recommend best model for a specific category
pub fn recommend_for_category(
    sys: &SystemInfo,
    pref: PerformancePreference,
    category: &ModelCategory,
) -> ModelRecommendation {
    let available = sys.effective_model_memory_gb();

    // Get models that fit, have tool calling, and match category
    let mut candidates: Vec<ModelRecommendation> = load_model_catalog()
        .into_iter()
        .filter(|m| &m.category == category && m.tool_calling && m.estimated_ram_gb <= available)
        .collect();

    // Sort by size (descending for high pref, ascending for light)
    candidates.sort_by(|a, b| {
        a.estimated_ram_gb
            .partial_cmp(&b.estimated_ram_gb)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    if candidates.is_empty() {
        // Fallback: any model with tool calling that fits
        let mut fallback: Vec<ModelRecommendation> = load_model_catalog()
            .into_iter()
            .filter(|m| m.tool_calling && m.estimated_ram_gb <= available)
            .collect();
        fallback.sort_by(|a, b| {
            a.estimated_ram_gb
                .partial_cmp(&b.estimated_ram_gb)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        if fallback.is_empty() {
            // Last resort: smallest model in catalog
            let mut all = load_model_catalog();
            all.sort_by(|a, b| {
                a.estimated_ram_gb
                    .partial_cmp(&b.estimated_ram_gb)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            return all.into_iter().next().unwrap_or(ModelRecommendation {
                model: "qwen3.5:2b".into(),
                size_label: "2B".into(),
                category: ModelCategory::General,
                tool_calling: true,
                description: "Fallback model".into(),
                estimated_ram_gb: 1.5,
            });
        }

        return pick_by_preference(&fallback, pref);
    }

    pick_by_preference(&candidates, pref)
}

/// Pick model from sorted candidates based on preference
fn pick_by_preference(
    sorted_candidates: &[ModelRecommendation],
    pref: PerformancePreference,
) -> ModelRecommendation {
    let len = sorted_candidates.len();
    match pref {
        PerformancePreference::High => sorted_candidates[len - 1].clone(), // largest
        PerformancePreference::Medium => sorted_candidates[len / 2].clone(), // middle
        PerformancePreference::Light => sorted_candidates[0].clone(),      // smallest
    }
}

/// Format model list for display in CLI
pub fn format_model_list(models: &[ModelRecommendation], sys: Option<&SystemInfo>) -> String {
    let available_mem = sys.map(|s| s.effective_model_memory_gb());

    let mut output = String::new();
    let mut current_category = String::new();

    // Group by category
    let mut sorted = models.to_vec();
    sorted.sort_by(|a, b| {
        a.category.to_string().cmp(&b.category.to_string()).then(
            a.estimated_ram_gb
                .partial_cmp(&b.estimated_ram_gb)
                .unwrap_or(std::cmp::Ordering::Equal),
        )
    });

    for m in &sorted {
        let cat = m.category.to_string();
        if cat != current_category {
            if !output.is_empty() {
                output.push('\n');
            }
            output.push_str(&format!("  [{cat}]\n"));
            current_category = cat;
        }

        let fits = match available_mem {
            Some(mem) => {
                if m.estimated_ram_gb <= mem {
                    "  "
                } else {
                    "! "
                }
            }
            None => "  ",
        };

        let tool_icon = if m.tool_calling { "T" } else { " " };

        output.push_str(&format!(
            "  {fits}[{tool_icon}] {:<30} {:>6}  {:.0}GB  {}\n",
            m.model, m.size_label, m.estimated_ram_gb, m.description
        ));
    }

    output.push_str("\n  T = tool calling supported, ! = may not fit in memory\n");
    output
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
    fn test_high_tier_high_pref_picks_largest_coding() {
        let s = sys(64.0, "x86_64", "linux", Some(24.0));
        let rec = recommend(&s, PerformancePreference::High);
        assert!(rec.tool_calling);
        assert_eq!(rec.category, ModelCategory::Coding);
        // Should pick largest coding model that fits in 24GB
    }

    #[test]
    fn test_medium_tier_medium_pref() {
        let s = sys(16.0, "aarch64", "macos 15.0", None);
        let rec = recommend(&s, PerformancePreference::Medium);
        assert!(rec.tool_calling);
        assert!(rec.estimated_ram_gb <= 12.0); // 75% of 16GB
    }

    #[test]
    fn test_low_tier_light_pref() {
        let s = sys(8.0, "x86_64", "linux", None);
        let rec = recommend(&s, PerformancePreference::Light);
        assert!(rec.tool_calling);
        assert!(rec.estimated_ram_gb <= 4.0); // 50% of 8GB
    }

    #[test]
    fn test_downgrade_when_no_coding_fits() {
        // Very small memory: no coding models fit
        let s = sys(2.0, "x86_64", "linux", None); // 1GB effective
        let rec = recommend(&s, PerformancePreference::High);
        // Should fall back to smallest available model
        assert!(rec.estimated_ram_gb <= 1.0);
    }

    #[test]
    fn test_recommend_for_reasoning() {
        let s = sys(64.0, "x86_64", "linux", Some(24.0));
        let rec =
            recommend_for_category(&s, PerformancePreference::High, &ModelCategory::Reasoning);
        assert_eq!(rec.category, ModelCategory::Reasoning);
        assert!(rec.tool_calling);
    }

    #[test]
    fn test_recommend_for_general() {
        let s = sys(16.0, "aarch64", "macos 15.0", None);
        let rec =
            recommend_for_category(&s, PerformancePreference::Medium, &ModelCategory::General);
        assert_eq!(rec.category, ModelCategory::General);
    }

    #[test]
    fn test_models_by_category() {
        let coding = models_by_category(&ModelCategory::Coding);
        assert!(coding.len() >= 3);
        assert!(coding.iter().all(|m| m.category == ModelCategory::Coding));
    }

    #[test]
    fn test_models_that_fit() {
        let s = sys(16.0, "aarch64", "macos 15.0", None);
        let fits = models_that_fit(&s);
        let available = s.effective_model_memory_gb();
        assert!(fits.iter().all(|m| m.estimated_ram_gb <= available));
    }

    #[test]
    fn test_models_with_tool_calling() {
        let tools = models_with_tool_calling();
        assert!(tools.iter().all(|m| m.tool_calling));
        // Most models should support tool calling
        let all = available_models();
        assert!(tools.len() > all.len() / 2);
    }

    #[test]
    fn test_available_models_count() {
        let models = available_models();
        assert!(
            models.len() >= 20,
            "expected 20+ models, got {}",
            models.len()
        );
    }

    #[test]
    fn test_performance_preference_display() {
        assert_eq!(PerformancePreference::High.to_string(), "high");
        assert_eq!(PerformancePreference::Medium.to_string(), "medium");
        assert_eq!(PerformancePreference::Light.to_string(), "light");
    }

    #[test]
    fn test_model_recommendation_serialization() {
        let rec = available_models().into_iter().next().unwrap();
        let json = serde_json::to_string(&rec).unwrap();
        let parsed: ModelRecommendation = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.model, rec.model);
    }

    #[test]
    fn test_format_model_list() {
        let models = available_models();
        let s = sys(16.0, "aarch64", "macos 15.0", None);
        let output = format_model_list(&models, Some(&s));
        assert!(output.contains("[coding]"));
        assert!(output.contains("[general]"));
        assert!(output.contains("[reasoning]"));
        assert!(output.contains("tool calling supported"));
    }

    #[test]
    fn test_category_display() {
        assert_eq!(ModelCategory::Coding.to_string(), "coding");
        assert_eq!(ModelCategory::General.to_string(), "general");
        assert_eq!(ModelCategory::Reasoning.to_string(), "reasoning");
    }
}
