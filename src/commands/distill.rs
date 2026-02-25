//! `leiter distill` — output unprocessed session logs for the agent to distill.
//!
//! Reads the `last_distilled` timestamp from the soul frontmatter, scans the
//! logs directory for files with timestamps >= that value, and outputs them
//! chronologically. The inclusive comparison ensures a log written in the same
//! second as the distillation timestamp is not lost.

use std::fs;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};

use crate::errors::LeiterError;
use crate::frontmatter::parse_soul;
use crate::log_filename::parse_log_filename;
use crate::paths;

/// Run the distill command.
///
/// Outputs all session logs whose filename timestamps are >= `last_distilled`
/// from the soul frontmatter, sorted chronologically. Each log is preceded by
/// a header line with the filename.
pub fn run(home: &Path, out: &mut impl Write) -> Result<()> {
    let soul_path = paths::soul_path(home);
    let logs_dir = paths::logs_dir(home);

    let content = fs::read_to_string(&soul_path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            LeiterError::SoulNotFound.into()
        } else {
            anyhow::anyhow!("failed to read {}: {e}", soul_path.display())
        }
    })?;
    let (fm, _) = parse_soul(&content)?;

    let entries = fs::read_dir(&logs_dir)
        .with_context(|| format!("failed to read logs directory: {}", logs_dir.display()))?;

    let mut logs = Vec::new();

    for entry in entries {
        let entry = entry?;
        let filename = entry.file_name();
        let Some(filename_str) = filename.to_str() else {
            continue;
        };

        let Ok((ts, _session_id)) = parse_log_filename(filename_str) else {
            continue;
        };

        if ts >= fm.last_distilled {
            logs.push((ts, filename_str.to_string(), entry.path()));
        }
    }

    if logs.is_empty() {
        writeln!(out, "No new session logs to process.")?;
        return Ok(());
    }

    logs.sort_by_key(|(ts, _, _)| *ts);

    for (_, filename, path) in &logs {
        let content = fs::read_to_string(path)
            .with_context(|| format!("failed to read log file: {}", path.display()))?;
        writeln!(out, "## {filename}\n")?;
        write!(out, "{content}")?;
        if !content.ends_with('\n') {
            writeln!(out)?;
        }
        writeln!(out)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::agent_setup;
    use crate::frontmatter::serialize_soul;
    use crate::log_filename::generate_log_filename;
    use chrono::{TimeZone, Utc};

    fn setup_home() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        agent_setup::run(tmp.path(), &mut Vec::new()).unwrap();
        tmp
    }

    fn run_distill(home: &Path) -> String {
        let mut out = Vec::new();
        run(home, &mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    fn write_log(
        home: &Path,
        year: i32,
        month: u32,
        day: u32,
        hour: u32,
        session_id: &str,
        content: &str,
    ) {
        let ts = Utc.with_ymd_and_hms(year, month, day, hour, 0, 0).unwrap();
        let filename = generate_log_filename(ts, session_id);
        let path = paths::logs_dir(home).join(filename);
        fs::write(path, content).unwrap();
    }

    fn set_last_distilled(home: &Path, year: i32, month: u32, day: u32, hour: u32) {
        let ts = Utc.with_ymd_and_hms(year, month, day, hour, 0, 0).unwrap();
        let soul_path = paths::soul_path(home);
        let content = fs::read_to_string(&soul_path).unwrap();
        let (mut fm, body) = parse_soul(&content).unwrap();
        fm.last_distilled = ts;
        fs::write(&soul_path, serialize_soul(&fm, body)).unwrap();
    }

    #[test]
    fn no_logs_at_all() {
        let tmp = setup_home();
        let output = run_distill(tmp.path());
        assert!(output.contains("No new session logs to process"));
    }

    #[test]
    fn all_logs_older_than_last_distilled() {
        let tmp = setup_home();
        write_log(tmp.path(), 2026, 1, 1, 10, "old", "old content");
        set_last_distilled(tmp.path(), 2026, 6, 1, 0);

        let output = run_distill(tmp.path());
        assert!(output.contains("No new session logs to process"));
    }

    #[test]
    fn log_with_timestamp_equal_to_last_distilled_is_included() {
        let tmp = setup_home();
        write_log(tmp.path(), 2026, 6, 1, 0, "exact", "exact content");
        set_last_distilled(tmp.path(), 2026, 6, 1, 0);

        let output = run_distill(tmp.path());
        assert!(output.contains("exact content"));
    }

    #[test]
    fn log_with_timestamp_after_last_distilled_is_included() {
        let tmp = setup_home();
        write_log(tmp.path(), 2026, 7, 1, 0, "new", "new content");
        set_last_distilled(tmp.path(), 2026, 6, 1, 0);

        let output = run_distill(tmp.path());
        assert!(output.contains("new content"));
    }

    #[test]
    fn multiple_logs_in_chronological_order() {
        let tmp = setup_home();
        write_log(tmp.path(), 2026, 3, 1, 0, "second", "BBB");
        write_log(tmp.path(), 2026, 1, 1, 0, "first", "AAA");
        write_log(tmp.path(), 2026, 5, 1, 0, "third", "CCC");

        let output = run_distill(tmp.path());
        let pos_a = output.find("AAA").unwrap();
        let pos_b = output.find("BBB").unwrap();
        let pos_c = output.find("CCC").unwrap();
        assert!(pos_a < pos_b);
        assert!(pos_b < pos_c);
    }

    #[test]
    fn each_log_has_filename_header() {
        let tmp = setup_home();
        write_log(tmp.path(), 2026, 1, 1, 0, "sess1", "content1");

        let output = run_distill(tmp.path());
        assert!(output.contains("## 20260101T000000Z-sess1.jsonl"));
    }

    #[test]
    fn log_content_reproduced_verbatim() {
        let tmp = setup_home();
        let original = "line one\n  indented\n\nlast line\n";
        write_log(tmp.path(), 2026, 1, 1, 0, "sess1", original);

        let output = run_distill(tmp.path());
        assert!(output.contains(original));
    }

    #[test]
    fn unparseable_filenames_silently_skipped() {
        let tmp = setup_home();
        write_log(tmp.path(), 2026, 1, 1, 0, "good", "good content");

        // Write a file with an unparseable name.
        let bad_path = paths::logs_dir(tmp.path()).join("not-a-log.txt");
        fs::write(bad_path, "bad").unwrap();

        let output = run_distill(tmp.path());
        assert!(output.contains("good content"));
        assert!(!output.contains("bad"));
    }

    #[test]
    fn missing_soul_errors() {
        let tmp = tempfile::tempdir().unwrap();
        // Create logs dir but no soul file.
        fs::create_dir_all(paths::logs_dir(tmp.path())).unwrap();

        let mut out = Vec::new();
        let result = run(tmp.path(), &mut out);
        assert!(result.is_err());
    }
}
