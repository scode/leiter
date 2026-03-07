//! `leiter config` — read and write user settings.

use std::io::Write;
use std::path::Path;

use anyhow::{Result, bail};
use tracing::warn;

use crate::config::LeiterConfig;
use crate::paths;

pub fn set(state_dir: &Path, out: &mut impl Write, key: &str, value: &str) -> Result<()> {
    let config_path = paths::leiter_config_path(state_dir);
    let mut config = match LeiterConfig::load(&config_path) {
        Ok(config) => config,
        Err(err) => {
            warn!("failed to load existing config, rewriting with defaults: {err}");
            LeiterConfig::default()
        }
    };

    match key {
        "enable_codex_experimental" => {
            config.enable_codex_experimental = parse_bool(value)?;
        }
        _ => bail!("unknown config key: {key}"),
    }

    config.save(&config_path)?;
    writeln!(out, "{key} set to {value}")?;
    Ok(())
}

fn parse_bool(value: &str) -> Result<bool> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => bail!("expected boolean value 'true' or 'false', got: {value}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::test_support::{bytes_to_string, setup_state_dir};

    #[test]
    fn set_updates_config_file() {
        let tmp = setup_state_dir();
        let mut out = Vec::new();

        set(tmp.path(), &mut out, "enable_codex_experimental", "true").unwrap();

        let output = bytes_to_string(out);
        assert_eq!(output, "enable_codex_experimental set to true\n");

        let config = LeiterConfig::load(&paths::leiter_config_path(tmp.path())).unwrap();
        assert!(config.enable_codex_experimental);
    }

    #[test]
    fn invalid_value_errors() {
        let tmp = setup_state_dir();
        let mut out = Vec::new();
        let err = set(tmp.path(), &mut out, "enable_codex_experimental", "yes").unwrap_err();
        assert!(err.to_string().contains("expected boolean value"));
    }
}
