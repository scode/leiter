//! Persistent user configuration stored under `~/.leiter/leiter.toml`.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// User-visible leiter settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct LeiterConfig {
    /// Experimental gate for Codex rollout distillation.
    #[serde(default)]
    pub enable_codex_experimental: bool,
}

impl LeiterConfig {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }

        let raw = fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        toml::from_str(&raw).with_context(|| format!("failed to parse {}", path.display()))
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let parent = path.parent().with_context(|| {
            format!(
                "config path must have a parent directory: {}",
                path.display()
            )
        })?;
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;

        let serialized = toml::to_string_pretty(self).context("failed to serialize config")?;
        fs::write(path, serialized).with_context(|| format!("failed to write {}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_uses_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        let config = LeiterConfig::load(&tmp.path().join("leiter.toml")).unwrap();
        assert!(!config.enable_codex_experimental);
    }

    #[test]
    fn save_round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("leiter.toml");

        let config = LeiterConfig {
            enable_codex_experimental: true,
        };
        config.save(&path).unwrap();

        let loaded = LeiterConfig::load(&path).unwrap();
        assert_eq!(loaded, config);
    }
}
