//! `leiter claude uninstall` — removes leiter plugin files from `~/.claude/`.
//!
//! For each known skill, checks for the sentinel marker in its SKILL.md and
//! removes that skill's directory only if the sentinel is present. Does NOT
//! touch `~/.leiter/` or `~/.claude/settings.json`.

use std::fs;
use std::io::Write;
use std::path::Path;

use anyhow::{Result, bail};
use tracing::{info, warn};

use crate::paths;
use crate::soul_validation::{SoulStatus, validate_soul};
use crate::templates::{PLUGIN_SENTINEL, SKILL_CONTENTS};

/// Run the `leiter claude uninstall` command.
///
/// For each known skill, checks whether the sentinel is present in its
/// SKILL.md and removes that skill's directory only if it is. This ensures
/// we never delete a directory we haven't verified ownership of.
pub fn run(state_dir: &Path, claude_home: &Path) -> Result<()> {
    match validate_soul(state_dir) {
        SoulStatus::Incompatible(reason) => bail!("{}", reason.user_message()),
        SoulStatus::Compatible { .. } => {}
    }

    let mut removed = 0;
    let mut failed: Vec<String> = Vec::new();

    // Each skill is checked and removed independently. The sentinel check
    // and the removal must use the same `skill_dir` to guarantee we only
    // delete directories whose SKILL.md we verified.
    for (name, _) in SKILL_CONTENTS {
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

    if removed == 0 && failed.is_empty() {
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

/// Output the agent-teardown instructions (hook removal).
///
/// Used by `leiter claude agent-teardown-instructions`.
pub fn agent_teardown_instructions(state_dir: &Path, out: &mut impl Write) -> Result<()> {
    match validate_soul(state_dir) {
        SoulStatus::Incompatible(reason) => bail!("{}", reason.agent_message()),
        SoulStatus::Compatible { .. } => {}
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
    use crate::commands::test_support::write_soul_with_epochs;
    use crate::templates::{SETUP_HARD_EPOCH, SETUP_SOFT_EPOCH};

    fn setup_plugin_files(claude_home: &Path, state_dir: &Path) {
        agent_setup::run(state_dir, claude_home).unwrap();
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

        run_uninstall(state_tmp.path(), claude_tmp.path()).unwrap();

        assert!(paths::soul_path(state_tmp.path()).is_file());
        assert!(paths::logs_dir(state_tmp.path()).is_dir());
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

        let err = run_uninstall(state_tmp.path(), claude_tmp.path()).unwrap_err();
        assert!(err.to_string().contains("nothing to uninstall"));
    }

    #[test]
    fn uninstall_skips_dir_without_sentinel() {
        let claude_tmp = tempfile::tempdir().unwrap();
        let state_tmp = tempfile::tempdir().unwrap();
        setup_plugin_files(claude_tmp.path(), state_tmp.path());

        // Replace one skill's SKILL.md with content lacking the sentinel.
        let tampered = paths::skill_dir(claude_tmp.path(), "leiter-setup");
        fs::write(tampered.join("SKILL.md"), "no sentinel here").unwrap();

        run_uninstall(state_tmp.path(), claude_tmp.path()).unwrap();

        // The tampered dir should be left alone; all others removed.
        assert!(
            tampered.exists(),
            "dir without sentinel should be preserved"
        );
        for (name, _) in SKILL_CONTENTS {
            if *name == "leiter-setup" {
                continue;
            }
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

        let err = run_uninstall(state_tmp.path(), claude_tmp.path()).unwrap_err();
        assert!(err.to_string().contains("nothing to uninstall"));
    }

    #[test]
    fn uninstall_tolerates_missing_skill_dirs() {
        let state_tmp = tempfile::tempdir().unwrap();
        let claude_tmp = tempfile::tempdir().unwrap();
        setup_plugin_files(claude_tmp.path(), state_tmp.path());

        // Remove some skill dirs before uninstall.
        for (name, _) in &SKILL_CONTENTS[..2] {
            fs::remove_dir_all(paths::skill_dir(claude_tmp.path(), name)).unwrap();
        }

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
        assert!(output.contains("leiter hook context"));
        assert!(output.contains("leiter hook nudge"));
        assert!(output.contains("leiter hook session-end"));
    }

    #[test]
    fn teardown_instructions_missing_soul_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let mut out = Vec::new();
        let result = agent_teardown_instructions(tmp.path(), &mut out);
        assert!(result.is_err());
    }

    #[test]
    fn hard_epoch_mismatch_new_soul_errors() {
        let state_tmp = tempfile::tempdir().unwrap();
        let claude_tmp = tempfile::tempdir().unwrap();
        write_soul_with_epochs(state_tmp.path(), SETUP_SOFT_EPOCH, SETUP_HARD_EPOCH + 1);

        let err = run(state_tmp.path(), claude_tmp.path()).unwrap_err();
        assert!(err.to_string().contains("binary is outdated"));
    }

    #[test]
    fn hard_epoch_mismatch_old_soul_errors() {
        let state_tmp = tempfile::tempdir().unwrap();
        let claude_tmp = tempfile::tempdir().unwrap();
        write_soul_with_epochs(
            state_tmp.path(),
            SETUP_SOFT_EPOCH,
            SETUP_HARD_EPOCH.saturating_sub(1),
        );

        let err = run(state_tmp.path(), claude_tmp.path()).unwrap_err();
        assert!(err.to_string().contains("setup is incompatible"));
    }

    #[test]
    fn corrupt_frontmatter_errors() {
        let state_tmp = tempfile::tempdir().unwrap();
        let claude_tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(state_tmp.path()).unwrap();
        fs::write(paths::soul_path(state_tmp.path()), "not frontmatter").unwrap();

        let err = run(state_tmp.path(), claude_tmp.path()).unwrap_err();
        assert!(err.to_string().contains("invalid YAML front matter"));
    }

    #[test]
    fn teardown_instructions_new_soul_epoch_mismatch_errors() {
        let tmp = tempfile::tempdir().unwrap();
        write_soul_with_epochs(tmp.path(), SETUP_SOFT_EPOCH, SETUP_HARD_EPOCH + 1);

        let mut out = Vec::new();
        let err = agent_teardown_instructions(tmp.path(), &mut out).unwrap_err();
        assert!(
            err.to_string()
                .contains("binary is older than your soul file")
        );
    }

    #[test]
    fn teardown_instructions_old_soul_epoch_mismatch_errors() {
        let tmp = tempfile::tempdir().unwrap();
        write_soul_with_epochs(
            tmp.path(),
            SETUP_SOFT_EPOCH,
            SETUP_HARD_EPOCH.saturating_sub(1),
        );

        let mut out = Vec::new();
        let err = agent_teardown_instructions(tmp.path(), &mut out).unwrap_err();
        assert!(err.to_string().contains("leiter claude install"));
    }

    #[test]
    fn teardown_instructions_corrupt_frontmatter_errors() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path()).unwrap();
        fs::write(paths::soul_path(tmp.path()), "---\n---\n").unwrap();

        let mut out = Vec::new();
        let err = agent_teardown_instructions(tmp.path(), &mut out).unwrap_err();
        assert!(err.to_string().contains("invalid YAML"));
    }
}
