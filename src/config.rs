//! Persistent user configuration stored under `~/.leiter/leiter.toml`.

use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Deserializer, Serialize};
use tracing::warn;

use crate::fs_atomic::write_atomic;

/// User-visible leiter settings.
///
/// Saving rewrites legacy aliases into the current shape. `agent_command` is
/// omitted when unset so the default distill agent stays implicit, while the
/// retention threshold is always materialized with its effective value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LeiterConfig {
    /// Gate for Codex rollout distillation and AGENTS.md soul delivery.
    pub codex: bool,
    /// Whole command line used by `leiter distill` instead of the built-in
    /// Claude invocation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_command: Option<Vec<String>>,
    /// Age threshold, in days, for warning about undistilled Claude sessions
    /// that are nearing Claude Code's external transcript retention window.
    pub retention_warn_days: u32,
}

impl Default for LeiterConfig {
    fn default() -> Self {
        Self {
            codex: false,
            agent_command: None,
            retention_warn_days: default_retention_warn_days(),
        }
    }
}

impl<'de> Deserialize<'de> for LeiterConfig {
    /// Accept the legacy `enable_codex_experimental` alias without tripping
    /// serde's duplicate-field rejection.
    ///
    /// A plain `#[serde(alias = ...)]` treats the modern and legacy keys as the
    /// same field, so a `leiter.toml` carrying *both* fails to parse — and
    /// every `load_config_best_effort` caller then silently downgrades to
    /// defaults, quietly disabling Codex for a user who has it on. Deserializing
    /// through a raw two-`Option` struct lets both keys coexist: the modern
    /// `codex` wins when they disagree (with a warning naming the ignored legacy
    /// key), and the legacy key is still honored when it is the only one
    /// present. Saving always writes just `codex`, so the collision heals itself
    /// the next time leiter persists the config.
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Raw {
            #[serde(default)]
            codex: Option<bool>,
            #[serde(default)]
            enable_codex_experimental: Option<bool>,
            #[serde(default)]
            agent_command: Option<Vec<String>>,
            #[serde(default = "default_retention_warn_days")]
            retention_warn_days: u32,
        }

        let raw = Raw::deserialize(deserializer)?;
        let codex = match (raw.codex, raw.enable_codex_experimental) {
            (Some(codex), Some(_legacy)) => {
                warn!(
                    "leiter.toml sets both `codex` and the legacy `enable_codex_experimental`; \
                     honoring `codex` and ignoring `enable_codex_experimental`"
                );
                codex
            }
            (Some(codex), None) => codex,
            (None, Some(legacy)) => legacy,
            (None, None) => false,
        };
        Ok(Self {
            codex,
            agent_command: raw.agent_command,
            retention_warn_days: raw.retention_warn_days,
        })
    }
}

fn default_retention_warn_days() -> u32 {
    21
}

/// Load `leiter.toml`, warning and falling back to defaults on any failure.
///
/// State-mutating commands that only consult `codex` share this instead of
/// propagating the load error: SPEC.md specifies warn-and-default for the
/// config-load step of `config set`, `distill`, `mark-distilled`, and friends,
/// so a malformed or unreadable `leiter.toml` must never block a soul or state
/// operation. Kept here (rather than duplicated per command) so the warning
/// text and the default stay in one place.
pub fn load_config_best_effort(state_dir: &Path) -> LeiterConfig {
    let path = crate::paths::leiter_config_path(state_dir);
    match LeiterConfig::load(&path) {
        Ok(config) => config,
        Err(err) => {
            warn!("failed to load leiter config, using defaults: {err}");
            LeiterConfig::default()
        }
    }
}

impl LeiterConfig {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }

        let raw = std::fs::read_to_string(path)
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
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;

        let serialized = toml::to_string_pretty(self).context("failed to serialize config")?;
        write_atomic(path, serialized.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_uses_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        let config = LeiterConfig::load(&tmp.path().join("leiter.toml")).unwrap();
        assert!(!config.codex);
        assert_eq!(config.agent_command, None);
        assert_eq!(config.retention_warn_days, 21);
    }

    #[test]
    fn save_round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("leiter.toml");

        let config = LeiterConfig {
            codex: true,
            ..Default::default()
        };
        config.save(&path).unwrap();

        let loaded = LeiterConfig::load(&path).unwrap();
        assert_eq!(loaded, config);
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("codex = true")
        );
        assert!(
            !std::fs::read_to_string(&path)
                .unwrap()
                .contains("enable_codex_experimental")
        );
    }

    #[test]
    fn agent_command_and_retention_days_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("leiter.toml");

        let config = LeiterConfig {
            codex: false,
            agent_command: Some(vec!["/tmp/fake-agent".to_string(), "--flag".to_string()]),
            retention_warn_days: 14,
        };
        config.save(&path).unwrap();

        let loaded = LeiterConfig::load(&path).unwrap();
        assert_eq!(loaded, config);
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("retention_warn_days = 14"));
        assert!(raw.contains("agent_command = ["));
    }

    #[test]
    fn legacy_codex_key_is_accepted() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("leiter.toml");
        std::fs::write(&path, "enable_codex_experimental = true\n").unwrap();

        let loaded = LeiterConfig::load(&path).unwrap();
        assert!(loaded.codex);
    }

    /// A file carrying both keys must parse (serde's duplicate-field rejection
    /// used to swallow it), with the modern `codex` value winning. Both
    /// disagreement directions are covered so neither is silently inverted.
    #[test]
    fn both_keys_present_codex_wins_over_legacy() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("leiter.toml");

        std::fs::write(&path, "codex = true\nenable_codex_experimental = false\n").unwrap();
        assert!(LeiterConfig::load(&path).unwrap().codex);

        std::fs::write(&path, "codex = false\nenable_codex_experimental = true\n").unwrap();
        assert!(!LeiterConfig::load(&path).unwrap().codex);
    }
}
