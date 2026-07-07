//! `leiter hook context` — retired SessionStart hook tombstone.
//!
//! This command survives for one release so old Claude Code settings still
//! route users to the hookless migration. It never injects the soul; the
//! managed `CLAUDE.md` block is the only delivery path now.

use std::io::Write;
use std::path::Path;

use anyhow::Result;

use crate::templates::HOOK_CONTEXT_RETIRED_MESSAGE;
use crate::validation::{ValidationStatus, validate_state};

/// Run the context command.
///
/// Validates enough state to decide which tombstone message to print.
///
/// Always exits successfully — the SessionStart hook should never fail the
/// session.
pub fn run(state_dir: &Path, out: &mut impl Write) -> Result<()> {
    match validate_state(state_dir) {
        ValidationStatus::Incompatible(reason) => {
            writeln!(out, "{}", reason.agent_message())?;
        }
        ValidationStatus::Compatible { .. } => {
            write!(out, "{HOOK_CONTEXT_RETIRED_MESSAGE}")?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::agent_setup;
    use crate::commands::test_support::write_state_with_epochs;
    use crate::paths;
    use crate::templates::{HOOK_CONTEXT_RETIRED_MESSAGE, SETUP_HARD_EPOCH, SETUP_SOFT_EPOCH};
    use std::fs;

    fn run_context(state_dir: &Path) -> String {
        let mut out = Vec::new();
        run(state_dir, &mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    fn setup_and_context(state_dir: &Path) -> String {
        let claude_tmp = tempfile::tempdir().unwrap();
        let codex_tmp = tempfile::tempdir().unwrap();
        agent_setup::run(
            state_dir,
            claude_tmp.path(),
            codex_tmp.path(),
            &mut Vec::new(),
        )
        .unwrap();
        run_context(state_dir)
    }

    #[test]
    fn healthy_state_outputs_retired_hook_one_liner() {
        let tmp = tempfile::tempdir().unwrap();
        let output = setup_and_context(tmp.path());
        assert_eq!(output, HOOK_CONTEXT_RETIRED_MESSAGE);
    }

    #[test]
    fn without_soul_suggests_agent_setup() {
        let tmp = tempfile::tempdir().unwrap();
        let output = run_context(tmp.path());
        assert!(output.contains("not initialized"));
        assert!(output.contains("leiter claude install"));
    }

    #[test]
    fn healthy_state_does_not_output_soul_content() {
        let tmp = tempfile::tempdir().unwrap();
        let output = setup_and_context(tmp.path());
        assert!(!output.contains("# Communication Style"));
    }

    #[test]
    fn legacy_layout_outputs_migration_message_without_soul() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        fs::write(
            paths::soul_path(dir),
            "---\nlast_distilled: 2026-01-01T00:00:00Z\nsoul_version: 1\nsetup_soft_epoch: 1\nsetup_hard_epoch: 1\n---\nlegacy body\n",
        )
        .unwrap();
        let output = run_context(dir);

        assert!(output.contains("Leiter has moved to hookless operation"));
        assert!(!output.contains("legacy body"));
    }

    #[test]
    fn hard_epoch_mismatch_old_soul_blocks() {
        let tmp = tempfile::tempdir().unwrap();
        write_state_with_epochs(
            tmp.path(),
            SETUP_SOFT_EPOCH,
            SETUP_HARD_EPOCH.saturating_sub(1),
        );
        let output = run_context(tmp.path());
        assert!(output.contains("ACTION REQUIRED"));
        assert!(output.contains("leiter claude install"));
        assert!(!output.contains("# Communication Style"));
    }

    #[test]
    fn hard_epoch_mismatch_new_soul_blocks() {
        let tmp = tempfile::tempdir().unwrap();
        write_state_with_epochs(tmp.path(), SETUP_SOFT_EPOCH, SETUP_HARD_EPOCH + 1);
        let output = run_context(tmp.path());
        assert!(output.contains("ACTION REQUIRED"));
        assert!(output.contains("binary is older than your soul file"));
        assert!(!output.contains("# Communication Style"));
    }

    #[test]
    fn soft_epoch_mismatch_old_soul_still_only_outputs_retired_hook_one_liner() {
        let tmp = tempfile::tempdir().unwrap();
        write_state_with_epochs(
            tmp.path(),
            SETUP_SOFT_EPOCH.saturating_sub(1),
            SETUP_HARD_EPOCH,
        );
        let output = run_context(tmp.path());
        assert_eq!(output, HOOK_CONTEXT_RETIRED_MESSAGE);
    }

    #[test]
    fn soft_epoch_mismatch_new_soul_still_only_outputs_retired_hook_one_liner() {
        let tmp = tempfile::tempdir().unwrap();
        write_state_with_epochs(tmp.path(), SETUP_SOFT_EPOCH + 1, SETUP_HARD_EPOCH);
        let output = run_context(tmp.path());
        assert_eq!(output, HOOK_CONTEXT_RETIRED_MESSAGE);
    }

    #[test]
    fn corrupt_state_blocks_soul_injection() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        fs::create_dir_all(dir).unwrap();
        fs::write(paths::soul_path(dir), "body\n").unwrap();
        let state_path = paths::state_path(dir);
        fs::write(&state_path, "not = valid = toml").unwrap();
        let output = run_context(dir);
        assert!(output.contains("ACTION REQUIRED"));
        assert!(output.contains("state file"));
        assert!(output.contains(&state_path.display().to_string()));
        assert!(!output.contains("body\n"));
    }

    #[test]
    fn matching_epochs_no_warnings() {
        let tmp = tempfile::tempdir().unwrap();
        let output = setup_and_context(tmp.path());
        assert!(!output.contains("incompatible"));
        assert!(!output.contains("optional improvements"));
        assert!(!output.contains("a bit behind"));
        assert_eq!(output, HOOK_CONTEXT_RETIRED_MESSAGE);
    }
}
