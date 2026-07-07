//! Best-effort Claude Code session discovery and transcript canonicalization.
//!
//! Claude Code's `projects/` layout is not a stable public API, so this module
//! treats discovery as opportunistic: it reads only files that look exactly
//! like session transcripts, warns on I/O failures, and never follows symlinks.
//! The caller decides whether changed sessions are emitted, staged, or both.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde_json::Value;
use tracing::warn;

use crate::claude_transcript::render_session_log;
use crate::state::SessionWatermark;

const HEADER_SCAN_LIMIT: usize = 50;

/// Canonicalized Claude session content discovered from `<claude_home>/projects/`.
///
/// A value represents a changed file state, not necessarily visible transcript
/// content. `rendered` may be empty for sessions that contain only system or
/// progress records; those sessions still carry a watermark so distill can
/// stage them and stop rescanning them after mark-distilled promotes the
/// pending set.
#[derive(Debug, Clone)]
pub struct DistilledClaudeSession {
    /// Stable Claude session id taken from the UUID filename stem.
    pub session_id: String,
    /// Claude-home-relative transcript path shown in distill output.
    pub file_label: String,
    /// Timestamp used to interleave this session with legacy hook logs.
    pub sort_timestamp: DateTime<Utc>,
    /// User-visible transcript text after shared Claude canonicalization.
    pub rendered: String,
    /// File-state snapshot to stage or commit after distillation.
    pub watermark: SessionWatermark,
}

/// Discovered Claude transcript that survived filesystem and filename filters.
///
/// This is the pre-canonicalization shape: it has enough metadata to compare
/// against committed watermarks and apply the `last_distilled` floor without
/// reading the full file unless the session actually changed.
#[derive(Debug, Clone)]
struct ClaudeCandidate {
    /// Stable Claude session id taken from the UUID filename stem.
    session_id: String,
    /// Absolute filesystem path of the discovered transcript.
    path: PathBuf,
    /// Claude-home-relative label derived from `path`.
    file_label: String,
    /// File size at discovery time.
    size_bytes: u64,
    /// Modification time at discovery time.
    mtime_utc: DateTime<Utc>,
    /// Filesystem identity `(dev, ino)` captured at discovery (unix only).
    ///
    /// Later reads verify the opened handle against this identity so a
    /// symlink swapped in after discovery cannot redirect the read — the
    /// discovery-time type check alone would be check-then-use.
    file_id: Option<(u64, u64)>,
}

/// The result of one discovery pass over `<claude_home>/projects/`.
pub struct ClaudeScanOutcome {
    /// Sessions that are new or changed relative to committed watermarks,
    /// canonicalized and sorted for emission/staging.
    pub changed: Vec<DistilledClaudeSession>,
    /// Session ids the external store fully accounts for: everything
    /// discovered except floor-skipped sessions. Legacy hook-copied logs with
    /// these ids are suppressed — either the session is being emitted from
    /// the external copy right now, or its committed watermark says it was
    /// already distilled (the idle-exit case, where the hook copies a log
    /// after mark-distilled has committed the session).
    pub accounted_ids: std::collections::BTreeSet<String>,
}

/// Discover external Claude sessions and return changed sessions for this distill run.
///
/// The scan walks `<claude_home>/projects/` recursively, accepts only regular
/// `.jsonl` files whose filename stem is a UUID, compares each candidate
/// against committed watermarks, and canonicalizes only sessions that are new
/// or changed. A new unwatermarked session older than `last_distilled` is
/// skipped completely; that floor prevents the first scan-enabled run from
/// replaying sessions already learned through the legacy hook-copied log path.
pub fn collect_changed_sessions(
    claude_home: &Path,
    committed: &BTreeMap<String, SessionWatermark>,
    last_distilled: DateTime<Utc>,
) -> ClaudeScanOutcome {
    let mut discovered = Vec::new();
    collect_session_files(&claude_home.join("projects"), claude_home, &mut discovered);
    discovered.sort_by(|a, b| a.path.cmp(&b.path));

    // Collapse duplicate session ids to one candidate, newest mtime winning
    // (then size, then path for determinism). Claude Code normally writes one
    // file per UUID, but without this collapse a duplicated id would stage
    // only one watermark while emitting both files, and the loser would then
    // re-emit forever because its path never matches the committed watermark.
    let mut candidates: BTreeMap<String, ClaudeCandidate> = BTreeMap::new();
    for candidate in discovered {
        match candidates.entry(candidate.session_id.clone()) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(candidate);
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                if candidate_replaces_existing(&candidate, entry.get()) {
                    entry.insert(candidate);
                }
            }
        }
    }

    let mut sessions = Vec::new();
    let mut accounted_ids = std::collections::BTreeSet::new();
    for candidate in candidates.into_values() {
        if committed.get(&candidate.session_id).is_some_and(|prev| {
            prev.path == candidate.path.display().to_string()
                && prev.size_bytes == candidate.size_bytes
                && prev.mtime_utc == candidate.mtime_utc
        }) {
            // Unchanged and committed: the external store owns this session;
            // a late hook-copied log for it must not re-emit it.
            accounted_ids.insert(candidate.session_id);
            continue;
        }

        if !committed.contains_key(&candidate.session_id) && candidate.mtime_utc < last_distilled {
            // Floor-skipped: deliberately NOT accounted. If a legacy log with
            // this id exists above last_distilled, the legacy path is the only
            // copy that would ever emit it — suppressing it would lose content.
            continue;
        }

        match parse_changed_session(candidate) {
            Ok(session) => {
                accounted_ids.insert(session.session_id.clone());
                sessions.push(session);
            }
            Err(err) => warn!("failed to parse changed Claude transcript: {err}"),
        }
    }

    sessions.sort_by(|a, b| {
        a.sort_timestamp
            .cmp(&b.sort_timestamp)
            .then_with(|| a.session_id.cmp(&b.session_id))
    });
    ClaudeScanOutcome {
        changed: sessions,
        accounted_ids,
    }
}

/// Decide whether a newly discovered duplicate-id candidate displaces the one
/// already chosen: newest mtime wins, then larger size, then lexicographically
/// larger path purely for determinism.
fn candidate_replaces_existing(new: &ClaudeCandidate, existing: &ClaudeCandidate) -> bool {
    (new.mtime_utc, new.size_bytes, &new.path)
        > (existing.mtime_utc, existing.size_bytes, &existing.path)
}

/// Recursively collect Claude transcript candidates under one `projects/` tree.
///
/// This uses `DirEntry::file_type`, which is based on symlink metadata on the
/// supported platforms Rust targets here. That matters for the security
/// boundary: a UUID-named symlink must not cause distill to read an arbitrary
/// file outside Claude's transcript store.
fn collect_session_files(root: &Path, claude_home: &Path, out: &mut Vec<ClaudeCandidate>) {
    if !root.exists() {
        return;
    }

    let read_dir = match fs::read_dir(root) {
        Ok(read_dir) => read_dir,
        Err(err) => {
            warn!("failed to read Claude directory {}: {err}", root.display());
            return;
        }
    };

    for entry in read_dir {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                warn!(
                    "failed to enumerate Claude directory {}: {err}",
                    root.display()
                );
                continue;
            }
        };

        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(err) => {
                warn!("failed to inspect {}: {err}", path.display());
                continue;
            }
        };

        if file_type.is_dir() {
            collect_session_files(&path, claude_home, out);
            continue;
        }

        if !file_type.is_file() || path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
            continue;
        }

        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        if !is_uuid_stem(stem) {
            continue;
        }

        // DirEntry::metadata does not traverse symlinks, so size/mtime and
        // the identity below describe the entry the type check just vetted.
        let metadata = match entry.metadata() {
            Ok(meta) => meta,
            Err(err) => {
                warn!("failed to stat Claude transcript {}: {err}", path.display());
                continue;
            }
        };

        let mtime_utc = match metadata.modified() {
            Ok(ts) => DateTime::<Utc>::from(ts),
            Err(err) => {
                warn!(
                    "failed to read mtime for Claude transcript {}: {err}",
                    path.display()
                );
                continue;
            }
        };

        let file_label = sanitize_label(
            &path
                .strip_prefix(claude_home)
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| path.display().to_string()),
        );

        out.push(ClaudeCandidate {
            session_id: stem.to_string(),
            path,
            file_label,
            size_bytes: metadata.len(),
            mtime_utc,
            file_id: file_identity(&metadata),
        });
    }
}

/// Validate the filename-stem UUID format Claude Code currently uses.
///
/// The project does not otherwise depend on a UUID parser, and discovery only
/// needs a strict shape check. Accepted stems are exactly 36 characters in the
/// `8-4-4-4-12` layout with ASCII hex digits in either case.
fn is_uuid_stem(stem: &str) -> bool {
    let bytes = stem.as_bytes();
    if bytes.len() != 36 {
        return false;
    }
    for (idx, byte) in bytes.iter().enumerate() {
        match idx {
            8 | 13 | 18 | 23 => {
                if *byte != b'-' {
                    return false;
                }
            }
            _ => {
                if !byte.is_ascii_hexdigit() {
                    return false;
                }
            }
        }
    }
    true
}

/// Return the first RFC3339 `timestamp` among a transcript's leading records.
///
/// Claude transcripts do not have a dedicated `session_meta` header like
/// Codex. Scanning at most [`HEADER_SCAN_LIMIT`] lines gives stable ordering
/// for normal files; the caller falls back to file mtime when nothing usable
/// appears. Operates on already-read content so it shares the single verified
/// read in [`parse_changed_session`].
fn extract_sort_timestamp(content: &str) -> Option<DateTime<Utc>> {
    for line in content.lines().take(HEADER_SCAN_LIMIT) {
        let Ok(val) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if let Some(ts) = val
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(parse_timestamp)
        {
            return Some(ts);
        }
    }
    None
}

/// Filesystem identity used to detect a file being swapped between discovery
/// and read. `(dev, ino)` on unix; unavailable elsewhere, where the fallback
/// is the weaker is-regular-file check on the opened handle.
#[cfg(unix)]
fn file_identity(metadata: &fs::Metadata) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    Some((metadata.dev(), metadata.ino()))
}

#[cfg(not(unix))]
fn file_identity(_metadata: &fs::Metadata) -> Option<(u64, u64)> {
    None
}

/// Open a candidate transcript and verify the handle still refers to the
/// regular file seen at discovery.
///
/// The discovery-time symlink rejection is a point-in-time check; without
/// this handle-level verification a UUID-named symlink swapped in after the
/// walk would redirect the content read to an arbitrary user-readable file
/// (SPEC: "a UUID-named symlink must not be able to pull an arbitrary
/// readable file into the distill output"). Reading through the verified
/// handle, never by path, closes the race.
fn open_verified(path: &Path, expected_id: Option<(u64, u64)>) -> Result<fs::File> {
    let file = fs::File::open(path)
        .with_context(|| format!("failed to open Claude transcript {}", path.display()))?;
    let metadata = file
        .metadata()
        .with_context(|| format!("failed to stat opened Claude transcript {}", path.display()))?;
    if !metadata.is_file() {
        anyhow::bail!(
            "Claude transcript {} is no longer a regular file; skipping",
            path.display()
        );
    }
    if let (Some(expected), Some(actual)) = (expected_id, file_identity(&metadata))
        && expected != actual
    {
        anyhow::bail!(
            "Claude transcript {} changed identity since discovery; skipping",
            path.display()
        );
    }
    Ok(file)
}

/// Fully parse a changed Claude candidate into rendered content plus watermark.
///
/// This is intentionally reached only after file-state dedupe and the
/// `last_distilled` floor have decided the session should be staged. Empty
/// rendered content is kept in the returned value because staging, not
/// emission, is what stops empty sessions from being rescanned forever.
fn parse_changed_session(candidate: ClaudeCandidate) -> Result<DistilledClaudeSession> {
    use std::io::Read as _;
    let mut file = open_verified(&candidate.path, candidate.file_id)?;
    let mut content = String::new();
    file.read_to_string(&mut content).with_context(|| {
        format!(
            "failed to read Claude transcript {}",
            candidate.path.display()
        )
    })?;
    let rendered = render_session_log(&content)?;
    let session_timestamp_utc = extract_sort_timestamp(&content);

    let watermark = SessionWatermark {
        path: candidate.path.display().to_string(),
        size_bytes: candidate.size_bytes,
        mtime_utc: candidate.mtime_utc,
        session_timestamp_utc,
        latest_event_timestamp_utc: None,
    };

    Ok(DistilledClaudeSession {
        session_id: candidate.session_id,
        file_label: candidate.file_label,
        sort_timestamp: session_timestamp_utc.unwrap_or(candidate.mtime_utc),
        rendered,
        watermark,
    })
}

/// Strip characters that could forge or break the `<session file="...">`
/// attribute from a home-relative label.
///
/// The legacy path's labels were format-validated filenames; scanned labels
/// embed arbitrary directory names, so quotes, angle brackets, and control
/// characters are replaced before the label reaches tag position.
fn sanitize_label(label: &str) -> String {
    label
        .chars()
        .map(|ch| match ch {
            '"' | '\'' | '<' | '>' => '_',
            ch if ch.is_control() => '_',
            ch => ch,
        })
        .collect()
}

/// Parse one RFC3339 timestamp and normalize it to UTC.
///
/// Invalid timestamps simply do not participate in ordering; the caller falls
/// back to file mtime when no usable timestamp is found in the bounded header
/// scan.
fn parse_timestamp(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|ts| ts.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn write_session(home: &Path, rel: &str, lines: &[Value]) -> PathBuf {
        let path = home.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut rendered = String::new();
        for line in lines {
            rendered.push_str(&serde_json::to_string(line).unwrap());
            rendered.push('\n');
        }
        fs::write(&path, rendered).unwrap();
        path
    }

    fn user_line(text: &str, timestamp: &str) -> Value {
        serde_json::json!({
            "timestamp": timestamp,
            "type": "user",
            "message": {"content": text}
        })
    }

    fn progress_line() -> Value {
        serde_json::json!({"type": "progress", "data": {"type": "agent_progress"}})
    }

    fn ts(year: i32, month: u32, day: u32, hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, hour, 0, 0).unwrap()
    }

    fn set_mtime(path: &Path, timestamp: DateTime<Utc>) {
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(std::time::SystemTime::from(timestamp)))
            .unwrap();
    }

    /// Shorthand for tests that only care about the changed-session list.
    fn changed_sessions(
        home: &Path,
        committed: &BTreeMap<String, SessionWatermark>,
        floor: DateTime<Utc>,
    ) -> Vec<DistilledClaudeSession> {
        collect_changed_sessions(home, committed, floor).changed
    }

    #[test]
    fn uuid_stem_accepts_current_claude_shape_only() {
        assert!(is_uuid_stem("0198fb08-e6e4-7a41-8b3f-2fc8a9ee215d"));
        assert!(is_uuid_stem("0198FB08-E6E4-7A41-8B3F-2FC8A9EE215D"));
        assert!(!is_uuid_stem("not-a-uuid"));
        assert!(!is_uuid_stem("0198fb08e6e47a418b3f2fc8a9ee215d"));
        assert!(!is_uuid_stem("0198fb08-e6e4-7a41-8b3f-2fc8a9ee215z"));
    }

    #[test]
    fn discovery_filters_non_sessions_and_symlinks() {
        let home = tempfile::tempdir().unwrap();
        let good = "0198fb08-e6e4-7a41-8b3f-2fc8a9ee215d";
        write_session(
            home.path(),
            &format!("projects/proj/{good}.jsonl"),
            &[user_line("keep me", "2026-07-01T12:00:00Z")],
        );
        write_session(
            home.path(),
            "projects/proj/not-a-uuid.jsonl",
            &[user_line("skip non uuid", "2026-07-01T12:00:00Z")],
        );
        fs::write(home.path().join("projects/proj/notes.md"), "skip non jsonl").unwrap();
        fs::create_dir_all(home.path().join("projects/proj/memory")).unwrap();
        fs::write(home.path().join("projects/proj/memory/memo.md"), "skip md").unwrap();

        #[cfg(unix)]
        {
            let target = home.path().join("outside.jsonl");
            fs::write(&target, "symlink target").unwrap();
            std::os::unix::fs::symlink(
                &target,
                home.path()
                    .join("projects/proj/11111111-2222-3333-4444-555555555555.jsonl"),
            )
            .unwrap();

            let symlinked_dir = "22222222-3333-4444-5555-666666666666";
            let nested_id = "33333333-4444-5555-6666-777777777777";
            let target_dir = home.path().join("outside-dir");
            fs::create_dir_all(&target_dir).unwrap();
            fs::write(
                target_dir.join(format!("{nested_id}.jsonl")),
                serde_json::to_string(&user_line("skip symlinked dir", "2026-07-01T12:00:00Z"))
                    .unwrap(),
            )
            .unwrap();
            std::os::unix::fs::symlink(
                &target_dir,
                home.path().join("projects/proj").join(symlinked_dir),
            )
            .unwrap();
        }

        let sessions = changed_sessions(home.path(), &BTreeMap::new(), ts(1970, 1, 1, 0));
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, good);
        assert!(sessions[0].rendered.contains("keep me"));
        assert!(!sessions[0].rendered.contains("skip symlinked dir"));
    }

    #[test]
    fn unchanged_committed_session_is_skipped_and_changed_session_reemits() {
        let home = tempfile::tempdir().unwrap();
        let session_id = "0198fb08-e6e4-7a41-8b3f-2fc8a9ee215d";
        let path = write_session(
            home.path(),
            &format!("projects/proj/{session_id}.jsonl"),
            &[user_line("first", "2026-07-01T12:00:00Z")],
        );

        let first = changed_sessions(home.path(), &BTreeMap::new(), ts(1970, 1, 1, 0));
        assert_eq!(first.len(), 1);
        let mut committed = BTreeMap::new();
        committed.insert(session_id.to_string(), first[0].watermark.clone());

        assert!(changed_sessions(home.path(), &committed, ts(1970, 1, 1, 0)).is_empty());

        fs::write(
            &path,
            format!(
                "{}\n{}\n",
                serde_json::to_string(&user_line("first", "2026-07-01T12:00:00Z")).unwrap(),
                serde_json::to_string(&user_line("second", "2026-07-01T12:00:01Z")).unwrap()
            ),
        )
        .unwrap();
        let changed = changed_sessions(home.path(), &committed, ts(1970, 1, 1, 0));
        assert_eq!(changed.len(), 1);
        assert!(changed[0].rendered.contains("first"));
        assert!(changed[0].rendered.contains("second"));
    }

    #[test]
    fn old_unwatermarked_session_is_below_last_distilled_floor() {
        let home = tempfile::tempdir().unwrap();
        let session_id = "0198fb08-e6e4-7a41-8b3f-2fc8a9ee215d";
        let path = write_session(
            home.path(),
            &format!("projects/proj/{session_id}.jsonl"),
            &[user_line("too old", "2026-07-01T12:00:00Z")],
        );
        let old = std::time::SystemTime::from(ts(2026, 1, 1, 0));
        let file = fs::File::options().write(true).open(&path).unwrap();
        file.set_times(fs::FileTimes::new().set_modified(old))
            .unwrap();

        let sessions = changed_sessions(home.path(), &BTreeMap::new(), ts(2026, 6, 1, 0));
        assert!(sessions.is_empty());
    }

    #[test]
    fn changed_committed_session_ignores_last_distilled_floor() {
        let home = tempfile::tempdir().unwrap();
        let session_id = "0198fb08-e6e4-7a41-8b3f-2fc8a9ee215d";
        let path = write_session(
            home.path(),
            &format!("projects/proj/{session_id}.jsonl"),
            &[user_line("before mark", "2026-01-01T00:00:00Z")],
        );
        set_mtime(&path, ts(2026, 1, 1, 0));

        let first = changed_sessions(home.path(), &BTreeMap::new(), ts(1970, 1, 1, 0));
        assert_eq!(first.len(), 1);
        let mut committed = BTreeMap::new();
        committed.insert(session_id.to_string(), first[0].watermark.clone());

        write_session(
            home.path(),
            &format!("projects/proj/{session_id}.jsonl"),
            &[
                user_line("before mark", "2026-01-01T00:00:00Z"),
                user_line("after mark", "2026-01-01T00:00:01Z"),
            ],
        );
        set_mtime(&path, ts(2026, 1, 2, 0));

        let changed = changed_sessions(home.path(), &committed, ts(2026, 6, 1, 0));
        assert_eq!(changed.len(), 1);
        assert!(changed[0].rendered.contains("before mark"));
        assert!(changed[0].rendered.contains("after mark"));
    }

    #[test]
    fn unwatermarked_session_at_last_distilled_floor_is_emitted() {
        let home = tempfile::tempdir().unwrap();
        let session_id = "0198fb08-e6e4-7a41-8b3f-2fc8a9ee215d";
        let floor = ts(2026, 6, 1, 0);
        let path = write_session(
            home.path(),
            &format!("projects/proj/{session_id}.jsonl"),
            &[user_line("exact floor", "2026-06-01T00:00:00Z")],
        );
        set_mtime(&path, floor);

        let sessions = changed_sessions(home.path(), &BTreeMap::new(), floor);
        assert_eq!(sessions.len(), 1);
        assert!(sessions[0].rendered.contains("exact floor"));
    }

    #[test]
    fn duplicate_session_ids_collapse_to_newest_candidate() {
        let home = tempfile::tempdir().unwrap();
        let session_id = "0198fb08-e6e4-7a41-8b3f-2fc8a9ee215d";
        let older = write_session(
            home.path(),
            &format!("projects/older/{session_id}.jsonl"),
            &[user_line("older duplicate", "2026-07-01T12:00:00Z")],
        );
        let newer = write_session(
            home.path(),
            &format!("projects/newer/{session_id}.jsonl"),
            &[user_line("newer duplicate", "2026-07-01T12:00:00Z")],
        );
        set_mtime(&older, ts(2026, 7, 1, 12));
        set_mtime(&newer, ts(2026, 7, 1, 13));

        let outcome = collect_changed_sessions(home.path(), &BTreeMap::new(), ts(1970, 1, 1, 0));

        assert_eq!(outcome.changed.len(), 1);
        assert_eq!(outcome.changed[0].session_id, session_id);
        assert!(outcome.changed[0].rendered.contains("newer duplicate"));
        assert!(!outcome.changed[0].rendered.contains("older duplicate"));
        assert_eq!(
            outcome.changed[0].watermark.path,
            newer.display().to_string()
        );
        assert_eq!(outcome.accounted_ids.len(), 1);
        assert!(outcome.accounted_ids.contains(session_id));
    }

    #[test]
    fn accounted_ids_distinguish_committed_floor_skipped_and_changed_sessions() {
        let home = tempfile::tempdir().unwrap();
        let committed_id = "0198fb08-e6e4-7a41-8b3f-2fc8a9ee215d";
        let floor_id = "1198fb08-e6e4-7a41-8b3f-2fc8a9ee215d";
        let changed_id = "2198fb08-e6e4-7a41-8b3f-2fc8a9ee215d";

        let committed_path = write_session(
            home.path(),
            &format!("projects/proj/{committed_id}.jsonl"),
            &[user_line("already committed", "2026-07-01T12:00:00Z")],
        );
        let first = changed_sessions(home.path(), &BTreeMap::new(), ts(1970, 1, 1, 0));
        let committed_watermark = first
            .iter()
            .find(|session| session.session_id == committed_id)
            .unwrap()
            .watermark
            .clone();

        let floor_path = write_session(
            home.path(),
            &format!("projects/proj/{floor_id}.jsonl"),
            &[user_line("floor skipped", "2026-01-01T00:00:00Z")],
        );
        set_mtime(&floor_path, ts(2026, 1, 1, 0));
        let changed_path = write_session(
            home.path(),
            &format!("projects/proj/{changed_id}.jsonl"),
            &[user_line("changed", "2026-07-01T12:00:00Z")],
        );
        set_mtime(&changed_path, ts(2026, 7, 1, 12));
        assert!(committed_path.exists());

        let mut committed = BTreeMap::new();
        committed.insert(committed_id.to_string(), committed_watermark);
        let outcome = collect_changed_sessions(home.path(), &committed, ts(2026, 6, 1, 0));
        let changed_ids: Vec<_> = outcome
            .changed
            .iter()
            .map(|session| session.session_id.as_str())
            .collect();

        assert!(outcome.accounted_ids.contains(committed_id));
        assert!(!changed_ids.contains(&committed_id));
        assert!(!outcome.accounted_ids.contains(floor_id));
        assert!(!changed_ids.contains(&floor_id));
        assert!(outcome.accounted_ids.contains(changed_id));
        assert!(changed_ids.contains(&changed_id));
    }

    #[test]
    fn empty_rendered_session_is_still_returned_for_staging() {
        let home = tempfile::tempdir().unwrap();
        let session_id = "0198fb08-e6e4-7a41-8b3f-2fc8a9ee215d";
        write_session(
            home.path(),
            &format!("projects/proj/{session_id}.jsonl"),
            &[progress_line()],
        );

        let sessions = changed_sessions(home.path(), &BTreeMap::new(), ts(1970, 1, 1, 0));
        assert_eq!(sessions.len(), 1);
        assert!(sessions[0].rendered.is_empty());
    }

    #[test]
    fn sort_timestamp_uses_first_leading_timestamp_then_mtime() {
        let home = tempfile::tempdir().unwrap();
        let first = "0198fb08-e6e4-7a41-8b3f-2fc8a9ee215d";
        let second = "1198fb08-e6e4-7a41-8b3f-2fc8a9ee215d";
        let path_no_ts = write_session(
            home.path(),
            &format!("projects/proj/{second}.jsonl"),
            &[serde_json::json!({"type": "future", "message": "no timestamp"})],
        );
        let old = std::time::SystemTime::from(ts(2026, 1, 1, 0));
        fs::File::options()
            .write(true)
            .open(&path_no_ts)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(old))
            .unwrap();

        let path_with_ts = home.path().join(format!("projects/proj/{first}.jsonl"));
        fs::create_dir_all(path_with_ts.parent().unwrap()).unwrap();
        fs::write(
            &path_with_ts,
            format!(
                "{}\n{}\n{}",
                serde_json::json!({"type": "system"}),
                serde_json::json!({"type": "progress"}),
                user_line("timestamped", "2026-07-01T12:00:00Z")
            ),
        )
        .unwrap();

        let sessions = changed_sessions(home.path(), &BTreeMap::new(), ts(1970, 1, 1, 0));
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].session_id, second);
        assert_eq!(sessions[1].session_id, first);
        assert_eq!(sessions[1].sort_timestamp, ts(2026, 7, 1, 12));
    }

    #[cfg(unix)]
    #[test]
    fn open_verified_accepts_matching_identity_and_rejects_mismatches() {
        use std::os::unix::fs::MetadataExt;

        let home = tempfile::tempdir().unwrap();
        let path = home.path().join("session.jsonl");
        fs::write(&path, "content").unwrap();
        let metadata = fs::metadata(&path).unwrap();
        let identity = (metadata.dev(), metadata.ino());

        assert!(open_verified(&path, Some(identity)).is_ok());

        let err = open_verified(&path, Some((identity.0, identity.1.saturating_add(1))))
            .unwrap_err()
            .to_string();
        assert!(err.contains("changed identity"));

        let dir_err = open_verified(home.path(), None).unwrap_err().to_string();
        assert!(dir_err.contains("regular file"));
    }

    #[test]
    fn sanitize_label_replaces_tag_breaking_characters_only() {
        assert_eq!(
            sanitize_label("projects/proj/normal-file.jsonl"),
            "projects/proj/normal-file.jsonl"
        );
        assert_eq!(
            sanitize_label("projects/\"quote\"/'single'/<tag>/line\n/control\u{0007}.jsonl"),
            "projects/_quote_/_single_/_tag_/line_/control_.jsonl"
        );
    }
}
