//! Claude transcript canonicalization shared by legacy logs and external scans.
//!
//! Claude Code writes the same JSONL transcript shape in both places leiter
//! currently reads from: hook-copied files under `<state_dir>/logs/` and live
//! files under `<claude_home>/projects/`. Keeping the filter here makes that
//! equivalence explicit. Unknown records and malformed lines fail useful by
//! passing through verbatim; known assistant records with no visible text and
//! no `tool_use` blocks are dropped because they currently carry only
//! machine-facing transcript noise.

use std::io::Write;

use anyhow::Result;
use serde_json::Value;

/// Render a Claude JSONL transcript into the stable text form used for soul distillation.
///
/// This is the in-memory companion to [`filter_session_log`]. Callers that need
/// to decide whether a session has any visible content use this function; it
/// preserves the same trailing-newline behavior as the writer path.
pub(crate) fn render_session_log(content: &str) -> Result<String> {
    let mut rendered = Vec::new();
    filter_session_log(content, &mut rendered)?;
    Ok(String::from_utf8(rendered).expect("canonicalized transcript output is always UTF-8"))
}

/// Pre-process a JSONL session log to extract user-visible content.
///
/// The filter deliberately favors retaining unfamiliar data over losing user
/// signal. Known machine-only records are dropped. Unknown record types and
/// malformed JSON are emitted verbatim so a future Claude transcript change is
/// visible in distill output instead of silently disappearing. Assistant
/// records are the deliberate exception: if an assistant message has neither
/// extractable text nor `tool_use` blocks, it is treated as machine-facing
/// noise and skipped.
pub(crate) fn filter_session_log(content: &str, out: &mut impl Write) -> Result<()> {
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

/// Extract visible message text from Claude's string-or-block content field.
///
/// Claude has used both plain strings and arrays of typed blocks for message
/// content. This helper keeps the caller focused on transcript roles while the
/// schema-specific block filtering stays in one place.
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

/// Build a one-line summary for a Claude `tool_use` content block.
///
/// Distillation needs to know that the assistant used a tool and roughly what
/// it targeted, not the full tool payload. The parameter priority mirrors the
/// historical filter behavior so old hook-copied logs and external sessions
/// render byte-for-byte the same.
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

/// Emit tool summaries from an assistant message's content blocks.
///
/// This function writes directly because it is part of the streaming
/// canonicalizer. It intentionally ignores non-`tool_use` blocks; visible text
/// is handled separately by [`extract_text`].
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
