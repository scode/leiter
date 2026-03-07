//! Best-effort Codex session discovery and transcript canonicalization.
//!
//! We only read rollout JSONL files under `~/.codex/`; we never mutate them or
//! consult any Codex SQLite state. Per-session file metadata decides whether an
//! unchanged session can be skipped entirely.

pub mod meta;

use std::collections::{BTreeMap, btree_map::Entry};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde_json::Value;
use tracing::warn;

pub use self::meta::{CodexMeta, CodexSessionMeta};

/// Canonicalized Codex session content that is ready to include in
/// `leiter soul distill` output.
///
/// This is produced after leiter discovers a rollout file, decides the session
/// changed relative to committed metadata, and renders the user-visible parts
/// of that session into a stable text form. A typical `file_label` looks like
/// `sessions/2026/03/07/rollout-2026-03-07T10-06-57-019cc97b-26c8-7c83-8566-0068c624816d.jsonl`.
#[derive(Debug, Clone)]
pub struct DistilledCodexSession {
    /// Stable Codex session id from the rollout's leading `session_meta`.
    pub session_id: String,
    /// Home-relative rollout path shown in distill output.
    pub file_label: String,
    /// Timestamp used to order emitted Codex sessions.
    pub sort_timestamp: DateTime<Utc>,
    /// User-visible transcript text after Codex-specific canonicalization.
    pub rendered: String,
    /// File-state snapshot to stage or commit after distillation.
    pub watermark: CodexSessionMeta,
}

/// Discovered rollout file that looks like the current best on-disk
/// representative for a Codex session, before full parsing/canonicalization.
///
/// "Candidate" means leiter has enough header and filesystem metadata to
/// consider this file for distillation, but has not yet re-read the whole file
/// to produce rendered session content.
#[derive(Debug, Clone)]
struct CodexCandidate {
    /// Stable Codex session id from the header.
    session_id: String,
    /// Absolute filesystem path of the discovered rollout file.
    path: PathBuf,
    /// Home-relative label derived from `path`.
    file_label: String,
    /// File size at discovery time.
    size_bytes: u64,
    /// Modification time at discovery time.
    mtime_utc: DateTime<Utc>,
    /// Session timestamp from the leading `session_meta`, when present.
    session_timestamp_utc: Option<DateTime<Utc>>,
}

/// Minimal session identity metadata extracted from the first `session_meta`
/// record in a rollout file.
///
/// Leiter reads this header first so it can decide which discovered files
/// belong to which Codex session, compare them against committed watermarks,
/// and sort changed sessions without fully parsing every file up front.
#[derive(Debug, Clone)]
struct CodexHeader {
    /// Stable Codex session id from `session_meta.payload.id`.
    session_id: String,
    /// Session timestamp from `session_meta.payload.timestamp`, when present.
    session_timestamp_utc: Option<DateTime<Utc>>,
}

/// Discover Codex rollout files and return only the sessions that should be
/// emitted by the current `leiter soul distill` run.
///
/// This scans both live and archived rollout trees, groups files by stable
/// session id, chooses the current best file for each session, compares that
/// file state against the committed watermark, and fully parses only the
/// sessions whose file changed. The returned sessions are already
/// canonicalized and sorted for distill output.
pub fn collect_changed_sessions(
    codex_home: &Path,
    committed: &BTreeMap<String, CodexSessionMeta>,
) -> Vec<DistilledCodexSession> {
    let mut files = Vec::new();
    collect_rollout_files(&codex_home.join("sessions"), codex_home, &mut files);
    collect_rollout_files(
        &codex_home.join("archived_sessions"),
        codex_home,
        &mut files,
    );
    files.sort_by(|a, b| a.0.cmp(&b.0));

    let mut candidates = BTreeMap::new();
    for (path, file_label) in files {
        let metadata = match fs::metadata(&path) {
            Ok(meta) => meta,
            Err(err) => {
                warn!("failed to stat Codex transcript {}: {err}", path.display());
                continue;
            }
        };

        let mtime_utc = match metadata.modified() {
            Ok(ts) => DateTime::<Utc>::from(ts),
            Err(err) => {
                warn!(
                    "failed to read mtime for Codex transcript {}: {err}",
                    path.display()
                );
                continue;
            }
        };

        let header = match inspect_header(&path) {
            Ok(Some(header)) => header,
            Ok(None) => continue,
            Err(err) => {
                warn!(
                    "failed to inspect Codex transcript {}: {err}",
                    path.display()
                );
                continue;
            }
        };

        let candidate = CodexCandidate {
            session_id: header.session_id,
            path,
            file_label,
            size_bytes: metadata.len(),
            mtime_utc,
            session_timestamp_utc: header.session_timestamp_utc,
        };

        match candidates.entry(candidate.session_id.clone()) {
            Entry::Vacant(entry) => {
                entry.insert(candidate);
            }
            Entry::Occupied(mut entry) => {
                if candidate_replaces_existing(&candidate, entry.get()) {
                    entry.insert(candidate);
                }
            }
        }
    }

    let mut sessions = Vec::new();
    for candidate in candidates.into_values() {
        if committed.get(&candidate.session_id).is_some_and(|prev| {
            prev.path == candidate.path.display().to_string()
                && prev.size_bytes == candidate.size_bytes
                && prev.mtime_utc == candidate.mtime_utc
        }) {
            continue;
        }

        match parse_changed_session(candidate) {
            Ok(Some(session)) => sessions.push(session),
            Ok(None) => {}
            Err(err) => {
                warn!("failed to parse changed Codex transcript: {err}");
            }
        }
    }

    sessions.sort_by(|a, b| {
        a.sort_timestamp
            .cmp(&b.sort_timestamp)
            .then_with(|| a.session_id.cmp(&b.session_id))
    });
    sessions
}

/// Recursively collect rollout `.jsonl` files under one Codex subtree.
///
/// This is the filesystem discovery pass for Codex distillation. It walks a
/// live or archived root, keeps only regular `.jsonl` files, and records each
/// file both as an absolute path and as a home-relative label suitable for
/// distill output.
fn collect_rollout_files(root: &Path, codex_home: &Path, out: &mut Vec<(PathBuf, String)>) {
    if !root.exists() {
        return;
    }

    let read_dir = match fs::read_dir(root) {
        Ok(read_dir) => read_dir,
        Err(err) => {
            warn!("failed to read Codex directory {}: {err}", root.display());
            return;
        }
    };

    for entry in read_dir {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                warn!(
                    "failed to enumerate Codex directory {}: {err}",
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
            collect_rollout_files(&path, codex_home, out);
            continue;
        }

        if !file_type.is_file() || path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
            continue;
        }

        let label = path
            .strip_prefix(codex_home)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| path.display().to_string());
        out.push((path, label));
    }
}

/// Read just the first record of a rollout file and extract the minimal header
/// information leiter needs before committing to a full parse.
///
/// This exists so discovery can cheaply identify the stable session id and
/// session timestamp for a file, reject malformed or unsupported rollout
/// files early, and compare candidate files against committed watermarks
/// without first canonicalizing the entire transcript.
fn inspect_header(path: &Path) -> Result<Option<CodexHeader>> {
    let file = fs::File::open(path)
        .with_context(|| format!("failed to open Codex transcript {}", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut first_line = String::new();
    if reader
        .read_line(&mut first_line)
        .with_context(|| format!("failed to read Codex transcript {}", path.display()))?
        == 0
    {
        warn!(
            "skipping empty Codex transcript without leading session_meta: {}",
            path.display()
        );
        return Ok(None);
    }

    let val: Value = serde_json::from_str(first_line.trim_end()).with_context(|| {
        format!(
            "failed to parse first JSONL record in Codex transcript {}",
            path.display()
        )
    })?;

    if val.get("type").and_then(Value::as_str) != Some("session_meta") {
        warn!(
            "skipping Codex transcript without leading session_meta: {}",
            path.display()
        );
        return Ok(None);
    }

    let Some(payload) = val.get("payload") else {
        warn!(
            "skipping Codex transcript with malformed leading session_meta payload: {}",
            path.display()
        );
        return Ok(None);
    };
    let Some(session_id) = payload.get("id").and_then(Value::as_str) else {
        warn!(
            "skipping Codex transcript without session_meta.payload.id: {}",
            path.display()
        );
        return Ok(None);
    };
    let session_timestamp_utc = payload
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(parse_timestamp);

    Ok(Some(CodexHeader {
        session_id: session_id.to_string(),
        session_timestamp_utc,
    }))
}

/// Decide whether a newly discovered rollout file is a better representative
/// for a session than the one we have already seen during this scan.
///
/// We prefer newer files first. When mtimes tie, prefer the larger file because
/// Codex rollouts are append-oriented and size is the best local signal for the
/// more complete transcript. File label is only a deterministic last
/// tie-breaker.
fn candidate_replaces_existing(candidate: &CodexCandidate, existing: &CodexCandidate) -> bool {
    candidate.mtime_utc > existing.mtime_utc
        || (candidate.mtime_utc == existing.mtime_utc
            && (candidate.size_bytes > existing.size_bytes
                || (candidate.size_bytes == existing.size_bytes
                    && candidate.file_label > existing.file_label)))
}

/// Fully parse a discovered Codex session whose file state already differs
/// from the committed watermark.
///
/// The "changed" part matters because this is intentionally not the generic
/// parse path for every discovered rollout file. `collect_changed_sessions`
/// only calls this after dedupe has decided that the candidate's file state is
/// new or different enough that the session should be emitted again. This
/// function then canonicalizes the whole rollout, computes the latest event
/// timestamp, and builds the new watermark that may later be staged/committed.
fn parse_changed_session(candidate: CodexCandidate) -> Result<Option<DistilledCodexSession>> {
    let content = fs::read_to_string(&candidate.path).with_context(|| {
        format!(
            "failed to read Codex transcript {}",
            candidate.path.display()
        )
    })?;

    let mut rendered_items = Vec::new();
    let mut latest_event_timestamp_utc = None;

    for line in content.lines() {
        let Ok(val) = serde_json::from_str::<Value>(line) else {
            continue;
        };

        if let Some(ts) = val
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(parse_timestamp)
        {
            latest_event_timestamp_utc = Some(match latest_event_timestamp_utc {
                Some(prev) if prev > ts => prev,
                _ => ts,
            });
        }

        for item in canonicalize_line(&val) {
            push_unique(&mut rendered_items, item);
        }
    }

    let rendered = if rendered_items.is_empty() {
        String::new()
    } else {
        format!("{}\n", rendered_items.join("\n"))
    };

    let watermark = CodexSessionMeta {
        path: candidate.path.display().to_string(),
        size_bytes: candidate.size_bytes,
        mtime_utc: candidate.mtime_utc,
        session_timestamp_utc: candidate.session_timestamp_utc,
        latest_event_timestamp_utc,
    };

    Ok(Some(DistilledCodexSession {
        session_id: candidate.session_id,
        file_label: candidate.file_label,
        sort_timestamp: candidate
            .session_timestamp_utc
            .unwrap_or(candidate.mtime_utc),
        rendered,
        watermark,
    }))
}

/// Append a canonicalized transcript line unless it is empty or an immediate
/// duplicate of the line we just emitted.
///
/// This intentionally does not use a set. We only want to collapse accidental
/// adjacent duplicates produced while canonicalizing Codex events; the same
/// rendered line may legitimately appear again later in the session, and a set
/// would erase that distinction while also discarding original transcript
/// order.
fn push_unique(items: &mut Vec<String>, item: String) {
    if item.is_empty() {
        return;
    }
    if items.last() == Some(&item) {
        return;
    }
    items.push(item);
}

/// Route one parsed rollout record to the Codex-specific canonicalization
/// helper that knows how to turn it into user-visible transcript lines.
///
/// This is the top-level event filter for Codex rollout parsing: unsupported
/// record types are dropped here so only relevant user-facing events continue
/// through the distillation pipeline.
fn canonicalize_line(val: &Value) -> Vec<String> {
    match val.get("type").and_then(Value::as_str) {
        Some("response_item") => canonicalize_response_item(val.get("payload")),
        Some("event_msg") => canonicalize_event_msg(val.get("payload")),
        _ => Vec::new(),
    }
}

/// Canonicalize a `response_item` payload into zero or more transcript lines.
///
/// `response_item` is the main payload family for Codex assistant/user
/// interaction records. We keep normal messages and function-call summaries,
/// and intentionally drop other response item variants that are not useful for
/// soul distillation.
fn canonicalize_response_item(payload: Option<&Value>) -> Vec<String> {
    let Some(payload) = payload else {
        return Vec::new();
    };

    match payload.get("type").and_then(Value::as_str) {
        Some("message") => canonicalize_message(payload),
        Some("function_call") => summarize_function_call(payload)
            .into_iter()
            .map(|summary| format!("[assistant tool]: {summary}"))
            .collect(),
        _ => Vec::new(),
    }
}

/// Canonicalize a Codex message payload into `[user]: ...` or
/// `[assistant]: ...` transcript lines.
///
/// This exists so message-role handling stays separate from lower-level text
/// extraction. It preserves only user/assistant messages because those are the
/// conversational parts relevant to distillation.
fn canonicalize_message(payload: &Value) -> Vec<String> {
    let Some(role) = payload.get("role").and_then(Value::as_str) else {
        return Vec::new();
    };

    match role {
        "user" => extract_message_text(payload)
            .into_iter()
            .map(|text| format!("[user]: {text}"))
            .collect(),
        "assistant" => extract_message_text(payload)
            .into_iter()
            .map(|text| format!("[assistant]: {text}"))
            .collect(),
        _ => Vec::new(),
    }
}

/// Canonicalize a Codex `event_msg` payload into transcript lines when that
/// event represents user-visible assistant commentary.
///
/// These events are separate from normal assistant message payloads, but some
/// of them correspond to commentary updates the user saw during the session, so
/// we preserve those as assistant lines for distillation context.
fn canonicalize_event_msg(payload: Option<&Value>) -> Vec<String> {
    let Some(payload) = payload else {
        return Vec::new();
    };

    match payload.get("type").and_then(Value::as_str) {
        Some("agent_message") => payload
            .get("message")
            .and_then(Value::as_str)
            .map(|msg| vec![format!("[assistant]: {msg}")])
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// Extract visible text fragments from a Codex message payload's `content`
/// field and join them into transcript-ready text blocks.
///
/// Codex message content can be either a plain string or an array of typed
/// blocks. This helper centralizes the block filtering so higher-level
/// canonicalization can talk in terms of user/assistant messages instead of
/// content-shape details.
fn extract_message_text(payload: &Value) -> Vec<String> {
    let Some(content) = payload.get("content") else {
        return Vec::new();
    };

    if let Some(s) = content.as_str() {
        return vec![s.to_string()];
    }

    let Some(blocks) = content.as_array() else {
        return Vec::new();
    };

    let parts: Vec<String> = blocks
        .iter()
        .filter_map(|block| {
            let block_type = block.get("type").and_then(Value::as_str)?;
            match block_type {
                "input_text" | "output_text" | "text" | "summary_text" => block
                    .get("text")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                _ => None,
            }
        })
        .collect();

    if parts.is_empty() {
        Vec::new()
    } else {
        vec![parts.join("\n\n")]
    }
}

/// Build a compact one-line summary for a Codex function call payload.
///
/// Distillation does not need raw tool arguments in full, but it is useful to
/// preserve that a tool was invoked and what its main target was. This helper
/// converts the structured function call into the `[assistant tool]: ...`
/// summary form used elsewhere in transcript output.
fn summarize_function_call(payload: &Value) -> Option<String> {
    let name = payload.get("name").and_then(Value::as_str)?;
    let args = payload.get("arguments")?;
    let parsed_args = if let Some(s) = args.as_str() {
        serde_json::from_str::<Value>(s).ok()
    } else {
        Some(args.clone())
    };

    let param = parsed_args.as_ref().and_then(extract_tool_param);
    Some(match param {
        Some(param) => format!("{name}({param})"),
        None => name.to_string(),
    })
}

fn extract_tool_param(args: &Value) -> Option<String> {
    if let Some(value) = args.get("file_path").and_then(Value::as_str) {
        return Some(value.to_string());
    }
    if let Some(value) = args.get("path").and_then(Value::as_str) {
        return Some(value.to_string());
    }
    if let Some(value) = args.get("command").and_then(Value::as_str) {
        return Some(truncate_command(value));
    }
    if let Some(value) = args.get("cmd").and_then(Value::as_str) {
        return Some(truncate_command(value));
    }
    if let Some(value) = args.get("pattern").and_then(Value::as_str) {
        return Some(value.to_string());
    }
    None
}

fn truncate_command(value: &str) -> String {
    let truncated: String = value.chars().take(120).collect();
    if truncated.len() < value.len() {
        format!("{truncated}...")
    } else {
        truncated
    }
}

fn parse_timestamp(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|ts| ts.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn write_rollout(root: &Path, rel: &str, lines: &[Value]) -> PathBuf {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut rendered = String::new();
        for line in lines {
            rendered.push_str(&serde_json::to_string(line).unwrap());
            rendered.push('\n');
        }
        fs::write(&path, rendered).unwrap();
        path
    }

    fn session_meta(id: &str, session_ts: &str) -> Value {
        serde_json::json!({
            "timestamp": session_ts,
            "type": "session_meta",
            "payload": {
                "id": id,
                "timestamp": session_ts
            }
        })
    }

    #[test]
    fn unchanged_session_skipped() {
        let home = tempfile::tempdir().unwrap();
        let path = write_rollout(
            home.path(),
            "sessions/2026/03/07/rollout-1.jsonl",
            &[session_meta("sess", "2026-03-07T18:00:00Z")],
        );
        let metadata = fs::metadata(&path).unwrap();
        let mtime = DateTime::<Utc>::from(metadata.modified().unwrap());
        let mut committed = BTreeMap::new();
        committed.insert(
            "sess".to_string(),
            CodexSessionMeta {
                path: path.display().to_string(),
                size_bytes: metadata.len(),
                mtime_utc: mtime,
                session_timestamp_utc: Some(Utc.with_ymd_and_hms(2026, 3, 7, 18, 0, 0).unwrap()),
                latest_event_timestamp_utc: Some(
                    Utc.with_ymd_and_hms(2026, 3, 7, 18, 0, 0).unwrap(),
                ),
            },
        );

        let sessions = collect_changed_sessions(home.path(), &committed);
        assert!(sessions.is_empty());
    }

    #[test]
    fn changed_session_emits_user_visible_content() {
        let home = tempfile::tempdir().unwrap();
        write_rollout(
            home.path(),
            "sessions/2026/03/07/rollout-1.jsonl",
            &[
                session_meta("sess", "2026-03-07T18:00:00Z"),
                serde_json::json!({
                    "timestamp": "2026-03-07T18:00:01Z",
                    "type": "response_item",
                    "payload": {
                        "type": "message",
                        "role": "user",
                        "content": [{"type": "input_text", "text": "hello"}]
                    }
                }),
                serde_json::json!({
                    "timestamp": "2026-03-07T18:00:02Z",
                    "type": "response_item",
                    "payload": {
                        "type": "function_call",
                        "name": "exec_command",
                        "arguments": "{\"cmd\":\"cargo test\"}"
                    }
                }),
                serde_json::json!({
                    "timestamp": "2026-03-07T18:00:03Z",
                    "type": "event_msg",
                    "payload": {
                        "type": "agent_message",
                        "message": "Inspecting files"
                    }
                }),
                serde_json::json!({
                    "timestamp": "2026-03-07T18:00:04Z",
                    "type": "response_item",
                    "payload": {
                        "type": "message",
                        "role": "assistant",
                        "content": [{"type": "output_text", "text": "done"}]
                    }
                }),
            ],
        );

        let sessions = collect_changed_sessions(home.path(), &BTreeMap::new());
        assert_eq!(sessions.len(), 1);
        let rendered = &sessions[0].rendered;
        assert!(rendered.contains("[user]: hello"));
        assert!(rendered.contains("[assistant tool]: exec_command(cargo test)"));
        assert!(rendered.contains("[assistant]: Inspecting files"));
        assert!(rendered.contains("[assistant]: done"));
    }

    #[test]
    fn duplicate_session_ids_collapse_to_one_candidate() {
        let home = tempfile::tempdir().unwrap();
        write_rollout(
            home.path(),
            "sessions/rollout-live.jsonl",
            &[session_meta("sess", "2026-03-07T18:00:00Z")],
        );
        write_rollout(
            home.path(),
            "archived_sessions/rollout-archived.jsonl",
            &[
                session_meta("sess", "2026-03-07T18:00:00Z"),
                serde_json::json!({
                    "timestamp": "2026-03-07T18:00:10Z",
                    "type": "response_item",
                    "payload": {
                        "type": "message",
                        "role": "assistant",
                        "content": [{"type": "output_text", "text": "archived"}]
                    }
                }),
            ],
        );

        let sessions = collect_changed_sessions(home.path(), &BTreeMap::new());
        assert_eq!(sessions.len(), 1);
    }

    #[test]
    fn larger_candidate_wins_when_mtime_ties() {
        let ts = Utc.with_ymd_and_hms(2026, 3, 7, 18, 0, 0).unwrap();
        let existing = CodexCandidate {
            session_id: "sess".to_string(),
            path: PathBuf::from("/tmp/sessions/rollout-live.jsonl"),
            file_label: "sessions/rollout-live.jsonl".to_string(),
            size_bytes: 100,
            mtime_utc: ts,
            session_timestamp_utc: Some(ts),
        };
        let candidate = CodexCandidate {
            session_id: "sess".to_string(),
            path: PathBuf::from("/tmp/archived_sessions/rollout-archived.jsonl"),
            file_label: "archived_sessions/rollout-archived.jsonl".to_string(),
            size_bytes: 200,
            mtime_utc: ts,
            session_timestamp_utc: Some(ts),
        };

        assert!(candidate_replaces_existing(&candidate, &existing));
        assert!(!candidate_replaces_existing(&existing, &candidate));
    }

    #[test]
    fn moved_session_path_counts_as_changed() {
        let home = tempfile::tempdir().unwrap();
        let old_path = home.path().join("sessions/rollout-live.jsonl");
        let new_path = write_rollout(
            home.path(),
            "archived_sessions/rollout-archived.jsonl",
            &[
                session_meta("sess", "2026-03-07T18:00:00Z"),
                serde_json::json!({
                    "timestamp": "2026-03-07T18:00:01Z",
                    "type": "response_item",
                    "payload": {
                        "type": "message",
                        "role": "assistant",
                        "content": [{"type": "output_text", "text": "moved"}]
                    }
                }),
            ],
        );
        let metadata = fs::metadata(&new_path).unwrap();
        let mtime = DateTime::<Utc>::from(metadata.modified().unwrap());

        let mut committed = BTreeMap::new();
        committed.insert(
            "sess".to_string(),
            CodexSessionMeta {
                path: old_path.display().to_string(),
                size_bytes: metadata.len(),
                mtime_utc: mtime,
                session_timestamp_utc: Some(Utc.with_ymd_and_hms(2026, 3, 7, 18, 0, 0).unwrap()),
                latest_event_timestamp_utc: Some(
                    Utc.with_ymd_and_hms(2026, 3, 7, 18, 0, 1).unwrap(),
                ),
            },
        );

        let sessions = collect_changed_sessions(home.path(), &committed);
        assert_eq!(sessions.len(), 1);
        assert!(sessions[0].rendered.contains("moved"));
    }
}
