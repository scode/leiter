//! `leiter config` — read and write user settings.

use std::io::Write;
use std::path::Path;

use anyhow::{Result, bail};

use crate::config::load_config_best_effort;
use crate::paths;
use crate::sync::{SyncHomes, resync_and_warn};
use crate::validation::{ValidationStatus, validate_state};

/// Run the `leiter config set` command.
///
/// `claude_home_override`/`codex_home_override` exist only for testability:
/// production callers (`main.rs`) pass `None` so the opportunistic resync
/// below resolves the real default homes through
/// [`crate::paths::resolve_claude_home`]/[`crate::paths::resolve_codex_home`].
/// Neither home has a CLI flag on this command — SPEC.md does not document
/// one — so a non-`None` override only ever comes from a unit test injecting
/// a temp directory in place of the caller's real `~/.claude`/`~/.codex`.
pub fn set(
    state_dir: &Path,
    out: &mut impl Write,
    key: &str,
    value: &str,
    claude_home_override: Option<&Path>,
    codex_home_override: Option<&Path>,
) -> Result<()> {
    let (mut state, soul) = match validate_state(state_dir) {
        ValidationStatus::Compatible { state, soul, .. } => (state, soul),
        ValidationStatus::Incompatible(reason) => bail!("{}", reason.user_message()),
    };

    let config_path = paths::leiter_config_path(state_dir);
    let mut config = load_config_best_effort(state_dir);

    let legacy_key = key == "enable_codex_experimental";
    match key {
        "codex" | "enable_codex_experimental" => {
            config.codex = parse_bool(value)?;
        }
        _ => bail!("unknown config key: {key}"),
    }

    let claude_home = paths::resolve_claude_home(claude_home_override);
    let codex_home = paths::resolve_codex_home(codex_home_override, config.codex);
    if let Some(claude_home) = claude_home.as_deref() {
        resync_and_warn(
            out,
            state_dir,
            &mut state,
            &soul,
            SyncHomes {
                claude_home: Some(claude_home),
                codex_home: codex_home.as_deref(),
            },
            config.codex,
        )?;
    }

    config.save(&config_path)?;
    writeln!(out, "codex set to {}", config.codex)?;
    if legacy_key {
        writeln!(
            out,
            "enable_codex_experimental is deprecated; use codex instead"
        )?;
    }
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
    use crate::config::LeiterConfig;
    use std::fs;

    #[test]
    fn set_updates_config_file() {
        let tmp = setup_state_dir();
        let mut out = Vec::new();

        set(tmp.path(), &mut out, "codex", "true", None, None).unwrap();

        let output = bytes_to_string(out);
        assert_eq!(output, "codex set to true\n");

        let config = LeiterConfig::load(&paths::leiter_config_path(tmp.path())).unwrap();
        assert!(config.codex);
        assert!(
            fs::read_to_string(paths::leiter_config_path(tmp.path()))
                .unwrap()
                .contains("codex = true")
        );
    }

    #[test]
    fn invalid_value_errors() {
        let tmp = setup_state_dir();
        let mut out = Vec::new();
        let err = set(tmp.path(), &mut out, "codex", "yes", None, None).unwrap_err();
        assert!(err.to_string().contains("expected boolean value"));
    }

    #[test]
    fn legacy_key_sets_codex_and_prints_deprecation_note() {
        let tmp = setup_state_dir();
        let mut out = Vec::new();

        set(
            tmp.path(),
            &mut out,
            "enable_codex_experimental",
            "true",
            None,
            None,
        )
        .unwrap();

        let output = bytes_to_string(out);
        assert!(output.contains("codex set to true"));
        assert!(output.contains("deprecated"));
        let raw = fs::read_to_string(paths::leiter_config_path(tmp.path())).unwrap();
        assert!(raw.contains("codex = true"));
        assert!(!raw.contains("enable_codex_experimental"));
    }

    /// Regression coverage for home-directory dependency injection: an
    /// injected override is the home the opportunistic resync heals, proving
    /// `set()` actually threads the override through rather than falling back to
    /// a real `~/.claude` whenever one happens to exist.
    #[test]
    fn opportunistic_resync_heals_stale_block_in_injected_home() {
        let tmp = setup_state_dir();
        let claude_home = tempfile::tempdir().unwrap();

        let soul_path = paths::soul_path(tmp.path());
        let old_soul = fs::read_to_string(&soul_path).unwrap();
        let old_block = crate::managed_block::compose_block(&soul_path, &old_soul);
        fs::write(paths::claude_md_path(claude_home.path()), &old_block).unwrap();
        crate::commands::test_support::update_state(tmp.path(), |state| {
            state.sync.claude_md = Some(crate::state::SyncHashes {
                soul_hash: crate::managed_block::sha256_hex(&old_soul),
                block_hash: crate::managed_block::sha256_hex(&old_block),
            });
        });
        fs::write(&soul_path, "new soul body\n").unwrap();

        let mut out = Vec::new();
        set(
            tmp.path(),
            &mut out,
            "codex",
            "true",
            Some(claude_home.path()),
            None,
        )
        .unwrap();

        let healed = fs::read_to_string(paths::claude_md_path(claude_home.path())).unwrap();
        assert!(healed.contains("new soul body"));
    }
}
