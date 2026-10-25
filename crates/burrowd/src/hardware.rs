//! What this machine can run, and which catalog model to recommend.
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Deserialize, Serialize, Clone)]
pub struct Model {
    pub id: String,
    pub name: String,
    pub repo: String,
    pub file: String,
    pub size: u64,
    pub sha256: String,
    pub min_ram_gb: u64,
    /// Context per agent slot, in tokens.
    pub ctx: u64,
}

pub fn catalog() -> Vec<Model> {
    serde_json::from_str(include_str!("../models.json")).expect("models.json is valid")
}

pub fn model(id: &str) -> Result<Model> {
    catalog().into_iter().find(|m| m.id == id).with_context(|| format!("`{id}` is not in the model catalog"))
}

pub fn ram_gb() -> f64 {
    let mut s = sysinfo::System::new();
    s.refresh_memory();
    s.total_memory() as f64 / (1u64 << 30) as f64
}

/// The largest model whose RAM tier fits. Half a GB of slack: an "8 GB" machine reports a bit less.
pub fn recommend(ram_gb: f64) -> Option<Model> {
    catalog().into_iter().filter(|m| ram_gb + 0.5 >= m.min_ram_gb as f64).max_by_key(|m| m.min_ram_gb)
}

pub fn detect() -> Value {
    let ram = ram_gb();
    // shortcut: GPU kind is a guess from the platform; the bundled Linux/Windows build is CPU or Vulkan, probe Vulkan in M5 onboarding.
    let gpu = if cfg!(all(target_os = "macos", target_arch = "aarch64")) { "metal" } else { "cpu" };
    json!({"ram_gb": (ram * 10.0).round() / 10.0, "cores": std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1),
        "os": std::env::consts::OS, "arch": std::env::consts::ARCH, "gpu": gpu, "recommended": recommend(ram).map(|m| m.id)})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recommendation_by_ram() {
        let id = |r: f64| recommend(r).map(|m| m.id);
        assert_eq!(id(4.0), None);
        assert_eq!(id(7.6).as_deref(), Some("qwen3-4b"));
        assert_eq!(id(18.0).as_deref(), Some("qwen3-8b"));
        assert_eq!(id(64.0).as_deref(), Some("qwen3-30b-a3b"));
        assert!(catalog().iter().all(|m| m.sha256.len() == 64 && m.file.ends_with(".gguf")));
    }
}
