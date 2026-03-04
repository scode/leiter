//! `leiter hook context` — inject soul content and agent instructions into the session.
//!
//! Called by the SessionStart hook on every session start. Checks setup epoch
//! compatibility, then outputs the preamble (explaining how to interact with
//! leiter) followed by the full soul file. Hard epoch mismatches and corrupt
//! frontmatter block the session; soft mismatches produce a nudge.

use std::io::Write;
use std::path::Path;

use anyhow::Result;

use crate::soul_validation::{SoulStatus, validate_soul};
use crate::templates::context_preamble;

/// Run the context command.
///
/// Validates the soul file, then outputs the preamble and soul content.
/// Hard epoch mismatches and corrupt frontmatter block the session (no soul
/// injected). Soft epoch mismatches produce a nudge but still inject the soul.
///
/// Always exits successfully — the SessionStart hook should never fail the
/// session.
pub fn run(state_dir: &Path, out: &mut impl Write) -> Result<()> {
    match validate_soul(state_dir) {
        SoulStatus::Incompatible(reason) => {
            writeln!(out, "{}", reason.agent_message())?;
        }
        SoulStatus::Compatible {
            raw_content,
            soft_nudge,
            ..
        } => {
            if let Some(nudge) = &soft_nudge {
                writeln!(out, "{nudge}\n")?;
            }
            write!(out, "{}{raw_content}", context_preamble(state_dir))?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::agent_setup;
    use crate::frontmatter::{SoulFrontmatter, serialize_soul};
    use crate::paths;
    use crate::templates::{SETUP_HARD_EPOCH, SETUP_SOFT_EPOCH};
    use chrono::{TimeZone, Utc};
    use std::fs;

    fn run_context(state_dir: &Path) -> String {
        let mut out = Vec::new();
        run(state_dir, &mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    fn setup_and_context(state_dir: &Path) -> String {
        let claude_tmp = tempfile::tempdir().unwrap();
        agent_setup::run(state_dir, claude_tmp.path()).unwrap();
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
        assert!(output.contains("leiter claude install"));
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
        assert!(output.contains("/leiter-distill"));
        assert!(output.contains("/leiter-instill"));
        assert!(output.contains("/leiter-soul-upgrade"));
    }

    #[test]
    fn soul_content_reproduced_verbatim() {
        let tmp = tempfile::tempdir().unwrap();
        let claude_tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        agent_setup::run(dir, claude_tmp.path()).unwrap();

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
        assert!(output.contains("leiter claude install"));
        assert!(!output.contains(&context_preamble(tmp.path())));
    }

    #[test]
    fn hard_epoch_mismatch_new_soul_blocks() {
        let tmp = tempfile::tempdir().unwrap();
        write_soul_with_epochs(tmp.path(), SETUP_SOFT_EPOCH, SETUP_HARD_EPOCH + 1);
        let output = run_context(tmp.path());
        assert!(output.contains("ACTION REQUIRED"));
        assert!(output.contains("binary is older than your soul file"));
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
        assert!(output.contains("leiter claude install"));
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
    fn malformed_frontmatter_blocks_soul_injection() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        fs::create_dir_all(dir).unwrap();
        let soul_path = paths::soul_path(dir);
        fs::write(&soul_path, "not valid frontmatter\n").unwrap();
        let output = run_context(dir);
        assert!(output.contains("ACTION REQUIRED"));
        assert!(output.contains("invalid YAML"));
        assert!(output.contains(&soul_path.display().to_string()));
        assert!(!output.contains(&context_preamble(dir)));
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
