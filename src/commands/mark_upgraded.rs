//! `leiter soul mark-upgraded` — accept the current soul template version.
//!
//! The agent edits `soul.md` during an upgrade, but version metadata belongs
//! to leiter. This command is the explicit commit point: if the agent abandons
//! the restructure, the state remains old and `leiter soul upgrade` will keep
//! prompting instead of falsely marking the work complete.

use std::io::Write;
use std::path::Path;

use anyhow::{Result, bail};

use crate::paths;
use crate::templates::SOUL_TEMPLATE_VERSION;
use crate::validation::{ValidationStatus, validate_state};

/// Run the mark-upgraded command.
///
/// Validates the current state and soul, then records the binary's current
/// soul template version in `state.toml` with the same atomic writer used by
/// all state updates.
pub fn run(state_dir: &Path, out: &mut impl Write) -> Result<()> {
    let mut state = match validate_state(state_dir) {
        ValidationStatus::Incompatible(reason) => bail!("{}", reason.agent_message()),
        ValidationStatus::Compatible { state, .. } => state,
    };

    state.soul_version = SOUL_TEMPLATE_VERSION;
    state.save(&paths::state_path(state_dir))?;

    writeln!(out, "soul_version set to {SOUL_TEMPLATE_VERSION}")?;
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
        run(state_dir, &mut out).unwrap();
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
        let err = run(tmp.path(), &mut out).unwrap_err();
        assert!(err.to_string().contains("not initialized"));
    }

    #[test]
    fn hard_epoch_mismatch_new_state_errors_without_writing_soul_version() {
        let tmp = tempfile::tempdir().unwrap();
        write_state_with_epochs(tmp.path(), SETUP_SOFT_EPOCH, SETUP_HARD_EPOCH + 1);
        update_state(tmp.path(), |state| state.soul_version = 0);

        let mut out = Vec::new();
        let err = run(tmp.path(), &mut out).unwrap_err();

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
        let err = run(tmp.path(), &mut out).unwrap_err();

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
        let err = run(tmp.path(), &mut out).unwrap_err();

        assert!(err.to_string().contains("state file"));
        assert_eq!(
            fs::read_to_string(crate::paths::state_path(tmp.path())).unwrap(),
            "not = valid = toml"
        );
        assert!(bytes_to_string(out).is_empty());
    }
}
