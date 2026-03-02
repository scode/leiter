//! `leiter soul instill` — output soul-writing instructions for a user preference.
//!
//! Takes the user's preference as a positional argument and outputs a
//! self-contained instruction block: the quoted preference, shared
//! soul-writing guidelines, and an edit instruction. This ensures
//! consistent entry quality whether the agent learns inline or via
//! distillation.

use std::io::Write;
use std::path::Path;

use anyhow::{Result, bail};

use crate::paths;
use crate::soul_validation::{SoulStatus, validate_soul};
use crate::templates::SOUL_WRITING_GUIDELINES;

/// Run the instill command.
///
/// Validates the soul file, then outputs the user's preference (quoted),
/// the shared soul-writing guidelines, and an instruction to edit the soul
/// file.
pub fn run(state_dir: &Path, out: &mut impl Write, text: &str) -> Result<()> {
    match validate_soul(state_dir) {
        SoulStatus::Incompatible(reason) => bail!("{}", reason.agent_message()),
        SoulStatus::Compatible { .. } => {}
    }

    writeln!(out, "The user wants you to remember:\n")?;
    for line in text.lines() {
        writeln!(out, "> {line}")?;
    }
    writeln!(out)?;
    write!(out, "{SOUL_WRITING_GUIDELINES}")?;
    writeln!(
        out,
        "Now read `{}` and edit the appropriate section following the guidelines above.",
        paths::soul_path(state_dir).display()
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::test_support::setup_state_dir;
    use crate::frontmatter::{SoulFrontmatter, serialize_soul};
    use crate::templates::{SETUP_HARD_EPOCH, SETUP_SOFT_EPOCH};
    use chrono::{TimeZone, Utc};
    use std::fs;

    fn run_instill(state_dir: &Path, text: &str) -> String {
        let mut out = Vec::new();
        run(state_dir, &mut out, text).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn output_contains_quoted_preference() {
        let tmp = setup_state_dir();
        let output = run_instill(tmp.path(), "always use snake_case");
        assert!(output.contains("> always use snake_case"));
    }

    #[test]
    fn output_contains_guidelines() {
        let tmp = setup_state_dir();
        let output = run_instill(tmp.path(), "test preference");
        assert!(output.contains("Soul-writing guidelines"));
    }

    #[test]
    fn output_contains_edit_instruction() {
        let tmp = setup_state_dir();
        let output = run_instill(tmp.path(), "test preference");
        assert!(output.contains("soul.md"));
        assert!(output.contains("edit the appropriate section"));
    }

    #[test]
    fn output_contains_remember_preamble() {
        let tmp = setup_state_dir();
        let output = run_instill(tmp.path(), "test preference");
        assert!(output.contains("The user wants you to remember"));
    }

    #[test]
    fn missing_soul_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let mut out = Vec::new();
        let result = run(tmp.path(), &mut out, "test");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not initialized"));
    }

    fn write_soul_with_epochs(state_dir: &Path, soft: u32, hard: u32) {
        let fm = SoulFrontmatter {
            last_distilled: Utc.with_ymd_and_hms(1970, 1, 1, 0, 0, 0).unwrap(),
            soul_version: 2,
            setup_soft_epoch: soft,
            setup_hard_epoch: hard,
        };
        let soul = serialize_soul(&fm, "body\n");
        fs::create_dir_all(state_dir).unwrap();
        fs::write(paths::soul_path(state_dir), soul).unwrap();
    }

    #[test]
    fn hard_epoch_mismatch_new_soul_errors() {
        let tmp = tempfile::tempdir().unwrap();
        write_soul_with_epochs(tmp.path(), SETUP_SOFT_EPOCH, SETUP_HARD_EPOCH + 1);

        let mut out = Vec::new();
        let err = run(tmp.path(), &mut out, "test").unwrap_err();
        assert!(
            err.to_string()
                .contains("binary is older than your soul file")
        );
    }

    #[test]
    fn hard_epoch_mismatch_old_soul_errors() {
        let tmp = tempfile::tempdir().unwrap();
        write_soul_with_epochs(
            tmp.path(),
            SETUP_SOFT_EPOCH,
            SETUP_HARD_EPOCH.saturating_sub(1),
        );

        let mut out = Vec::new();
        let err = run(tmp.path(), &mut out, "test").unwrap_err();
        assert!(err.to_string().contains("leiter claude install"));
    }

    #[test]
    fn corrupt_frontmatter_errors() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path()).unwrap();
        fs::write(paths::soul_path(tmp.path()), "not frontmatter").unwrap();

        let mut out = Vec::new();
        let err = run(tmp.path(), &mut out, "test").unwrap_err();
        assert!(err.to_string().contains("invalid YAML"));
    }
}
