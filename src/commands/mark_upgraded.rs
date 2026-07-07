//! `leiter soul mark-upgraded` — accept the current soul template version.
//!
//! The agent edits `soul.md` during an upgrade, but version metadata belongs
//! to leiter. This command is the explicit commit point: if the agent abandons
//! the restructure, the state remains old and `leiter soul upgrade` will keep
//! prompting instead of falsely marking the work complete.

use std::io::Write;
use std::path::Path;

use anyhow::{Result, bail};

use crate::config::load_config_best_effort;
use crate::paths;
use crate::sync::{SyncHomes, resync_and_warn};
use crate::templates::SOUL_TEMPLATE_VERSION;
use crate::validation::{ValidationStatus, validate_state};

/// Run the mark-upgraded command.
///
/// Validates the current state and soul, then records the binary's current
/// soul template version in `state.toml` with the same atomic writer used by
/// all state updates.
///
/// `claude_home_override`/`codex_home_override` exist only for testability:
/// production callers (`main.rs`) pass `None` so the opportunistic resync
/// below resolves the real default homes through
/// [`crate::paths::resolve_claude_home`]/[`crate::paths::resolve_codex_home`].
/// Neither home has a CLI flag on this command — SPEC.md does not document
/// one — so a non-`None` override only ever comes from a unit test injecting
/// a temp directory in place of the caller's real `~/.claude`/`~/.codex`.
pub fn run(
    state_dir: &Path,
    out: &mut impl Write,
    claude_home_override: Option<&Path>,
    codex_home_override: Option<&Path>,
) -> Result<()> {
    let (mut state, soul) = match validate_state(state_dir) {
        ValidationStatus::Incompatible(reason) => bail!("{}", reason.agent_message()),
        ValidationStatus::Compatible { state, soul, .. } => (state, soul),
    };

    state.soul_version = SOUL_TEMPLATE_VERSION;
    state.save(&paths::state_path(state_dir))?;

    writeln!(out, "soul_version set to {SOUL_TEMPLATE_VERSION}")?;
    let config = load_config_best_effort(state_dir);
    if let Some(claude_home) = paths::resolve_claude_home(claude_home_override) {
        let codex_home = paths::resolve_codex_home(codex_home_override, config.codex);
        resync_and_warn(
            out,
            state_dir,
            &mut state,
            &soul,
            SyncHomes {
                claude_home: Some(&claude_home),
                codex_home: codex_home.as_deref(),
            },
            config.codex,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::test_support::{
        bytes_to_string, setup_state_dir, update_state, write_state_with_epochs,
    };
    use crate::state::LeiterState;
    use crate::templates::{SETUP_HARD_EPOCH, SETUP_SOFT_EPOCH};
    use std::fs;

    fn run_mark_upgraded(state_dir: &Path) -> String {
        let mut out = Vec::new();
        run(state_dir, &mut out, None, None).unwrap();
        bytes_to_string(out)
    }

    #[test]
    fn sets_soul_version_to_current_template_version() {
        let tmp = setup_state_dir();
        update_state(tmp.path(), |state| state.soul_version = 0);

        let output = run_mark_upgraded(tmp.path());
        let state = LeiterState::load(&crate::paths::state_path(tmp.path())).unwrap();
        assert_eq!(state.soul_version, SOUL_TEMPLATE_VERSION);
        assert!(output.contains(&SOUL_TEMPLATE_VERSION.to_string()));
    }

    #[test]
    fn preserves_other_state_fields() {
        let tmp = setup_state_dir();
        let before = LeiterState::load(&crate::paths::state_path(tmp.path())).unwrap();

        run_mark_upgraded(tmp.path());

        let after = LeiterState::load(&crate::paths::state_path(tmp.path())).unwrap();
        assert_eq!(after.last_distilled, before.last_distilled);
        assert_eq!(after.setup_soft_epoch, before.setup_soft_epoch);
        assert_eq!(after.setup_hard_epoch, before.setup_hard_epoch);
    }

    #[test]
    fn missing_state_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let mut out = Vec::new();
        let err = run(tmp.path(), &mut out, None, None).unwrap_err();
        assert!(err.to_string().contains("not initialized"));
    }

    #[test]
    fn hard_epoch_mismatch_new_state_errors_without_writing_soul_version() {
        let tmp = tempfile::tempdir().unwrap();
        write_state_with_epochs(tmp.path(), SETUP_SOFT_EPOCH, SETUP_HARD_EPOCH + 1);
        update_state(tmp.path(), |state| state.soul_version = 0);

        let mut out = Vec::new();
        let err = run(tmp.path(), &mut out, None, None).unwrap_err();

        let state = LeiterState::load(&crate::paths::state_path(tmp.path())).unwrap();
        assert!(
            err.to_string()
                .contains("binary is older than your soul file")
        );
        assert_eq!(state.soul_version, 0);
        assert!(bytes_to_string(out).is_empty());
    }

    #[test]
    fn hard_epoch_mismatch_old_state_errors_without_writing_soul_version() {
        let tmp = tempfile::tempdir().unwrap();
        write_state_with_epochs(
            tmp.path(),
            SETUP_SOFT_EPOCH,
            SETUP_HARD_EPOCH.saturating_sub(1),
        );
        update_state(tmp.path(), |state| state.soul_version = 0);

        let mut out = Vec::new();
        let err = run(tmp.path(), &mut out, None, None).unwrap_err();

        let state = LeiterState::load(&crate::paths::state_path(tmp.path())).unwrap();
        assert!(err.to_string().contains("leiter claude install"));
        assert_eq!(state.soul_version, 0);
        assert!(bytes_to_string(out).is_empty());
    }

    #[test]
    fn corrupt_state_errors_without_rewriting_state() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(crate::paths::soul_path(tmp.path()), "body\n").unwrap();
        fs::write(crate::paths::state_path(tmp.path()), "not = valid = toml").unwrap();

        let mut out = Vec::new();
        let err = run(tmp.path(), &mut out, None, None).unwrap_err();

        assert!(err.to_string().contains("state file"));
        assert_eq!(
            fs::read_to_string(crate::paths::state_path(tmp.path())).unwrap(),
            "not = valid = toml"
        );
        assert!(bytes_to_string(out).is_empty());
    }

    /// Regression coverage for home-directory dependency injection: an
    /// injected override is the home the opportunistic resync heals. Proven the
    /// same way as the equivalent `mark_distilled` test — by observing healing
    /// land in the temp home we handed in, since `paths::resolve_claude_home`
    /// only reaches a real default when the override is `None`.
    #[test]
    fn opportunistic_resync_heals_stale_block_in_injected_home() {
        let tmp = setup_state_dir();
        let claude_home = tempfile::tempdir().unwrap();

        let soul_path = crate::paths::soul_path(tmp.path());
        let old_soul = fs::read_to_string(&soul_path).unwrap();
        let old_block = crate::managed_block::compose_block(&soul_path, &old_soul);
        fs::write(crate::paths::claude_md_path(claude_home.path()), &old_block).unwrap();
        update_state(tmp.path(), |state| {
            state.sync.claude_md = Some(crate::state::SyncHashes {
                soul_hash: crate::managed_block::sha256_hex(&old_soul),
                block_hash: crate::managed_block::sha256_hex(&old_block),
            });
        });
        fs::write(&soul_path, "new soul body\n").unwrap();

        let mut out = Vec::new();
        run(tmp.path(), &mut out, Some(claude_home.path()), None).unwrap();

        let healed = fs::read_to_string(crate::paths::claude_md_path(claude_home.path())).unwrap();
        assert!(healed.contains("new soul body"));
    }

    /// Opportunistic re-sync never performs a first install: a target with no
    /// recorded sync hashes must stay untouched, even in the injected home.
    #[test]
    fn opportunistic_resync_never_installs_a_never_synced_target() {
        let tmp = tempfile::tempdir().unwrap();
        write_state_with_epochs(tmp.path(), SETUP_SOFT_EPOCH, SETUP_HARD_EPOCH);
        let claude_home = tempfile::tempdir().unwrap();

        let mut out = Vec::new();
        run(tmp.path(), &mut out, Some(claude_home.path()), None).unwrap();

        assert!(!crate::paths::claude_md_path(claude_home.path()).exists());
    }
}
