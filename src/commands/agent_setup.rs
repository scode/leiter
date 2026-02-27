//! `leiter agent-setup` — first-time initialization and hook configuration instructions.
//!
//! Creates the leiter state directory structure and initial soul file, then
//! prints instructions for the agent to wire up Claude Code hooks. If any
//! filesystem step fails, outputs a message for the agent to relay the error
//! to the user instead of exiting non-zero.

use std::fs;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};
use chrono::{DateTime, TimeZone, Utc};
use tracing::info;

use crate::frontmatter::{SoulFrontmatter, parse_soul, serialize_soul};
use crate::paths;
use crate::templates::{
    AGENT_SETUP_INSTRUCTIONS, SETUP_HARD_EPOCH, SETUP_SOFT_EPOCH, SOUL_TEMPLATE,
    SOUL_TEMPLATE_VERSION,
};

/// Run the agent-setup command.
///
/// Creates directories and the initial soul file under `state_dir`, then
/// writes setup instructions to `out`. If the filesystem steps fail, writes
/// an error relay message to `out` instead of returning an error — this way
/// the agent always gets actionable output.
pub fn run(state_dir: &Path, out: &mut impl Write) -> Result<()> {
    if let Err(e) = init_filesystem(state_dir) {
        write!(
            out,
            "leiter agent-setup failed during initialization:\n\n  {e:#}\n\nPlease relay this error to the user.\n"
        )?;
        return Ok(());
    }

    write!(out, "{AGENT_SETUP_INSTRUCTIONS}")?;
    Ok(())
}

/// Deterministic filesystem initialization: create dirs and seed soul file.
fn init_filesystem(state_dir: &Path) -> Result<()> {
    let logs_dir = paths::logs_dir(state_dir);
    let soul_path = paths::soul_path(state_dir);

    fs::create_dir_all(state_dir)
        .with_context(|| format!("failed to create {}", state_dir.display()))?;
    fs::create_dir_all(&logs_dir)
        .with_context(|| format!("failed to create {}", logs_dir.display()))?;

    if !soul_path.exists() {
        let frontmatter = SoulFrontmatter {
            last_distilled: epoch(),
            soul_version: SOUL_TEMPLATE_VERSION,
            setup_soft_epoch: SETUP_SOFT_EPOCH,
            setup_hard_epoch: SETUP_HARD_EPOCH,
        };
        let content = serialize_soul(&frontmatter, SOUL_TEMPLATE);
        fs::write(&soul_path, &content)
            .with_context(|| format!("failed to write {}", soul_path.display()))?;
        info!("created {}", soul_path.display());
    } else {
        info!("soul file already exists, updating epochs");
        update_epochs(&soul_path)?;
    }

    Ok(())
}

/// Update setup epoch fields in an existing soul file's frontmatter.
///
/// Preserves all other frontmatter fields and the body. If the frontmatter
/// can't be parsed, silently skips — a corrupt soul shouldn't block setup.
fn update_epochs(soul_path: &Path) -> Result<()> {
    let content = fs::read_to_string(soul_path)
        .with_context(|| format!("failed to read {}", soul_path.display()))?;

    let (mut fm, body) = match parse_soul(&content) {
        Ok(parsed) => parsed,
        Err(e) => {
            info!("skipping epoch update, frontmatter unparseable: {e}");
            return Ok(());
        }
    };

    if fm.setup_soft_epoch == SETUP_SOFT_EPOCH && fm.setup_hard_epoch == SETUP_HARD_EPOCH {
        info!("epochs already current");
        return Ok(());
    }

    fm.setup_soft_epoch = SETUP_SOFT_EPOCH;
    fm.setup_hard_epoch = SETUP_HARD_EPOCH;
    let updated = serialize_soul(&fm, body);
    fs::write(soul_path, &updated)
        .with_context(|| format!("failed to write {}", soul_path.display()))?;
    info!("updated epochs to soft={SETUP_SOFT_EPOCH}, hard={SETUP_HARD_EPOCH}");

    Ok(())
}

fn epoch() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(1970, 1, 1, 0, 0, 0).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontmatter::{parse_soul, serialize_soul};

    /// Return everything after the closing `---\n` frontmatter delimiter.
    /// This is the raw file suffix — byte-for-byte what follows the frontmatter.
    fn raw_body(content: &str) -> &str {
        let after_opening = content.strip_prefix("---\n").unwrap();
        let (_, body) = after_opening.split_once("\n---\n").unwrap();
        body
    }

    fn run_setup(state_dir: &Path) -> String {
        let mut out = Vec::new();
        run(state_dir, &mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn fresh_setup_creates_directories_and_soul() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        run_setup(dir);

        assert!(dir.is_dir());
        assert!(paths::logs_dir(dir).is_dir());
        assert!(paths::soul_path(dir).is_file());
    }

    #[test]
    fn soul_has_expected_frontmatter() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        run_setup(dir);

        let content = fs::read_to_string(paths::soul_path(dir)).unwrap();
        let (fm, _) = parse_soul(&content).unwrap();
        assert_eq!(fm.last_distilled, epoch());
        assert_eq!(fm.soul_version, SOUL_TEMPLATE_VERSION);
    }

    #[test]
    fn soul_body_matches_template() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        run_setup(dir);

        let content = fs::read_to_string(paths::soul_path(dir)).unwrap();
        let (_, body) = parse_soul(&content).unwrap();
        assert_eq!(body, SOUL_TEMPLATE);
    }

    #[test]
    fn running_twice_does_not_overwrite_soul() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        run_setup(dir);

        let soul = paths::soul_path(dir);
        fs::write(&soul, "modified").unwrap();

        run_setup(dir);

        let content = fs::read_to_string(&soul).unwrap();
        assert_eq!(content, "modified");
    }

    #[test]
    fn rerun_updates_stale_epochs() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        run_setup(dir);

        let soul = paths::soul_path(dir);
        let content = fs::read_to_string(&soul).unwrap();
        let (mut fm, body) = parse_soul(&content).unwrap();

        // Simulate an older soul with outdated epochs.
        fm.setup_soft_epoch = 0;
        fm.setup_hard_epoch = 0;
        fs::write(&soul, serialize_soul(&fm, body)).unwrap();
        let before_body = raw_body(&fs::read_to_string(&soul).unwrap()).to_owned();

        run_setup(dir);

        let updated = fs::read_to_string(&soul).unwrap();
        let (updated_fm, _) = parse_soul(&updated).unwrap();
        assert_eq!(updated_fm.setup_soft_epoch, SETUP_SOFT_EPOCH);
        assert_eq!(updated_fm.setup_hard_epoch, SETUP_HARD_EPOCH);
        assert_eq!(updated_fm.last_distilled, fm.last_distilled);
        assert_eq!(updated_fm.soul_version, fm.soul_version);
        // Raw bytes after frontmatter are identical.
        assert_eq!(raw_body(&updated), before_body);
    }

    #[test]
    fn rerun_preserves_modified_body() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        run_setup(dir);

        let soul = paths::soul_path(dir);
        let content = fs::read_to_string(&soul).unwrap();
        let (mut fm, _) = parse_soul(&content).unwrap();
        fm.setup_soft_epoch = 0;
        let custom_body = "# My customized soul\n\nLearned preferences here.\n";
        fs::write(&soul, serialize_soul(&fm, custom_body)).unwrap();

        run_setup(dir);

        let updated = fs::read_to_string(&soul).unwrap();
        let (updated_fm, _) = parse_soul(&updated).unwrap();
        assert_eq!(updated_fm.setup_soft_epoch, SETUP_SOFT_EPOCH);
        // Raw bytes after frontmatter are identical.
        assert_eq!(raw_body(&updated), custom_body);
    }

    #[test]
    fn rerun_with_unparseable_frontmatter_does_not_error() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        run_setup(dir);

        let soul = paths::soul_path(dir);
        fs::write(&soul, "---\ngarbage: true\n---\nbody\n").unwrap();

        // Should not panic or return error.
        let output = run_setup(dir);
        assert!(output.contains("leiter context"));
    }

    #[test]
    fn running_twice_still_creates_missing_directories() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        run_setup(dir);

        fs::remove_dir(paths::logs_dir(dir)).unwrap();

        run_setup(dir);

        assert!(paths::logs_dir(dir).is_dir());
    }

    #[test]
    fn output_contains_setup_instructions() {
        let tmp = tempfile::tempdir().unwrap();
        let output = run_setup(tmp.path());
        assert!(output.contains("leiter context"));
        assert!(output.contains("leiter session-end"));
    }

    #[test]
    fn init_failure_outputs_error_relay_message() {
        let bad_dir = Path::new("/dev/null/impossible");
        let output = run_setup(bad_dir);
        assert!(output.contains("failed during initialization"));
        assert!(output.contains("relay this error"));
    }
}
