//! `leiter hook session-end` — handle the Claude Code SessionEnd event.
//!
//! Reads the SessionEnd hook JSON from stdin (which includes `session_id` and
//! `transcript_path`), then copies the transcript to the leiter logs directory.

use std::io::{Read, Write};
use std::path::Path;

use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde::Deserialize;

use crate::log_filename::generate_log_filename;
use crate::paths;

#[derive(Deserialize)]
struct SessionEndInput {
    session_id: String,
    transcript_path: String,
}

pub fn run(state_dir: &Path, input: &mut impl Read, out: &mut impl Write) -> Result<()> {
    let logs_dir = paths::logs_dir(state_dir);

    if !logs_dir.is_dir() {
        bail!("logs directory does not exist: {}", logs_dir.display());
    }

    let mut raw = String::new();
    input
        .read_to_string(&mut raw)
        .context("failed to read stdin")?;

    let hook_input: SessionEndInput =
        serde_json::from_str(&raw).context("failed to parse session-end JSON")?;

    let transcript = std::fs::read(&hook_input.transcript_path).with_context(|| {
        format!(
            "failed to read transcript at {}",
            hook_input.transcript_path
        )
    })?;

    let timestamp = Utc::now();
    let filename = generate_log_filename(timestamp, &hook_input.session_id);
    let final_path = logs_dir.join(&filename);

    let mut tmp = tempfile::NamedTempFile::new_in(&logs_dir)
        .context("failed to create temp file in logs directory")?;
    tmp.write_all(&transcript)
        .context("failed to write to temp file")?;
    tmp.persist(&final_path)
        .with_context(|| format!("failed to rename temp file to {}", final_path.display()))?;

    writeln!(out, "Transcript saved: {}", final_path.display())?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::test_support::{bytes_to_string, setup_state_dir};
    use crate::log_filename::parse_log_filename;
    use std::fs;
    use std::io::Cursor;

    fn run_session_end(state_dir: &Path, session_id: &str, transcript_path: &str) -> String {
        let json = serde_json::json!({
            "session_id": session_id,
            "transcript_path": transcript_path,
        });
        let mut input = Cursor::new(json.to_string().into_bytes());
        let mut out = Vec::new();
        run(state_dir, &mut input, &mut out).unwrap();
        bytes_to_string(out)
    }

    #[test]
    fn copies_transcript_to_logs_dir() {
        let tmp = setup_state_dir();
        let transcript_file = tempfile::NamedTempFile::new().unwrap();
        fs::write(transcript_file.path(), b"{\"role\":\"user\"}\n").unwrap();

        run_session_end(
            tmp.path(),
            "sess1",
            transcript_file.path().to_str().unwrap(),
        );

        let logs = paths::logs_dir(tmp.path());
        let entries: Vec<_> = fs::read_dir(&logs).unwrap().collect();
        assert_eq!(entries.len(), 1);

        let content = fs::read_to_string(entries[0].as_ref().unwrap().path()).unwrap();
        assert_eq!(content, "{\"role\":\"user\"}\n");
    }

    #[test]
    fn filename_has_correct_format() {
        let tmp = setup_state_dir();
        let transcript_file = tempfile::NamedTempFile::new().unwrap();
        fs::write(transcript_file.path(), b"data").unwrap();

        run_session_end(
            tmp.path(),
            "my-sess",
            transcript_file.path().to_str().unwrap(),
        );

        let logs = paths::logs_dir(tmp.path());
        let entry = fs::read_dir(&logs).unwrap().next().unwrap().unwrap();
        let filename = entry.file_name();
        let (_, session_id) = parse_log_filename(filename.to_str().unwrap()).unwrap();
        assert_eq!(session_id, "my-sess");
    }

    #[test]
    fn confirmation_includes_file_path() {
        let tmp = setup_state_dir();
        let transcript_file = tempfile::NamedTempFile::new().unwrap();
        fs::write(transcript_file.path(), b"data").unwrap();

        let output = run_session_end(
            tmp.path(),
            "sess1",
            transcript_file.path().to_str().unwrap(),
        );
        assert!(output.contains("Transcript saved:"));
        assert!(output.contains("sess1.jsonl"));
    }

    #[test]
    fn missing_logs_dir_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let transcript_file = tempfile::NamedTempFile::new().unwrap();
        fs::write(transcript_file.path(), b"data").unwrap();

        let json = serde_json::json!({
            "session_id": "sess1",
            "transcript_path": transcript_file.path().to_str().unwrap(),
        });
        let mut input = Cursor::new(json.to_string().into_bytes());
        let mut out = Vec::new();
        let result = run(tmp.path(), &mut input, &mut out);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("logs directory does not exist")
        );
    }

    #[test]
    fn missing_transcript_file_errors() {
        let tmp = setup_state_dir();
        let json = serde_json::json!({
            "session_id": "sess1",
            "transcript_path": "/nonexistent/transcript.jsonl",
        });
        let mut input = Cursor::new(json.to_string().into_bytes());
        let mut out = Vec::new();
        let result = run(tmp.path(), &mut input, &mut out);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("failed to read transcript")
        );
    }

    #[test]
    fn invalid_json_errors() {
        let tmp = setup_state_dir();
        let mut input = Cursor::new(b"not json at all".to_vec());
        let mut out = Vec::new();
        let result = run(tmp.path(), &mut input, &mut out);
        assert!(result.is_err());
    }

    #[test]
    fn extra_fields_ignored() {
        let tmp = setup_state_dir();
        let transcript_file = tempfile::NamedTempFile::new().unwrap();
        fs::write(transcript_file.path(), b"data").unwrap();

        let json = serde_json::json!({
            "session_id": "sess1",
            "transcript_path": transcript_file.path().to_str().unwrap(),
            "extra_field": "ignored",
        });
        let mut input = Cursor::new(json.to_string().into_bytes());
        let mut out = Vec::new();
        let result = run(tmp.path(), &mut input, &mut out);
        assert!(result.is_ok());
    }

    #[test]
    fn timestamp_reflects_current_time() {
        let tmp = setup_state_dir();
        let transcript_file = tempfile::NamedTempFile::new().unwrap();
        fs::write(transcript_file.path(), b"data").unwrap();

        let before = Utc::now() - chrono::Duration::seconds(1);
        run_session_end(
            tmp.path(),
            "sess1",
            transcript_file.path().to_str().unwrap(),
        );
        let after = Utc::now();

        let logs = paths::logs_dir(tmp.path());
        let entry = fs::read_dir(&logs).unwrap().next().unwrap().unwrap();
        let filename = entry.file_name();
        let (ts, _) = parse_log_filename(filename.to_str().unwrap()).unwrap();

        assert!(ts >= before);
        assert!(ts <= after);
    }
}
