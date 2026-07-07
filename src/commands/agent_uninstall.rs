//! `leiter claude uninstall` — removes leiter plugin files from `~/.claude/`.
//!
//! For each known skill, checks for the sentinel marker in its SKILL.md and
//! removes that skill's directory only if the sentinel is present. Also removes
//! the managed `CLAUDE.md` block without touching `~/.leiter/`.

use std::fs;
use std::io::Write;
use std::path::Path;

use anyhow::{Result, bail};
use tracing::{info, warn};

use crate::managed_block::{RemoveOutcome, remove_managed_block};
use crate::paths;
use crate::templates::{LEGACY_SKILL_DIRS, PLUGIN_SENTINEL, SKILL_CONTENTS};
use crate::validation::{ValidationStatus, validate_state};

/// Run the `leiter claude uninstall` command.
///
/// For each known skill, checks whether the sentinel is present in its
/// SKILL.md and removes that skill's directory only if it is. This ensures
/// we never delete a directory we haven't verified ownership of.
pub fn run(state_dir: &Path, claude_home: &Path) -> Result<()> {
    match validate_state(state_dir) {
        ValidationStatus::Incompatible(reason) => bail!("{}", reason.user_message()),
        ValidationStatus::Compatible { .. } => {}
    }

    let mut removed = 0;
    let mut failed: Vec<String> = Vec::new();

    // Each skill is checked and removed independently. The sentinel check
    // and the removal must use the same `skill_dir` to guarantee we only
    // delete directories whose SKILL.md we verified.
    for name in owned_skill_names() {
        let skill_dir = paths::skill_dir(claude_home, name);
        let skill_md = skill_dir.join("SKILL.md");

        let has_sentinel = fs::read_to_string(&skill_md)
            .map(|content| content.contains(PLUGIN_SENTINEL))
            .unwrap_or(false);
        if !has_sentinel {
            continue;
        }

        match fs::remove_dir_all(&skill_dir) {
            Ok(()) => {
                info!("removed {}", skill_dir.display());
                removed += 1;
            }
            Err(e) => {
                warn!("failed to remove {}: {e}", skill_dir.display());
                failed.push(skill_dir.display().to_string());
            }
        }
    }

    // Remove the managed block BEFORE deciding whether leiter was installed at
    // all. The block is independent evidence of an install: a rerun after a
    // partial uninstall (skills already gone, but an earlier error left the
    // block behind) must still finish the job. Ordering the removal ahead of
    // the "nothing to uninstall" bail is what lets that rerun succeed.
    let block_removed = matches!(
        remove_managed_block(&paths::claude_md_path(claude_home))?,
        RemoveOutcome::Removed
    );

    if removed == 0 && failed.is_empty() && !block_removed {
        bail!("no leiter skill files with sentinel found; nothing to uninstall");
    }

    let dir = state_dir.display();
    if failed.is_empty() {
        info!("Leiter plugin files removed");
    } else {
        warn!("Leiter plugin files partially removed");
        warn!("Failed to remove:");
        for path in &failed {
            warn!("  {path}");
        }
    }
    info!(
        "To remove hooks, run `leiter claude agent-teardown-instructions` in a Claude Code session and follow the output (or manually edit ~/.claude/settings.json)"
    );
    info!("To completely remove leiter, also delete `{dir}/` and uninstall the binary");
    info!("To re-enable leiter later, run `leiter claude install`");

    if !failed.is_empty() {
        bail!("failed to remove some plugin directories");
    }

    Ok(())
}

fn owned_skill_names() -> impl Iterator<Item = &'static str> {
    SKILL_CONTENTS
        .iter()
        .map(|(name, _)| *name)
        .chain(LEGACY_SKILL_DIRS.iter().copied())
}

/// Output the agent-teardown instructions (hook removal).
///
/// Used by `leiter claude agent-teardown-instructions`.
pub fn agent_teardown_instructions(state_dir: &Path, out: &mut impl Write) -> Result<()> {
    match validate_state(state_dir) {
        ValidationStatus::Incompatible(reason) => bail!("{}", reason.agent_message()),
        ValidationStatus::Compatible { .. } => {}
    }
    write!(
        out,
        "{}",
        crate::templates::agent_uninstall_instructions(state_dir)
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::agent_setup;
    use crate::commands::test_support::write_state_with_epochs;
    use crate::templates::{SETUP_HARD_EPOCH, SETUP_SOFT_EPOCH};

    fn setup_plugin_files(claude_home: &Path, state_dir: &Path) {
        let codex_tmp = tempfile::tempdir().unwrap();
        agent_setup::run(state_dir, claude_home, codex_tmp.path(), &mut Vec::new()).unwrap();
    }

    fn run_uninstall(state_dir: &Path, claude_home: &Path) -> Result<()> {
        run(state_dir, claude_home)
    }

    #[test]
    fn uninstall_removes_skill_dirs() {
        let state_tmp = tempfile::tempdir().unwrap();
        let claude_tmp = tempfile::tempdir().unwrap();
        setup_plugin_files(claude_tmp.path(), state_tmp.path());

        run_uninstall(state_tmp.path(), claude_tmp.path()).unwrap();

        for (name, _) in SKILL_CONTENTS {
            assert!(
                !paths::skill_dir(claude_tmp.path(), name).exists(),
                "skill dir {name} should be removed"
            );
        }
    }

    #[test]
    fn uninstall_does_not_touch_state_dir() {
        let state_tmp = tempfile::tempdir().unwrap();
        let claude_tmp = tempfile::tempdir().unwrap();
        setup_plugin_files(claude_tmp.path(), state_tmp.path());

        // Plant a legacy logs dir to prove uninstall leaves ~/.leiter alone
        // (fresh installs no longer create it, so the fixture does).
        fs::create_dir_all(paths::logs_dir(state_tmp.path())).unwrap();

        run_uninstall(state_tmp.path(), claude_tmp.path()).unwrap();

        assert!(paths::soul_path(state_tmp.path()).is_file());
        assert!(paths::logs_dir(state_tmp.path()).is_dir());
    }

    #[test]
    fn uninstall_removes_claude_block_and_preserves_unrelated_content() {
        let state_tmp = tempfile::tempdir().unwrap();
        let claude_tmp = tempfile::tempdir().unwrap();
        fs::write(paths::claude_md_path(claude_tmp.path()), "before\n").unwrap();
        setup_plugin_files(claude_tmp.path(), state_tmp.path());
        let claude_md = paths::claude_md_path(claude_tmp.path());
        let with_after = format!("{}after\n", fs::read_to_string(&claude_md).unwrap());
        fs::write(&claude_md, with_after).unwrap();

        run_uninstall(state_tmp.path(), claude_tmp.path()).unwrap();

        let updated = fs::read_to_string(&claude_md).unwrap();
        assert_eq!(updated, "before\nafter\n");
    }

    #[test]
    fn uninstall_fails_without_soul() {
        let claude_tmp = tempfile::tempdir().unwrap();
        let state_tmp = tempfile::tempdir().unwrap();
        let err = run_uninstall(state_tmp.path(), claude_tmp.path()).unwrap_err();
        assert!(err.to_string().contains("not initialized"));
    }

    #[test]
    fn uninstall_fails_without_skill_files() {
        let state_tmp = tempfile::tempdir().unwrap();
        let claude_tmp = tempfile::tempdir().unwrap();
        setup_plugin_files(claude_tmp.path(), state_tmp.path());

        // Remove all skill dirs so none have the sentinel.
        for (name, _) in SKILL_CONTENTS {
            let skill_dir = paths::skill_dir(claude_tmp.path(), name);
            if skill_dir.exists() {
                fs::remove_dir_all(&skill_dir).unwrap();
            }
        }
        // Also drop the managed block: with skills gone AND no block, nothing
        // leiter-owned remains, so the command must report there is nothing to
        // uninstall.
        fs::remove_file(paths::claude_md_path(claude_tmp.path())).unwrap();

        let err = run_uninstall(state_tmp.path(), claude_tmp.path()).unwrap_err();
        assert!(err.to_string().contains("nothing to uninstall"));
    }

    /// A rerun after a partial uninstall — skills already gone, but the managed
    /// block still on disk from an earlier failed run — must still remove the
    /// block instead of bailing with "nothing to uninstall".
    #[test]
    fn uninstall_rerun_removes_leftover_block_after_skills_gone() {
        let state_tmp = tempfile::tempdir().unwrap();
        let claude_tmp = tempfile::tempdir().unwrap();
        setup_plugin_files(claude_tmp.path(), state_tmp.path());

        // Simulate the partial-uninstall state: skills removed, block left.
        for (name, _) in SKILL_CONTENTS {
            let skill_dir = paths::skill_dir(claude_tmp.path(), name);
            if skill_dir.exists() {
                fs::remove_dir_all(&skill_dir).unwrap();
            }
        }
        let claude_md = paths::claude_md_path(claude_tmp.path());
        assert!(
            fs::read_to_string(&claude_md)
                .unwrap()
                .contains("SCODE_LEITER_BEGIN"),
            "precondition: managed block present"
        );

        run_uninstall(state_tmp.path(), claude_tmp.path()).unwrap();

        assert!(
            !fs::read_to_string(&claude_md)
                .unwrap()
                .contains("SCODE_LEITER_BEGIN"),
            "rerun must remove the leftover managed block"
        );
    }

    #[test]
    fn uninstall_skips_dir_without_sentinel() {
        let claude_tmp = tempfile::tempdir().unwrap();
        let state_tmp = tempfile::tempdir().unwrap();
        setup_plugin_files(claude_tmp.path(), state_tmp.path());

        // A legacy-named directory without the sentinel is user content from
        // leiter's perspective and must be left alone.
        let tampered = paths::skill_dir(claude_tmp.path(), "leiter-setup");
        fs::create_dir_all(&tampered).unwrap();
        fs::write(tampered.join("SKILL.md"), "no sentinel here").unwrap();

        run_uninstall(state_tmp.path(), claude_tmp.path()).unwrap();

        assert!(
            tampered.exists(),
            "dir without sentinel should be preserved"
        );
        for (name, _) in SKILL_CONTENTS {
            assert!(
                !paths::skill_dir(claude_tmp.path(), name).exists(),
                "skill dir {name} should be removed"
            );
        }
    }

    #[test]
    fn uninstall_fails_when_all_lack_sentinel() {
        let state_tmp = tempfile::tempdir().unwrap();
        let claude_tmp = tempfile::tempdir().unwrap();
        setup_plugin_files(claude_tmp.path(), state_tmp.path());

        // Replace all skill SKILL.md files with content lacking the sentinel.
        for (name, _) in SKILL_CONTENTS {
            let skill_dir = paths::skill_dir(claude_tmp.path(), name);
            fs::write(skill_dir.join("SKILL.md"), "no sentinel here").unwrap();
        }
        // Drop the managed block too: no sentinel skill and no block means
        // nothing leiter-owned remains to remove.
        fs::remove_file(paths::claude_md_path(claude_tmp.path())).unwrap();

        let err = run_uninstall(state_tmp.path(), claude_tmp.path()).unwrap_err();
        assert!(err.to_string().contains("nothing to uninstall"));
    }

    #[test]
    fn uninstall_tolerates_missing_skill_dirs() {
        let state_tmp = tempfile::tempdir().unwrap();
        let claude_tmp = tempfile::tempdir().unwrap();
        setup_plugin_files(claude_tmp.path(), state_tmp.path());

        let legacy = paths::skill_dir(claude_tmp.path(), "leiter-distill");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(
            legacy.join("SKILL.md"),
            format!("<!-- {PLUGIN_SENTINEL} -->\n"),
        )
        .unwrap();
        fs::remove_dir_all(&legacy).unwrap();

        run_uninstall(state_tmp.path(), claude_tmp.path()).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn uninstall_reports_partial_removal() {
        use std::os::unix::fs::PermissionsExt;

        let state_tmp = tempfile::tempdir().unwrap();
        let claude_tmp = tempfile::tempdir().unwrap();
        setup_plugin_files(claude_tmp.path(), state_tmp.path());

        // Make the skills/ parent dir read-only so remove_dir_all fails,
        // while SKILL.md files inside remain readable for the sentinel check.
        let skills_parent = claude_tmp.path().join("skills");
        fs::set_permissions(&skills_parent, fs::Permissions::from_mode(0o555)).unwrap();

        let result = run(state_tmp.path(), claude_tmp.path());

        // Restore permissions so tempdir cleanup works.
        fs::set_permissions(&skills_parent, fs::Permissions::from_mode(0o755)).unwrap();

        assert!(result.is_err());
    }

    #[test]
    fn teardown_instructions_contain_hook_commands() {
        let state_tmp = tempfile::tempdir().unwrap();
        let claude_tmp = tempfile::tempdir().unwrap();
        setup_plugin_files(claude_tmp.path(), state_tmp.path());

        let mut out = Vec::new();
        agent_teardown_instructions(state_tmp.path(), &mut out).unwrap();
        let output = String::from_utf8(out).unwrap();
        assert!(output.contains("command` field contains `leiter hook`"));
        assert!(output.contains("Keep `Bash(leiter:*)`"));
        assert!(!output.contains("Remove them"));
    }

    #[test]
    fn teardown_instructions_missing_soul_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let mut out = Vec::new();
        let result = agent_teardown_instructions(tmp.path(), &mut out);
        assert!(result.is_err());
    }

    #[test]
    fn hard_epoch_mismatch_new_state_errors() {
        let state_tmp = tempfile::tempdir().unwrap();
        let claude_tmp = tempfile::tempdir().unwrap();
        write_state_with_epochs(state_tmp.path(), SETUP_SOFT_EPOCH, SETUP_HARD_EPOCH + 1);

        let err = run(state_tmp.path(), claude_tmp.path()).unwrap_err();
        assert!(err.to_string().contains("binary is outdated"));
    }

    #[test]
    fn hard_epoch_mismatch_old_state_errors() {
        let state_tmp = tempfile::tempdir().unwrap();
        let claude_tmp = tempfile::tempdir().unwrap();
        write_state_with_epochs(
            state_tmp.path(),
            SETUP_SOFT_EPOCH,
            SETUP_HARD_EPOCH.saturating_sub(1),
        );

        let err = run(state_tmp.path(), claude_tmp.path()).unwrap_err();
        assert!(err.to_string().contains("setup is incompatible"));
    }

    #[test]
    fn corrupt_state_errors() {
        let state_tmp = tempfile::tempdir().unwrap();
        let claude_tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(state_tmp.path()).unwrap();
        fs::write(paths::soul_path(state_tmp.path()), "body\n").unwrap();
        fs::write(paths::state_path(state_tmp.path()), "not = valid = toml").unwrap();

        let err = run(state_tmp.path(), claude_tmp.path()).unwrap_err();
        assert!(err.to_string().contains("state file"));
    }

    #[test]
    fn teardown_instructions_new_state_epoch_mismatch_errors() {
        let tmp = tempfile::tempdir().unwrap();
        write_state_with_epochs(tmp.path(), SETUP_SOFT_EPOCH, SETUP_HARD_EPOCH + 1);

        let mut out = Vec::new();
        let err = agent_teardown_instructions(tmp.path(), &mut out).unwrap_err();
        assert!(
            err.to_string()
                .contains("binary is older than your soul file")
        );
    }

    #[test]
    fn teardown_instructions_old_state_epoch_mismatch_errors() {
        let tmp = tempfile::tempdir().unwrap();
        write_state_with_epochs(
            tmp.path(),
            SETUP_SOFT_EPOCH,
            SETUP_HARD_EPOCH.saturating_sub(1),
        );

        let mut out = Vec::new();
        let err = agent_teardown_instructions(tmp.path(), &mut out).unwrap_err();
        assert!(err.to_string().contains("leiter claude install"));
    }

    #[test]
    fn teardown_instructions_corrupt_state_errors() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path()).unwrap();
        fs::write(paths::soul_path(tmp.path()), "body\n").unwrap();
        fs::write(paths::state_path(tmp.path()), "not = valid = toml").unwrap();

        let mut out = Vec::new();
        let err = agent_teardown_instructions(tmp.path(), &mut out).unwrap_err();
        assert!(err.to_string().contains("state file"));
    }
}
