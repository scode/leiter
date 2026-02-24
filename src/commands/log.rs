//! `leiter log --session-id <id>` — store a session log.
//!
//! Reads free-form markdown from stdin, writes it atomically to
//! `~/.leiter/logs/<timestamp>-<session_id>.md`. The timestamp is captured
//! after stdin is fully read so the filename reflects when the log was
//! received, not when the command started.

use std::io::{Read, Write};
use std::path::Path;

use anyhow::{bail, Context, Result};
use chrono::Utc;

use crate::log_filename::generate_log_filename;
use crate::paths;

/// Run the log command.
///
/// Reads all content from `input`, writes it atomically to the logs directory,
/// and prints a confirmation to `out`. Takes `input` and `out` as parameters
/// so tests can substitute a buffer for stdin/stdout.
pub fn run(home: &Path, session_id: &str, input: &mut impl Read, out: &mut impl Write) -> Result<()> {
    let logs_dir = paths::logs_dir(home);

    if !logs_dir.is_dir() {
        bail!("logs directory does not exist: {}", logs_dir.display());
    }

    let mut content = String::new();
    input.read_to_string(&mut content).context("failed to read stdin")?;

    let timestamp = Utc::now();
    let filename = generate_log_filename(timestamp, session_id);
    let final_path = logs_dir.join(&filename);

    let mut tmp = tempfile::NamedTempFile::new_in(&logs_dir)
        .context("failed to create temp file in logs directory")?;
    tmp.write_all(content.as_bytes())
        .context("failed to write to temp file")?;
    tmp.persist(&final_path)
        .with_context(|| format!("failed to rename temp file to {}", final_path.display()))?;

    writeln!(out, "Session log saved: {}", final_path.display())?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::agent_setup;
    use crate::log_filename::parse_log_filename;
    use std::fs;
    use std::io::Cursor;

    fn setup_home() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        agent_setup::run(tmp.path(), &mut Vec::new()).unwrap();
        tmp
    }

    fn run_log(home: &Path, session_id: &str, content: &str) -> String {
        let mut input = Cursor::new(content.as_bytes().to_vec());
        let mut out = Vec::new();
        run(home, session_id, &mut input, &mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn creates_file_with_correct_name_format() {
        let tmp = setup_home();
        run_log(tmp.path(), "sess1", "hello");

        let logs = paths::logs_dir(tmp.path());
        let entries: Vec<_> = fs::read_dir(&logs).unwrap().collect();
        assert_eq!(entries.len(), 1);

        let filename = entries[0].as_ref().unwrap().file_name();
        let filename = filename.to_str().unwrap();
        let (_, session_id) = parse_log_filename(filename).unwrap();
        assert_eq!(session_id, "sess1");
    }

    #[test]
    fn file_contains_exact_content() {
        let tmp = setup_home();
        run_log(tmp.path(), "sess1", "my log content\nline 2\n");

        let logs = paths::logs_dir(tmp.path());
        let entry = fs::read_dir(&logs).unwrap().next().unwrap().unwrap();
        let content = fs::read_to_string(entry.path()).unwrap();
        assert_eq!(content, "my log content\nline 2\n");
    }

    #[test]
    fn confirmation_includes_file_path() {
        let tmp = setup_home();
        let output = run_log(tmp.path(), "sess1", "content");
        assert!(output.contains("Session log saved:"));
        assert!(output.contains("sess1.md"));
    }

    #[test]
    fn missing_logs_dir_errors() {
        let tmp = tempfile::tempdir().unwrap();
        // Don't run agent-setup, so logs dir doesn't exist.
        let mut input = Cursor::new(b"content".to_vec());
        let mut out = Vec::new();
        let result = run(tmp.path(), "sess1", &mut input, &mut out);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("logs directory does not exist"));
    }

    #[test]
    fn session_id_appears_in_filename() {
        let tmp = setup_home();
        run_log(tmp.path(), "my-session-42", "content");

        let logs = paths::logs_dir(tmp.path());
        let entry = fs::read_dir(&logs).unwrap().next().unwrap().unwrap();
        let filename = entry.file_name();
        assert!(filename.to_str().unwrap().contains("my-session-42"));
    }

    #[test]
    fn timestamp_reflects_post_stdin_time() {
        let tmp = setup_home();

        // The filename format truncates sub-seconds, so the parsed timestamp
        // can appear up to 1s before the wall clock. Subtract 1s to account.
        let before = Utc::now() - chrono::Duration::seconds(1);
        run_log(tmp.path(), "sess1", "content");
        let after = Utc::now();

        let logs = paths::logs_dir(tmp.path());
        let entry = fs::read_dir(&logs).unwrap().next().unwrap().unwrap();
        let filename = entry.file_name();
        let (ts, _) = parse_log_filename(filename.to_str().unwrap()).unwrap();

        assert!(ts >= before);
        assert!(ts <= after);
    }
}
