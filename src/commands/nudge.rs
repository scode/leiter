//! `leiter hook nudge` — trigger distillation when stale logs exist.
//!
//! Checks for undistilled session logs older than a threshold. In normal mode
//! (24h threshold), outputs a nudge asking the user. In `--auto-distill` mode
//! (4h threshold), outputs an instruction to silently run distillation.
//! Otherwise outputs nothing (zero context pollution). Silently succeeds when
//! leiter is not initialized.

use std::io::ErrorKind;
use std::io::Write;
use std::path::Path;

use anyhow::Result;
use chrono::Utc;
use tracing::warn;

use crate::log_filename::collect_log_entries;
use crate::paths;
use crate::soul_validation::{SoulIncompatibility, SoulStatus, validate_soul};
use crate::templates::{AUTO_DISTILL_MESSAGE, NUDGE_MESSAGE};

pub fn run(state_dir: &Path, out: &mut impl Write, auto_distill: bool) -> Result<()> {
    let logs_dir = paths::logs_dir(state_dir);

    let fm = match validate_soul(state_dir) {
        SoulStatus::Incompatible(SoulIncompatibility::SoulNotFound) => return Ok(()),
        SoulStatus::Incompatible(reason) => {
            writeln!(out, "{}", reason.agent_message())?;
            return Ok(());
        }
        SoulStatus::Compatible { frontmatter, .. } => frontmatter,
    };

    let entries = match collect_log_entries(&logs_dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(()),
        Err(err) => {
            warn!(
                "failed to read logs directory {}: {err}",
                logs_dir.display()
            );
            return Ok(());
        }
    };

    let (threshold, message) = if auto_distill {
        (chrono::Duration::hours(4), AUTO_DISTILL_MESSAGE)
    } else {
        (chrono::Duration::hours(24), NUDGE_MESSAGE)
    };
    let cutoff = Utc::now() - threshold;

    for entry in entries {
        if entry.timestamp >= fm.last_distilled && entry.timestamp < cutoff {
            write!(out, "{message}")?;
            return Ok(());
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::test_support::{bytes_to_string, setup_state_dir, write_soul_with_epochs};
    use crate::frontmatter::{parse_soul, serialize_soul};
    use crate::log_filename::generate_log_filename;
    use crate::templates::{SETUP_HARD_EPOCH, SETUP_SOFT_EPOCH};
    use chrono::Utc;
    use std::fs;

    fn run_nudge(state_dir: &Path) -> String {
        run_nudge_with(state_dir, false)
    }

    fn run_nudge_with(state_dir: &Path, auto_distill: bool) -> String {
        let mut out = Vec::new();
        run(state_dir, &mut out, auto_distill).unwrap();
        bytes_to_string(out)
    }

    fn write_log(state_dir: &Path, ts: chrono::DateTime<Utc>, session_id: &str) {
        let filename = generate_log_filename(ts, session_id);
        let path = paths::logs_dir(state_dir).join(filename);
        fs::write(path, "log content\n").unwrap();
    }

    fn set_last_distilled(state_dir: &Path, ts: chrono::DateTime<Utc>) {
        let soul_path = paths::soul_path(state_dir);
        let content = fs::read_to_string(&soul_path).unwrap();
        let (mut fm, body) = parse_soul(&content).unwrap();
        fm.last_distilled = ts;
        fs::write(&soul_path, serialize_soul(&fm, body)).unwrap();
    }

    #[test]
    fn no_logs_outputs_nothing() {
        let tmp = setup_state_dir();
        let output = run_nudge(tmp.path());
        assert!(output.is_empty());
    }

    #[test]
    fn stale_undistilled_log_outputs_nudge() {
        let tmp = setup_state_dir();
        let stale_ts = Utc::now() - chrono::Duration::hours(48);
        write_log(tmp.path(), stale_ts, "stale-sess");
        let output = run_nudge(tmp.path());
        assert!(output.contains("undistilled leiter session logs"));
    }

    #[test]
    fn recent_undistilled_log_outputs_nothing() {
        let tmp = setup_state_dir();
        let recent_ts = Utc::now() - chrono::Duration::hours(1);
        write_log(tmp.path(), recent_ts, "recent-sess");
        let output = run_nudge(tmp.path());
        assert!(output.is_empty());
    }

    #[test]
    fn already_distilled_stale_log_outputs_nothing() {
        let tmp = setup_state_dir();
        let stale_ts = Utc::now() - chrono::Duration::hours(48);
        write_log(tmp.path(), stale_ts, "old-sess");
        set_last_distilled(tmp.path(), Utc::now());
        let output = run_nudge(tmp.path());
        assert!(output.is_empty());
    }

    #[test]
    fn missing_soul_outputs_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let output = run_nudge(tmp.path());
        assert!(output.is_empty());
    }

    #[test]
    fn missing_logs_dir_outputs_nothing() {
        let tmp = setup_state_dir();
        fs::remove_dir_all(paths::logs_dir(tmp.path())).unwrap();
        let output = run_nudge(tmp.path());
        assert!(output.is_empty());
    }

    #[test]
    fn boundary_exactly_24h_outputs_nothing() {
        let tmp = setup_state_dir();
        let not_stale_ts = Utc::now() - chrono::Duration::hours(24) + chrono::Duration::seconds(2);
        write_log(tmp.path(), not_stale_ts, "boundary-sess2");

        let output = run_nudge(tmp.path());
        assert!(output.is_empty());
    }

    #[test]
    fn just_over_24h_outputs_nudge() {
        let tmp = setup_state_dir();
        let ts = Utc::now() - chrono::Duration::hours(25);
        write_log(tmp.path(), ts, "stale-sess");
        let output = run_nudge(tmp.path());
        assert!(output.contains("undistilled leiter session logs"));
    }

    #[test]
    fn mix_of_stale_and_recent_outputs_nudge() {
        let tmp = setup_state_dir();
        let stale_ts = Utc::now() - chrono::Duration::hours(48);
        let recent_ts = Utc::now() - chrono::Duration::hours(1);
        write_log(tmp.path(), stale_ts, "stale-sess");
        write_log(tmp.path(), recent_ts, "recent-sess");
        let output = run_nudge(tmp.path());
        assert!(output.contains("undistilled leiter session logs"));
    }

    #[test]
    fn unparseable_filenames_ignored() {
        let tmp = setup_state_dir();
        let bad_path = paths::logs_dir(tmp.path()).join("not-a-log.txt");
        fs::write(bad_path, "junk").unwrap();
        let output = run_nudge(tmp.path());
        assert!(output.is_empty());
    }

    #[test]
    fn malformed_soul_outputs_error() {
        let tmp = setup_state_dir();
        fs::write(paths::soul_path(tmp.path()), "not frontmatter").unwrap();

        let output = run_nudge(tmp.path());
        assert!(output.contains("ACTION REQUIRED"));
        assert!(output.contains("invalid YAML"));
    }

    #[test]
    fn hard_epoch_mismatch_new_soul_outputs_error() {
        let tmp = tempfile::tempdir().unwrap();
        write_soul_with_epochs(tmp.path(), SETUP_SOFT_EPOCH, SETUP_HARD_EPOCH + 1);
        let output = run_nudge(tmp.path());
        assert!(output.contains("ACTION REQUIRED"));
        assert!(output.contains("binary is older than your soul file"));
    }

    #[test]
    fn hard_epoch_mismatch_old_soul_outputs_error() {
        let tmp = tempfile::tempdir().unwrap();
        write_soul_with_epochs(
            tmp.path(),
            SETUP_SOFT_EPOCH,
            SETUP_HARD_EPOCH.saturating_sub(1),
        );
        let output = run_nudge(tmp.path());
        assert!(output.contains("ACTION REQUIRED"));
        assert!(output.contains("leiter claude install"));
    }

    #[test]
    fn auto_distill_5h_old_log_triggers_message() {
        let tmp = setup_state_dir();
        let ts = Utc::now() - chrono::Duration::hours(5);
        write_log(tmp.path(), ts, "auto-sess");
        let output = run_nudge_with(tmp.path(), true);
        assert!(output.contains("/leiter-distill"));
    }

    #[test]
    fn auto_distill_3h_old_log_outputs_nothing() {
        let tmp = setup_state_dir();
        let ts = Utc::now() - chrono::Duration::hours(3);
        write_log(tmp.path(), ts, "recent-sess");
        let output = run_nudge_with(tmp.path(), true);
        assert!(output.is_empty());
    }

    #[test]
    fn auto_distill_boundary_at_4h_outputs_nothing() {
        let tmp = setup_state_dir();
        let ts = Utc::now() - chrono::Duration::hours(4) + chrono::Duration::seconds(2);
        write_log(tmp.path(), ts, "boundary-sess");
        let output = run_nudge_with(tmp.path(), true);
        assert!(output.is_empty());
    }
}
