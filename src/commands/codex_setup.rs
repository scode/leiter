//! `leiter codex install` / `leiter codex uninstall`.
//!
//! Codex support is controlled by `codex = true|false` in `leiter.toml`.
//! Install flips the gate on and writes `AGENTS.md`; uninstall removes the
//! managed block regardless of the current gate so cleanup still works after a
//! manual config change.

use std::io::Write;
use std::path::Path;

use anyhow::{Result, bail};

use crate::config::load_config_best_effort;
use crate::managed_block::remove_managed_block;
use crate::paths;
use crate::sync::{SyncHomes, SyncSelection, resync_and_warn, sync_blocks_selected};
use crate::validation::{ValidationStatus, validate_state};

/// Enable Codex support and write the managed `AGENTS.md` block.
///
/// `claude_home_override` exists only for testability: production callers
/// (`main.rs`) pass `None` so the opportunistic Claude resync below resolves
/// the real default home through [`crate::paths::resolve_claude_home`].
/// `leiter codex install` has no `--claude-home` flag — SPEC.md documents
/// only `--codex-home` here — so a non-`None` override only ever comes from a
/// unit test injecting a temp directory in place of the caller's real
/// `~/.claude`.
pub fn install(
    state_dir: &Path,
    codex_home: &Path,
    claude_home_override: Option<&Path>,
    out: &mut impl Write,
) -> Result<()> {
    let (mut state, soul) = match validate_state(state_dir) {
        ValidationStatus::Compatible { state, soul, .. } => (state, soul),
        ValidationStatus::Incompatible(reason) => bail!("{}", reason.user_message()),
    };

    let mut config = load_config_best_effort(state_dir);
    config.codex = true;
    config.save(&paths::leiter_config_path(state_dir))?;

    let outcomes = sync_blocks_selected(
        state_dir,
        &mut state,
        &soul,
        // Codex install only writes AGENTS.md; there is no Claude home in play
        // here, so pass `None` rather than a placeholder.
        SyncHomes {
            claude_home: None,
            codex_home: Some(codex_home),
        },
        SyncSelection {
            claude_md: false,
            agents_md: true,
        },
        false,
    )?;
    if let Some(claude_home) = paths::resolve_claude_home(claude_home_override) {
        resync_and_warn(
            out,
            state_dir,
            &mut state,
            &soul,
            SyncHomes {
                claude_home: Some(&claude_home),
                codex_home: Some(codex_home),
            },
            true,
        )?;
    }

    writeln!(out, "codex set to true")?;
    // The selection above only reconciles AGENTS.md, so every outcome here is
    // for that target.
    for outcome in &outcomes {
        writeln!(out, "{}", outcome.line())?;
    }
    if outcomes.iter().any(|outcome| outcome.is_refused()) {
        bail!(
            "AGENTS.md managed block was refused; rerun `leiter sync --force` after enabling Codex to overwrite hand edits"
        );
    }
    Ok(())
}

/// Disable Codex support and remove the managed `AGENTS.md` block if present.
pub fn uninstall(state_dir: &Path, codex_home: &Path, out: &mut impl Write) -> Result<()> {
    match validate_state(state_dir) {
        ValidationStatus::Incompatible(reason) => bail!("{}", reason.user_message()),
        ValidationStatus::Compatible { .. } => {}
    }

    let agents_md = paths::agents_md_path(codex_home);
    let removed = remove_managed_block(&agents_md)?;

    let mut config = load_config_best_effort(state_dir);
    config.codex = false;
    config.save(&paths::leiter_config_path(state_dir))?;

    match removed {
        crate::managed_block::RemoveOutcome::Removed => {
            writeln!(out, "codex set to false; AGENTS.md managed block removed")?;
        }
        crate::managed_block::RemoveOutcome::NotPresent => {
            writeln!(
                out,
                "codex set to false; no AGENTS.md managed block was present"
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::test_support::{bytes_to_string, setup_state_dir};
    use crate::config::LeiterConfig;
    use crate::managed_block::BEGIN_SENTINEL;

    #[test]
    fn install_sets_config_and_writes_agents_block() {
        let tmp = setup_state_dir();
        let codex_tmp = tempfile::tempdir().unwrap();
        let mut out = Vec::new();

        install(tmp.path(), codex_tmp.path(), None, &mut out).unwrap();

        let output = bytes_to_string(out);
        assert!(output.contains("codex set to true"));
        assert!(paths::agents_md_path(codex_tmp.path()).is_file());
        assert!(
            std::fs::read_to_string(paths::agents_md_path(codex_tmp.path()))
                .unwrap()
                .contains(BEGIN_SENTINEL)
        );
        assert!(
            LeiterConfig::load(&paths::leiter_config_path(tmp.path()))
                .unwrap()
                .codex
        );
    }

    #[test]
    fn uninstall_removes_block_even_when_config_already_false() {
        let tmp = setup_state_dir();
        let codex_tmp = tempfile::tempdir().unwrap();
        let mut out = Vec::new();
        install(tmp.path(), codex_tmp.path(), None, &mut out).unwrap();
        LeiterConfig { codex: false }
            .save(&paths::leiter_config_path(tmp.path()))
            .unwrap();

        let mut uninstall_out = Vec::new();
        uninstall(tmp.path(), codex_tmp.path(), &mut uninstall_out).unwrap();

        let agents = std::fs::read_to_string(paths::agents_md_path(codex_tmp.path())).unwrap();
        assert!(!agents.contains(BEGIN_SENTINEL));
        assert!(
            !LeiterConfig::load(&paths::leiter_config_path(tmp.path()))
                .unwrap()
                .codex
        );
    }

    /// Regression coverage for home-directory dependency injection: install's
    /// opportunistic Claude resync (distinct from the `AGENTS.md` write, which
    /// always targets the required `codex_home` argument) must operate on the
    /// injected `claude_home_override`, not silently fall back to a real
    /// `~/.claude` — `setup_state_dir` already synced a `CLAUDE.md` into its
    /// own temp Claude home with recorded hashes, so staling the soul here
    /// gives opportunistic resync something to heal.
    #[test]
    fn opportunistic_claude_resync_heals_the_injected_claude_home() {
        let tmp = setup_state_dir();
        let codex_tmp = tempfile::tempdir().unwrap();
        std::fs::write(paths::soul_path(tmp.path()), "new soul body\n").unwrap();

        let mut out = Vec::new();
        install(
            tmp.path(),
            codex_tmp.path(),
            Some(tmp.claude.path()),
            &mut out,
        )
        .unwrap();

        let healed = std::fs::read_to_string(paths::claude_md_path(tmp.claude.path())).unwrap();
        assert!(healed.contains("new soul body"));
    }
}
