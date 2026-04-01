pub mod download;
pub mod recommend;
pub mod sysinfo_detect;

pub use recommend::{ModelCategory, ModelRecommendation, PerformancePreference};
pub use sysinfo_detect::SystemInfo;
