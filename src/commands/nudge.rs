//! `leiter hook nudge` — remind the agent to suggest distillation when stale logs exist.
//!
//! Checks for undistilled session logs older than 24 hours. If any exist,
//! outputs a short nudge message. Otherwise outputs nothing (zero context
//! pollution). Silently succeeds when leiter is not initialized.

use std::fs;
use std::io::Write;
use std::path::Path;

use anyhow::Result;
use chrono::Utc;

use crate::frontmatter::parse_soul;
use crate::log_filename::collect_log_entries;
use crate::paths;
use crate::templates::NUDGE_MESSAGE;

pub fn run(state_dir: &Path, out: &mut impl Write) -> Result<()> {
    let soul_path = paths::soul_path(state_dir);
    let logs_dir = paths::logs_dir(state_dir);

    let Ok(content) = fs::read_to_string(&soul_path) else {
        return Ok(());
    };
    let Ok((fm, _)) = parse_soul(&content) else {
        return Ok(());
    };
    let Ok(entries) = collect_log_entries(&logs_dir) else {
        return Ok(());
    };

    let cutoff = Utc::now() - chrono::Duration::hours(24);

    for entry in entries {
        if entry.timestamp >= fm.last_distilled && entry.timestamp < cutoff {
            write!(out, "{NUDGE_MESSAGE}")?;
            return Ok(());
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::agent_setup;
    use crate::frontmatter::serialize_soul;
    use crate::log_filename::generate_log_filename;
    use chrono::Utc;

    fn setup_state_dir() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        agent_setup::run(tmp.path(), &mut Vec::new()).unwrap();
        tmp
    }

    fn run_nudge(state_dir: &Path) -> String {
        let mut out = Vec::new();
        run(state_dir, &mut out).unwrap();
        String::from_utf8(out).unwrap()
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
}
