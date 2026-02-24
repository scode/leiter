//! Generate and parse session log filenames.
//!
//! Log files are named `<YYYYMMDDTHHMMSSZ>-<session_id>.md` using UTC ISO 8601
//! basic format for the timestamp. Basic format (no colons or hyphens in the
//! timestamp) keeps filenames filesystem-safe and ensures lexicographic sorting
//! matches chronological order.

use chrono::{DateTime, NaiveDateTime, Utc};

use crate::errors::LeiterError;

const TIMESTAMP_FORMAT: &str = "%Y%m%dT%H%M%SZ";

/// Build a log filename from a timestamp and session ID.
///
/// The timestamp uses UTC ISO 8601 basic format so filenames sort
/// chronologically when sorted lexicographically.
pub fn generate_log_filename(timestamp: DateTime<Utc>, session_id: &str) -> String {
    format!("{}-{}.md", timestamp.format(TIMESTAMP_FORMAT), session_id)
}

/// Extract the timestamp and session ID from a log filename.
///
/// Expects the format produced by [`generate_log_filename`]. The session ID
/// may contain hyphens — only the first hyphen after the timestamp is used
/// as the separator.
pub fn parse_log_filename(filename: &str) -> Result<(DateTime<Utc>, String), LeiterError> {
    let stem = filename.strip_suffix(".md").ok_or_else(|| {
        LeiterError::LogFilenameParse(format!("log filename missing .md extension: {filename}"))
    })?;

    // Timestamp is fixed-width (16 chars: YYYYMMDDTHHMMSSZ), followed by a hyphen.
    if stem.len() < 18 || stem.as_bytes()[16] != b'-' {
        return Err(LeiterError::LogFilenameParse(format!(
            "invalid log filename format: {filename}"
        )));
    }

    let ts_str = &stem[..16];
    let session_id = &stem[17..];

    if session_id.is_empty() {
        return Err(LeiterError::LogFilenameParse(format!(
            "missing session ID in log filename: {filename}"
        )));
    }

    let naive = NaiveDateTime::parse_from_str(ts_str, TIMESTAMP_FORMAT).map_err(|e| {
        LeiterError::LogFilenameParse(format!("bad timestamp in log filename {filename}: {e}"))
    })?;

    Ok((naive.and_utc(), session_id.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn ts(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, mo, d, h, mi, s).unwrap()
    }

    #[test]
    fn generate_produces_correct_format() {
        let filename = generate_log_filename(ts(2026, 2, 23, 17, 30, 0), "abc123");
        assert_eq!(filename, "20260223T173000Z-abc123.md");
    }

    #[test]
    fn parse_extracts_timestamp_and_session_id() {
        let (timestamp, session_id) = parse_log_filename("20260223T173000Z-abc123.md").unwrap();
        assert_eq!(timestamp, ts(2026, 2, 23, 17, 30, 0));
        assert_eq!(session_id, "abc123");
    }

    #[test]
    fn round_trip() {
        let original_ts = ts(2026, 1, 15, 8, 5, 59);
        let original_id = "session-42";
        let filename = generate_log_filename(original_ts, original_id);
        let (parsed_ts, parsed_id) = parse_log_filename(&filename).unwrap();
        assert_eq!(parsed_ts, original_ts);
        assert_eq!(parsed_id, original_id);
    }

    #[test]
    fn parse_rejects_missing_md_extension() {
        assert!(parse_log_filename("20260223T173000Z-abc123").is_err());
    }

    #[test]
    fn parse_rejects_bad_timestamp() {
        assert!(parse_log_filename("not-a-timestamp-abc123.md").is_err());
    }

    #[test]
    fn parse_rejects_missing_session_id() {
        assert!(parse_log_filename("20260223T173000Z-.md").is_err());
    }

    #[test]
    fn session_id_with_hyphens() {
        let filename = generate_log_filename(ts(2026, 2, 23, 17, 30, 0), "a-b-c");
        let (_, session_id) = parse_log_filename(&filename).unwrap();
        assert_eq!(session_id, "a-b-c");
    }

    #[test]
    fn filenames_sort_chronologically() {
        let earlier = generate_log_filename(ts(2026, 1, 1, 0, 0, 0), "aaa");
        let later = generate_log_filename(ts(2026, 12, 31, 23, 59, 59), "zzz");
        assert!(earlier < later);
    }
}
