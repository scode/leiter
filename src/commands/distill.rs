//! `leiter soul distill` — output new session logs for the agent to distill.
//!
//! Reads the `last_distilled` timestamp from the soul frontmatter, scans the
//! logs directory for files with timestamps >= that value, and outputs them
//! chronologically. The inclusive comparison ensures a log written in the same
//! second as the distillation timestamp is not lost.
//!
//! After output, performs best-effort deletion of log files with timestamps
//! strictly before `last_distilled` (already processed by a prior
//! distillation). With `--dry-run`, reports what would be deleted instead.
//!
//! ## JSONL pre-processing
//!
//! Claude Code session transcripts are JSONL files where each line is a JSON
//! object with a `type` field. The vast majority of content is tool machinery
//! invisible to the user (tool results, tool invocations, progress events,
//! thinking blocks, file history snapshots). In observed sessions, user text +
//! assistant text combined are typically 2–15% of the file.
//!
//! We filter each log down to approximately what the user saw:
//!
//! Kept:
//!   - `type: "user"` without `toolUseResult` key (user messages)
//!   - `type: "assistant"` with text blocks (assistant responses)
//!   - Unknown types (fail-useful)
//!   - Non-JSON lines (fail-useful)
//!
//! Dropped:
//!   - `type: "user"` with `toolUseResult` (tool output)
//!   - `type: "assistant"` with only tool_use/thinking blocks
//!   - `type: "progress"`, `"file-history-snapshot"`, `"system"`
//!
//! Uses `serde_json::Value` (not typed structs) to stay resilient to schema
//! changes. If parsing or field access fails, we include the raw line rather
//! than silently dropping it.

use std::fs;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::log_filename::collect_log_entries;
use crate::paths;
use crate::soul_validation::{SoulStatus, validate_soul};
use crate::templates::{DISTILL_DATA_PREAMBLE, SOUL_WRITING_GUIDELINES};

/// Run the distill command.
///
/// Validates the soul file, then outputs all session logs whose filename
/// timestamps are >= `last_distilled` from the soul frontmatter, sorted
/// chronologically. Then deletes obsolete logs (timestamps strictly before
/// `last_distilled`). With `dry_run`, reports what would be deleted instead.
pub fn run(state_dir: &Path, out: &mut impl Write, dry_run: bool) -> Result<()> {
    let logs_dir = paths::logs_dir(state_dir);

    let fm = match validate_soul(state_dir) {
        SoulStatus::Incompatible(reason) => bail!("{}", reason.agent_message()),
        SoulStatus::Compatible { frontmatter, .. } => frontmatter,
    };

    let entries = collect_log_entries(&logs_dir)
        .with_context(|| format!("failed to read logs directory: {}", logs_dir.display()))?;

    let mut logs = Vec::new();
    let mut obsolete = Vec::new();

    for entry in entries {
        if entry.timestamp >= fm.last_distilled {
            logs.push(entry);
        } else {
            obsolete.push(entry);
        }
    }

    if logs.is_empty() {
        writeln!(out, "No new session logs to process.")?;
    } else {
        logs.sort_by_key(|entry| entry.timestamp);

        write!(out, "{SOUL_WRITING_GUIDELINES}")?;
        writeln!(out, "{DISTILL_DATA_PREAMBLE}")?;
        writeln!(out, "<session-transcripts>")?;

        for entry in &logs {
            let content = fs::read_to_string(&entry.path)
                .with_context(|| format!("failed to read log file: {}", entry.path.display()))?;
            let filename = &entry.filename;
            writeln!(out, "<session file=\"{filename}\">")?;
            filter_session_log(&content, out)?;
            writeln!(out, "</session>")?;
        }

        writeln!(out, "</session-transcripts>")?;
    }

    if !obsolete.is_empty() {
        obsolete.sort_by(|a, b| a.filename.cmp(&b.filename));

        if dry_run {
            writeln!(out, "Obsolete logs that would be deleted:")?;
            for entry in &obsolete {
                writeln!(out, "  {}", entry.filename)?;
            }
        } else {
            for entry in &obsolete {
                match fs::remove_file(&entry.path) {
                    Ok(()) => {
                        tracing::debug!("deleted obsolete log: {}", entry.filename);
                    }
                    Err(e) => {
                        tracing::warn!("failed to delete obsolete log {}: {e}", entry.filename);
                    }
                }
            }
        }
    }

    Ok(())
}

/// Extract concatenated text from a `message.content` value that may be either
/// a plain string or an array of content blocks (with `type: "text"` entries).
/// Returns `None` if no text could be extracted.
fn extract_text(content_val: &Value) -> Option<String> {
    if let Some(s) = content_val.as_str() {
        return Some(s.to_string());
    }
    let blocks = content_val.as_array()?;
    let parts: Vec<&str> = blocks
        .iter()
        .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|b| b.get("text").and_then(Value::as_str))
        .collect();
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n\n"))
    }
}

/// Build a one-line summary for a `tool_use` content block.
///
/// Format: `ToolName(key_param)` — the key parameter is chosen heuristically
/// from `input.file_path`, `input.command` (truncated to ~120 chars),
/// `input.pattern`, or omitted if none match.
fn extract_tool_summary(block: &Value) -> Option<String> {
    let name = block.get("name")?.as_str()?;
    let input = block.get("input");

    let param = input.and_then(|inp| {
        if let Some(fp) = inp.get("file_path").and_then(Value::as_str) {
            return Some(fp.to_string());
        }
        if let Some(cmd) = inp.get("command").and_then(Value::as_str) {
            let truncated: String = cmd.chars().take(120).collect();
            if truncated.len() < cmd.len() {
                return Some(format!("{truncated}..."));
            }
            return Some(truncated);
        }
        if let Some(pat) = inp.get("pattern").and_then(Value::as_str) {
            return Some(pat.to_string());
        }
        None
    });

    match param {
        Some(p) => Some(format!("{name}({p})")),
        None => Some(name.to_string()),
    }
}

/// Extract `[assistant tool]:` summary lines from `message.content` blocks.
fn extract_tool_summaries(content_val: &Value, out: &mut impl Write) -> Result<()> {
    if let Some(blocks) = content_val.as_array() {
        for block in blocks {
            if block.get("type").and_then(Value::as_str) == Some("tool_use")
                && let Some(summary) = extract_tool_summary(block)
            {
                writeln!(out, "[assistant tool]: {summary}")?;
            }
        }
    }
    Ok(())
}

/// Pre-process a JSONL session log to extract user-visible content.
///
/// For each line:
/// 1. If JSON parse fails → include raw line (format may have changed).
/// 2. If `type` field is missing or not a string → include raw line.
/// 3. Known noise types ("progress", "file-history-snapshot", "system") → drop.
/// 4. `type: "user"`: drop if `toolUseResult` key exists (tool output);
///    otherwise extract text from `message.content` → emit as `[user]: <text>`.
/// 5. `type: "assistant"`: emit `[assistant]: <text>` for text blocks and
///    `[assistant tool]: Name(param)` for tool_use blocks. Drop only if
///    neither text nor tool_use blocks are present.
/// 6. Unknown type → include raw line (new type we don't know about).
fn filter_session_log(content: &str, out: &mut impl Write) -> Result<()> {
    for line in content.lines() {
        let Ok(val) = serde_json::from_str::<Value>(line) else {
            writeln!(out, "{line}")?;
            continue;
        };

        let Some(obj) = val.as_object() else {
            writeln!(out, "{line}")?;
            continue;
        };

        let Some(type_val) = obj.get("type").and_then(Value::as_str) else {
            writeln!(out, "{line}")?;
            continue;
        };

        match type_val {
            "progress" | "file-history-snapshot" | "system" => continue,

            "user" => {
                if obj.contains_key("toolUseResult") {
                    continue;
                }
                let content_val = obj.get("message").and_then(|m| m.get("content"));
                match content_val.and_then(extract_text) {
                    Some(text) => writeln!(out, "[user]: {text}")?,
                    None => writeln!(out, "{line}")?,
                }
            }

            "assistant" => {
                let content_val = obj.get("message").and_then(|m| m.get("content"));
                let has_text = content_val.and_then(extract_text);
                let has_tools = content_val.and_then(Value::as_array).is_some_and(|blocks| {
                    blocks
                        .iter()
                        .any(|b| b.get("type").and_then(Value::as_str) == Some("tool_use"))
                });

                if has_text.is_none() && !has_tools {
                    continue;
                }

                if let Some(text) = has_text {
                    writeln!(out, "[assistant]: {text}")?;
                }
                if let Some(cv) = content_val {
                    extract_tool_summaries(cv, out)?;
                }
            }

            _ => writeln!(out, "{line}")?,
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::test_support::{bytes_to_string, setup_state_dir};
    use crate::frontmatter::{SoulFrontmatter, parse_soul, serialize_soul};
    use crate::log_filename::generate_log_filename;
    use crate::templates::{SETUP_HARD_EPOCH, SETUP_SOFT_EPOCH};
    use chrono::{TimeZone, Utc};

    fn run_distill(state_dir: &Path) -> String {
        let mut out = Vec::new();
        run(state_dir, &mut out, false).unwrap();
        bytes_to_string(out)
    }

    fn run_distill_dry(state_dir: &Path) -> String {
        let mut out = Vec::new();
        run(state_dir, &mut out, true).unwrap();
        bytes_to_string(out)
    }

    fn write_log(
        state_dir: &Path,
        year: i32,
        month: u32,
        day: u32,
        hour: u32,
        session_id: &str,
        content: &str,
    ) {
        let ts = Utc.with_ymd_and_hms(year, month, day, hour, 0, 0).unwrap();
        let filename = generate_log_filename(ts, session_id);
        let path = paths::logs_dir(state_dir).join(filename);
        fs::write(path, content).unwrap();
    }

    fn set_last_distilled(state_dir: &Path, year: i32, month: u32, day: u32, hour: u32) {
        let ts = Utc.with_ymd_and_hms(year, month, day, hour, 0, 0).unwrap();
        let soul_path = paths::soul_path(state_dir);
        let content = fs::read_to_string(&soul_path).unwrap();
        let (mut fm, body) = parse_soul(&content).unwrap();
        fm.last_distilled = ts;
        fs::write(&soul_path, serialize_soul(&fm, body)).unwrap();
    }

    #[test]
    fn no_logs_at_all() {
        let tmp = setup_state_dir();
        let output = run_distill(tmp.path());
        assert!(output.contains("No new session logs to process"));
    }

    #[test]
    fn all_logs_older_than_last_distilled() {
        let tmp = setup_state_dir();
        write_log(tmp.path(), 2026, 1, 1, 10, "old", "old content");
        set_last_distilled(tmp.path(), 2026, 6, 1, 0);

        let output = run_distill(tmp.path());
        assert!(output.contains("No new session logs to process"));
    }

    #[test]
    fn log_with_timestamp_equal_to_last_distilled_is_included() {
        let tmp = setup_state_dir();
        write_log(tmp.path(), 2026, 6, 1, 0, "exact", "exact content");
        set_last_distilled(tmp.path(), 2026, 6, 1, 0);

        let output = run_distill(tmp.path());
        assert!(output.contains("exact content"));
    }

    #[test]
    fn log_with_timestamp_after_last_distilled_is_included() {
        let tmp = setup_state_dir();
        write_log(tmp.path(), 2026, 7, 1, 0, "new", "new content");
        set_last_distilled(tmp.path(), 2026, 6, 1, 0);

        let output = run_distill(tmp.path());
        assert!(output.contains("new content"));
    }

    #[test]
    fn multiple_logs_in_chronological_order() {
        let tmp = setup_state_dir();
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
        let tmp = setup_state_dir();
        write_log(tmp.path(), 2026, 1, 1, 0, "sess1", "content1");

        let output = run_distill(tmp.path());
        assert!(output.contains("<session file=\"20260101T000000Z-sess1.jsonl\">"));
    }

    #[test]
    fn non_json_content_preserved_verbatim() {
        let tmp = setup_state_dir();
        let original = "line one\n  indented\n\nlast line\n";
        write_log(tmp.path(), 2026, 1, 1, 0, "sess1", original);

        let output = run_distill(tmp.path());
        assert!(output.contains(original));
    }

    #[test]
    fn unparseable_filenames_silently_skipped() {
        let tmp = setup_state_dir();
        write_log(tmp.path(), 2026, 1, 1, 0, "good", "good content");

        let bad_path = paths::logs_dir(tmp.path()).join("not-a-log.txt");
        fs::write(bad_path, "bad").unwrap();

        let output = run_distill(tmp.path());
        assert!(output.contains("good content"));
        assert!(!output.contains("bad"));
    }

    #[test]
    fn output_includes_writing_guidelines() {
        let tmp = setup_state_dir();
        write_log(tmp.path(), 2026, 1, 1, 0, "sess1", "content");

        let output = run_distill(tmp.path());
        assert!(output.contains("Soul-writing guidelines"));
    }

    #[test]
    fn guidelines_appear_before_logs() {
        let tmp = setup_state_dir();
        write_log(tmp.path(), 2026, 1, 1, 0, "sess1", "content");

        let output = run_distill(tmp.path());
        let guidelines_pos = output.find("Soul-writing guidelines").unwrap();
        let log_pos = output
            .find("<session file=\"20260101T000000Z-sess1.jsonl\">")
            .unwrap();
        assert!(guidelines_pos < log_pos);
    }

    #[test]
    fn no_guidelines_when_no_logs() {
        let tmp = setup_state_dir();
        let output = run_distill(tmp.path());
        assert!(!output.contains("Soul-writing guidelines"));
    }

    #[test]
    fn output_includes_data_preamble() {
        let tmp = setup_state_dir();
        write_log(tmp.path(), 2026, 1, 1, 0, "sess1", "content");

        let output = run_distill(tmp.path());
        assert!(output.contains("HISTORICAL DATA"));
        assert!(output.contains("<session-transcripts>"));
    }

    #[test]
    fn data_preamble_between_guidelines_and_sessions() {
        let tmp = setup_state_dir();
        write_log(tmp.path(), 2026, 1, 1, 0, "sess1", "content");

        let output = run_distill(tmp.path());
        let guidelines_pos = output.find("Soul-writing guidelines").unwrap();
        let preamble_pos = output.find("HISTORICAL DATA").unwrap();
        // Find the standalone tag, not the reference inside the preamble text.
        let session_pos = output.find("\n<session-transcripts>\n").unwrap();
        assert!(guidelines_pos < preamble_pos);
        assert!(preamble_pos < session_pos);
    }

    #[test]
    fn sessions_wrapped_in_xml_tags() {
        let tmp = setup_state_dir();
        write_log(tmp.path(), 2026, 1, 1, 0, "s1", "content1");
        write_log(tmp.path(), 2026, 2, 1, 0, "s2", "content2");

        let output = run_distill(tmp.path());
        assert!(output.contains("<session file=\"20260101T000000Z-s1.jsonl\">"));
        assert!(output.contains("<session file=\"20260201T000000Z-s2.jsonl\">"));
        assert!(output.contains("</session>"));
        assert!(output.contains("</session-transcripts>"));
    }

    #[test]
    fn no_xml_tags_when_no_logs() {
        let tmp = setup_state_dir();
        let output = run_distill(tmp.path());
        assert!(!output.contains("<session-transcripts>"));
        assert!(!output.contains("HISTORICAL DATA"));
    }

    #[test]
    fn missing_soul_errors() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(paths::logs_dir(tmp.path())).unwrap();

        let mut out = Vec::new();
        let result = run(tmp.path(), &mut out, false);
        assert!(result.is_err());
    }

    // --- filter_session_log unit tests ---

    fn filter(input: &str) -> String {
        let mut out = Vec::new();
        filter_session_log(input, &mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    fn jsonl_user(text: &str) -> String {
        serde_json::json!({"type": "user", "message": {"content": text}}).to_string()
    }

    fn jsonl_assistant_text(text: &str) -> String {
        serde_json::json!({
            "type": "assistant",
            "message": {"content": [{"type": "text", "text": text}]}
        })
        .to_string()
    }

    fn jsonl_assistant_tool_use() -> String {
        serde_json::json!({
            "type": "assistant",
            "message": {"content": [{"type": "tool_use", "id": "t1", "name": "Read", "input": {}}]}
        })
        .to_string()
    }

    fn jsonl_tool_result() -> String {
        serde_json::json!({
            "type": "user",
            "toolUseResult": {"tool_use_id": "t1"},
            "message": {"content": "file contents here"}
        })
        .to_string()
    }

    fn jsonl_progress() -> String {
        serde_json::json!({"type": "progress", "data": {"type": "agent_progress"}}).to_string()
    }

    #[test]
    fn filter_extracts_user_text() {
        let output = filter(&jsonl_user("hello world"));
        assert_eq!(output, "[user]: hello world\n");
    }

    #[test]
    fn filter_extracts_assistant_text() {
        let output = filter(&jsonl_assistant_text("here is my response"));
        assert_eq!(output, "[assistant]: here is my response\n");
    }

    #[test]
    fn filter_concatenates_multiple_text_blocks() {
        let line = serde_json::json!({
            "type": "assistant",
            "message": {"content": [
                {"type": "text", "text": "first part"},
                {"type": "tool_use", "id": "t1", "name": "Read", "input": {}},
                {"type": "text", "text": "second part"}
            ]}
        })
        .to_string();
        let output = filter(&line);
        assert_eq!(
            output,
            "[assistant]: first part\n\nsecond part\n[assistant tool]: Read\n"
        );
    }

    #[test]
    fn filter_drops_tool_results() {
        let output = filter(&jsonl_tool_result());
        assert_eq!(output, "");
    }

    #[test]
    fn filter_emits_tool_summary_for_tool_use_only_assistant() {
        let output = filter(&jsonl_assistant_tool_use());
        assert_eq!(output, "[assistant tool]: Read\n");
    }

    #[test]
    fn filter_drops_progress() {
        let output = filter(&jsonl_progress());
        assert_eq!(output, "");
    }

    #[test]
    fn filter_drops_system() {
        let line = serde_json::json!({"type": "system", "event": "init"}).to_string();
        assert_eq!(filter(&line), "");
    }

    #[test]
    fn filter_drops_file_history_snapshot() {
        let line = serde_json::json!({"type": "file-history-snapshot", "files": []}).to_string();
        assert_eq!(filter(&line), "");
    }

    #[test]
    fn filter_includes_unknown_type_as_raw() {
        let line = serde_json::json!({"type": "new_future_type", "data": 42}).to_string();
        let output = filter(&line);
        assert_eq!(output, format!("{line}\n"));
    }

    #[test]
    fn filter_includes_non_json_as_raw() {
        let output = filter("this is not json at all");
        assert_eq!(output, "this is not json at all\n");
    }

    #[test]
    fn filter_includes_json_without_type_as_raw() {
        let line = serde_json::json!({"foo": "bar"}).to_string();
        let output = filter(&line);
        assert_eq!(output, format!("{line}\n"));
    }

    #[test]
    fn filter_preserves_blank_lines() {
        let input = format!("{}\n\n{}", jsonl_user("hi"), jsonl_user("bye"));
        let output = filter(&input);
        assert_eq!(output, "[user]: hi\n\n[user]: bye\n");
    }

    #[test]
    fn filter_mixed_session() {
        let lines = [
            jsonl_user("help me with rust"),
            jsonl_assistant_text("Sure, I can help."),
            jsonl_assistant_tool_use(),
            jsonl_tool_result(),
            jsonl_progress(),
            jsonl_assistant_text("Here is the result."),
            jsonl_user("thanks"),
        ];
        let input = lines.join("\n");
        let output = filter(&input);
        assert_eq!(
            output,
            "[user]: help me with rust\n\
             [assistant]: Sure, I can help.\n\
             [assistant tool]: Read\n\
             [assistant]: Here is the result.\n\
             [user]: thanks\n"
        );
    }

    #[test]
    fn filter_drops_thinking_only_assistant() {
        let line = serde_json::json!({
            "type": "assistant",
            "message": {"content": [{"type": "thinking", "thinking": "let me think..."}]}
        })
        .to_string();
        assert_eq!(filter(&line), "");
    }

    #[test]
    fn filter_tool_summary_with_file_path() {
        let line = serde_json::json!({
            "type": "assistant",
            "message": {"content": [
                {"type": "tool_use", "id": "t1", "name": "Edit", "input": {"file_path": "src/main.rs"}}
            ]}
        })
        .to_string();
        assert_eq!(filter(&line), "[assistant tool]: Edit(src/main.rs)\n");
    }

    #[test]
    fn filter_tool_summary_with_command() {
        let line = serde_json::json!({
            "type": "assistant",
            "message": {"content": [
                {"type": "tool_use", "id": "t1", "name": "Bash", "input": {"command": "cargo test"}}
            ]}
        })
        .to_string();
        assert_eq!(filter(&line), "[assistant tool]: Bash(cargo test)\n");
    }

    #[test]
    fn filter_tool_summary_with_pattern() {
        let line = serde_json::json!({
            "type": "assistant",
            "message": {"content": [
                {"type": "tool_use", "id": "t1", "name": "Grep", "input": {"pattern": "fn main"}}
            ]}
        })
        .to_string();
        assert_eq!(filter(&line), "[assistant tool]: Grep(fn main)\n");
    }

    #[test]
    fn filter_tool_summary_no_key_param() {
        let line = serde_json::json!({
            "type": "assistant",
            "message": {"content": [
                {"type": "tool_use", "id": "t1", "name": "Agent", "input": {"prompt": "explore"}}
            ]}
        })
        .to_string();
        assert_eq!(filter(&line), "[assistant tool]: Agent\n");
    }

    #[test]
    fn filter_tool_summary_command_truncated() {
        let long_cmd = "x".repeat(200);
        let line = serde_json::json!({
            "type": "assistant",
            "message": {"content": [
                {"type": "tool_use", "id": "t1", "name": "Bash", "input": {"command": long_cmd}}
            ]}
        })
        .to_string();
        let output = filter(&line);
        let expected_truncated: String = "x".repeat(120);
        assert_eq!(
            output,
            format!("[assistant tool]: Bash({expected_truncated}...)\n")
        );
    }

    #[test]
    fn filter_tool_summary_multiple_tools() {
        let line = serde_json::json!({
            "type": "assistant",
            "message": {"content": [
                {"type": "tool_use", "id": "t1", "name": "Read", "input": {"file_path": "a.rs"}},
                {"type": "tool_use", "id": "t2", "name": "Bash", "input": {"command": "ls"}}
            ]}
        })
        .to_string();
        assert_eq!(
            filter(&line),
            "[assistant tool]: Read(a.rs)\n[assistant tool]: Bash(ls)\n"
        );
    }

    #[test]
    fn filter_tool_summary_file_path_takes_priority_over_command() {
        let line = serde_json::json!({
            "type": "assistant",
            "message": {"content": [
                {"type": "tool_use", "id": "t1", "name": "Edit", "input": {
                    "file_path": "src/lib.rs",
                    "command": "ignored"
                }}
            ]}
        })
        .to_string();
        assert_eq!(filter(&line), "[assistant tool]: Edit(src/lib.rs)\n");
    }

    #[test]
    fn filter_extracts_user_text_from_array_content() {
        let line = serde_json::json!({
            "type": "user",
            "message": {"content": [{"type": "text", "text": "hello from array"}]}
        })
        .to_string();
        assert_eq!(filter(&line), "[user]: hello from array\n");
    }

    // --- obsolete log cleanup tests ---

    #[test]
    fn obsolete_logs_deleted() {
        let tmp = setup_state_dir();
        write_log(tmp.path(), 2026, 1, 1, 10, "old", "old content");
        write_log(tmp.path(), 2026, 7, 1, 0, "new", "new content");
        set_last_distilled(tmp.path(), 2026, 6, 1, 0);

        let output = run_distill(tmp.path());
        assert!(output.contains("new content"));
        assert!(!output.contains("old content"));

        let remaining: Vec<_> = fs::read_dir(paths::logs_dir(tmp.path()))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(remaining.len(), 1);
        assert!(remaining[0].contains("new"));
    }

    #[test]
    fn obsolete_logs_dry_run_does_not_delete() {
        let tmp = setup_state_dir();
        write_log(tmp.path(), 2026, 1, 1, 10, "old", "old content");
        write_log(tmp.path(), 2026, 7, 1, 0, "new", "new content");
        set_last_distilled(tmp.path(), 2026, 6, 1, 0);

        let output = run_distill_dry(tmp.path());
        assert!(output.contains("would be deleted"));
        assert!(output.contains("old"));

        let remaining: Vec<_> = fs::read_dir(paths::logs_dir(tmp.path())).unwrap().collect();
        assert_eq!(remaining.len(), 2);
    }

    #[test]
    fn no_obsolete_logs_no_cleanup_output() {
        let tmp = setup_state_dir();
        write_log(tmp.path(), 2026, 7, 1, 0, "new", "new content");
        set_last_distilled(tmp.path(), 2026, 6, 1, 0);

        let output = run_distill(tmp.path());
        assert!(!output.contains("deleted"));
        assert!(!output.contains("would be deleted"));
        assert!(!output.contains("Obsolete"));
    }

    #[test]
    fn all_obsolete_no_new_logs() {
        let tmp = setup_state_dir();
        write_log(tmp.path(), 2026, 1, 1, 10, "old", "old content");
        set_last_distilled(tmp.path(), 2026, 6, 1, 0);

        let output = run_distill(tmp.path());
        assert!(output.contains("No new session logs to process"));

        let remaining: Vec<_> = fs::read_dir(paths::logs_dir(tmp.path())).unwrap().collect();
        assert_eq!(remaining.len(), 0);
    }

    #[test]
    fn unparseable_filenames_not_deleted() {
        let tmp = setup_state_dir();
        let bad_path = paths::logs_dir(tmp.path()).join("not-a-log.txt");
        fs::write(&bad_path, "bad").unwrap();
        set_last_distilled(tmp.path(), 2026, 6, 1, 0);

        run_distill(tmp.path());
        assert!(bad_path.exists(), "unparseable file must not be deleted");
    }

    #[test]
    fn log_at_last_distilled_not_deleted() {
        let tmp = setup_state_dir();
        write_log(tmp.path(), 2026, 6, 1, 0, "exact", "exact content");
        set_last_distilled(tmp.path(), 2026, 6, 1, 0);

        let output = run_distill(tmp.path());
        assert!(output.contains("exact content"));

        let remaining: Vec<_> = fs::read_dir(paths::logs_dir(tmp.path()))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(remaining.len(), 1);
        assert!(remaining[0].contains("exact"));
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
    fn soft_epoch_mismatch_succeeds() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(paths::logs_dir(tmp.path())).unwrap();
        write_soul_with_epochs(tmp.path(), SETUP_SOFT_EPOCH + 1, SETUP_HARD_EPOCH);
        write_log(tmp.path(), 2026, 1, 1, 0, "sess1", "log content");

        let mut out = Vec::new();
        run(tmp.path(), &mut out, false).unwrap();
        let output = bytes_to_string(out);
        assert!(output.contains("log content"));
    }

    #[test]
    fn hard_epoch_mismatch_new_soul_errors() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(paths::logs_dir(tmp.path())).unwrap();
        write_soul_with_epochs(tmp.path(), SETUP_SOFT_EPOCH, SETUP_HARD_EPOCH + 1);

        let mut out = Vec::new();
        let err = run(tmp.path(), &mut out, false).unwrap_err();
        assert!(
            err.to_string()
                .contains("binary is older than your soul file")
        );
    }

    #[test]
    fn hard_epoch_mismatch_old_soul_errors() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(paths::logs_dir(tmp.path())).unwrap();
        write_soul_with_epochs(
            tmp.path(),
            SETUP_SOFT_EPOCH,
            SETUP_HARD_EPOCH.saturating_sub(1),
        );

        let mut out = Vec::new();
        let err = run(tmp.path(), &mut out, false).unwrap_err();
        assert!(err.to_string().contains("leiter claude install"));
    }

    #[test]
    fn corrupt_frontmatter_errors() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(paths::logs_dir(tmp.path())).unwrap();
        fs::write(paths::soul_path(tmp.path()), "not frontmatter").unwrap();

        let mut out = Vec::new();
        let result = run(tmp.path(), &mut out, false);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("invalid YAML"));
    }
}
