//! `leiter context` — inject soul content and agent instructions into the session.
//!
//! Called by the SessionStart hook on every session start. Outputs the preamble
//! (explaining how to interact with leiter) followed by the full soul file, so
//! the agent has all learned preferences in context.

use std::fs;
use std::io::Write;
use std::path::Path;

use std::cmp::Ordering;

use anyhow::Result;
use tracing::warn;

use crate::frontmatter::parse_soul;
use crate::paths;
use crate::templates::{CONTEXT_PREAMBLE, SETUP_HARD_EPOCH, SETUP_SOFT_EPOCH};

/// Run the context command.
///
/// If the soul file exists, checks setup epochs for compatibility, then
/// outputs the preamble and soul content. Hard epoch mismatches block the
/// session (no soul injected). Soft epoch mismatches produce a nudge but
/// still inject the soul.
///
/// If the soul doesn't exist, outputs a message suggesting `leiter agent-setup`.
/// Either way, exits successfully — the SessionStart hook should never fail
/// the session.
pub fn run(state_dir: &Path, out: &mut impl Write) -> Result<()> {
    let soul_path = paths::soul_path(state_dir);

    if !soul_path.exists() {
        writeln!(
            out,
            "Leiter is not initialized. Run `leiter agent-setup` to set up."
        )?;
        return Ok(());
    }

    let soul_content = fs::read_to_string(&soul_path)?;

    if let Ok((fm, _)) = parse_soul(&soul_content) {
        match fm.setup_hard_epoch.cmp(&SETUP_HARD_EPOCH) {
            Ordering::Less => {
                writeln!(
                    out,
                    "Leiter setup is incompatible: the leiter binary is newer than the soul on disk (setup_hard_epoch: soul={}, binary={}). This likely means the binary was upgraded but setup was not re-run. Tell Claude: \"Run the shell command `leiter agent-setup` and follow the instructions it outputs on stdout.\"",
                    fm.setup_hard_epoch, SETUP_HARD_EPOCH,
                )?;
                return Ok(());
            }
            Ordering::Greater => {
                writeln!(
                    out,
                    "Leiter setup is incompatible: the soul on disk is newer than this leiter binary (setup_hard_epoch: soul={}, binary={}). This likely means a newer binary was used to set up the soul. Please upgrade the leiter binary.",
                    fm.setup_hard_epoch, SETUP_HARD_EPOCH,
                )?;
                return Ok(());
            }
            Ordering::Equal => {}
        }

        match fm.setup_soft_epoch.cmp(&SETUP_SOFT_EPOCH) {
            Ordering::Less => {
                writeln!(
                    out,
                    "Note: The leiter binary is newer than the soul on disk (setup_soft_epoch: soul={}, binary={}). This likely means the binary was upgraded but setup was not re-run. When convenient, tell Claude: \"Run the shell command `leiter agent-setup` and follow the instructions it outputs on stdout.\"\n",
                    fm.setup_soft_epoch, SETUP_SOFT_EPOCH,
                )?;
            }
            Ordering::Greater => {
                writeln!(
                    out,
                    "Note: The soul on disk is newer than this leiter binary (setup_soft_epoch: soul={}, binary={}). This likely means a newer binary was used to set up the soul. Consider upgrading the leiter binary when convenient.\n",
                    fm.setup_soft_epoch, SETUP_SOFT_EPOCH,
                )?;
            }
            Ordering::Equal => {}
        }
    } else {
        warn!("failed to parse soul frontmatter; skipping epoch checks");
    }

    write!(out, "{CONTEXT_PREAMBLE}{soul_content}")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::agent_setup;
    use crate::frontmatter::{SoulFrontmatter, serialize_soul};
    use chrono::{TimeZone, Utc};

    fn run_context(state_dir: &Path) -> String {
        let mut out = Vec::new();
        run(state_dir, &mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    fn setup_and_context(state_dir: &Path) -> String {
        agent_setup::run(state_dir, &mut Vec::new()).unwrap();
        run_context(state_dir)
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
    fn with_soul_output_starts_with_preamble() {
        let tmp = tempfile::tempdir().unwrap();
        let output = setup_and_context(tmp.path());
        assert!(output.starts_with(CONTEXT_PREAMBLE));
    }

    #[test]
    fn without_soul_suggests_agent_setup() {
        let tmp = tempfile::tempdir().unwrap();
        let output = run_context(tmp.path());
        assert!(output.contains("not initialized"));
        assert!(output.contains("leiter agent-setup"));
    }

    #[test]
    fn preamble_contains_required_elements() {
        let tmp = tempfile::tempdir().unwrap();
        let output = setup_and_context(tmp.path());
        assert!(output.contains("~/.leiter/soul.md"));
        assert!(output.contains("Read/Edit/Write"));
        assert!(output.contains("remember"));
        assert!(output.contains("session log"));
        assert!(output.contains("leiter distill"));
        assert!(output.contains("leiter soul-upgrade"));
    }

    #[test]
    fn soul_content_reproduced_verbatim() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        agent_setup::run(dir, &mut Vec::new()).unwrap();

        let soul_content = fs::read_to_string(paths::soul_path(dir)).unwrap();
        let output = run_context(dir);

        assert!(output.ends_with(&soul_content));
    }

    #[test]
    fn hard_epoch_mismatch_old_soul_blocks() {
        let tmp = tempfile::tempdir().unwrap();
        write_soul_with_epochs(
            tmp.path(),
            SETUP_SOFT_EPOCH,
            SETUP_HARD_EPOCH.saturating_sub(1),
        );
        let output = run_context(tmp.path());
        assert!(output.contains("binary is newer than the soul"));
        assert!(output.contains("leiter agent-setup"));
        assert!(!output.contains(CONTEXT_PREAMBLE));
    }

    #[test]
    fn hard_epoch_mismatch_new_soul_blocks() {
        let tmp = tempfile::tempdir().unwrap();
        write_soul_with_epochs(tmp.path(), SETUP_SOFT_EPOCH, SETUP_HARD_EPOCH + 1);
        let output = run_context(tmp.path());
        assert!(output.contains("soul on disk is newer than this leiter binary"));
        assert!(output.contains("upgrade the leiter binary"));
        assert!(!output.contains(CONTEXT_PREAMBLE));
    }

    #[test]
    fn soft_epoch_mismatch_old_soul_nudges() {
        let tmp = tempfile::tempdir().unwrap();
        write_soul_with_epochs(
            tmp.path(),
            SETUP_SOFT_EPOCH.saturating_sub(1),
            SETUP_HARD_EPOCH,
        );
        let output = run_context(tmp.path());
        assert!(output.contains("binary is newer than the soul"));
        assert!(output.contains("leiter agent-setup"));
        assert!(output.contains(CONTEXT_PREAMBLE));
    }

    #[test]
    fn soft_epoch_mismatch_new_soul_nudges() {
        let tmp = tempfile::tempdir().unwrap();
        write_soul_with_epochs(tmp.path(), SETUP_SOFT_EPOCH + 1, SETUP_HARD_EPOCH);
        let output = run_context(tmp.path());
        assert!(output.contains("soul on disk is newer than this leiter binary"));
        assert!(output.contains("upgrading the leiter binary"));
        assert!(output.contains(CONTEXT_PREAMBLE));
    }

    #[test]
    fn matching_epochs_no_warnings() {
        let tmp = tempfile::tempdir().unwrap();
        let output = setup_and_context(tmp.path());
        assert!(!output.contains("incompatible"));
        assert!(!output.contains("slightly behind"));
        assert!(output.starts_with(CONTEXT_PREAMBLE));
    }
}
