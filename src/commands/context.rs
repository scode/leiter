//! `leiter context` — inject soul content and agent instructions into the session.
//!
//! Called by the SessionStart hook on every session start. Checks setup epoch
//! compatibility, then outputs the preamble (explaining how to interact with
//! leiter) followed by the full soul file. Hard epoch mismatches block the
//! session; soft mismatches produce a nudge.

use std::cmp::Ordering;
use std::fs;
use std::io::Write;
use std::path::Path;

use anyhow::Result;
use tracing::warn;

use crate::frontmatter::parse_soul;
use crate::paths;
use crate::templates::{SETUP_HARD_EPOCH, SETUP_SOFT_EPOCH, context_preamble};

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
                    "ACTION REQUIRED: Leiter setup is incompatible (setup_hard_epoch: soul={}, binary={}). The binary was upgraded but setup was not re-run. Before responding to the user's first message, tell them: \"Leiter setup needs to be re-run — please run `leiter agent-setup` in your terminal and follow the instructions, then start a new session.\" Do not attempt to use leiter commands in this session.",
                    fm.setup_hard_epoch, SETUP_HARD_EPOCH,
                )?;
                return Ok(());
            }
            Ordering::Greater => {
                writeln!(
                    out,
                    "ACTION REQUIRED: Leiter setup is incompatible (setup_hard_epoch: soul={}, binary={}). The soul was created by a newer leiter binary than the one currently installed. Before responding to the user's first message, tell them: \"Your leiter binary is outdated — please upgrade it, then start a new session.\" Do not attempt to use leiter commands in this session.",
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
                    "Before responding to the user's first message, briefly mention that leiter setup is slightly behind the binary (setup_soft_epoch: soul={}, binary={}). Suggest they run `leiter agent-setup` when convenient. Keep it to one short sentence.\n",
                    fm.setup_soft_epoch, SETUP_SOFT_EPOCH,
                )?;
            }
            Ordering::Greater => {
                writeln!(
                    out,
                    "Before responding to the user's first message, briefly mention that the leiter binary is slightly behind the setup (setup_soft_epoch: soul={}, binary={}). Suggest they upgrade leiter when convenient. Keep it to one short sentence.\n",
                    fm.setup_soft_epoch, SETUP_SOFT_EPOCH,
                )?;
            }
            Ordering::Equal => {}
        }
    } else {
        warn!("failed to parse soul frontmatter; skipping epoch checks");
    }

    write!(out, "{}{soul_content}", context_preamble(state_dir))?;

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
        assert!(output.starts_with(&context_preamble(tmp.path())));
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
        let soul_path = paths::soul_path(tmp.path()).display().to_string();
        assert!(output.contains(&soul_path));
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
        assert!(output.contains("ACTION REQUIRED"));
        assert!(output.contains("leiter agent-setup"));
        assert!(!output.contains(&context_preamble(tmp.path())));
    }

    #[test]
    fn hard_epoch_mismatch_new_soul_blocks() {
        let tmp = tempfile::tempdir().unwrap();
        write_soul_with_epochs(tmp.path(), SETUP_SOFT_EPOCH, SETUP_HARD_EPOCH + 1);
        let output = run_context(tmp.path());
        assert!(output.contains("ACTION REQUIRED"));
        assert!(output.contains("binary is outdated"));
        assert!(!output.contains(&context_preamble(tmp.path())));
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
        assert!(output.contains("setup is slightly behind"));
        assert!(output.contains("leiter agent-setup"));
        assert!(output.contains(&context_preamble(tmp.path())));
    }

    #[test]
    fn soft_epoch_mismatch_new_soul_nudges() {
        let tmp = tempfile::tempdir().unwrap();
        write_soul_with_epochs(tmp.path(), SETUP_SOFT_EPOCH + 1, SETUP_HARD_EPOCH);
        let output = run_context(tmp.path());
        assert!(output.contains("binary is slightly behind"));
        assert!(output.contains("upgrade leiter"));
        assert!(output.contains(&context_preamble(tmp.path())));
    }

    #[test]
    fn matching_epochs_no_warnings() {
        let tmp = tempfile::tempdir().unwrap();
        let output = setup_and_context(tmp.path());
        assert!(!output.contains("incompatible"));
        assert!(!output.contains("slightly behind"));
        assert!(output.starts_with(&context_preamble(tmp.path())));
    }
}
