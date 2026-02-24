//! `leiter stop-hook` — handle the Claude Code Stop event.
//!
//! Reads the Stop hook JSON from stdin, checks `stop_hook_active`, and either
//! blocks the stop with a session-logging prompt or allows it silently. This
//! is the mechanism that ensures every session produces a log before ending.

use std::io::{Read, Write};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::templates::STOP_HOOK_PROMPT_TEMPLATE;

/// Fields we need from the Stop hook JSON input. Extra fields are ignored
/// via `#[serde(deny_unknown_fields)]` being absent.
#[derive(Deserialize)]
struct StopHookInput {
    session_id: String,
    stop_hook_active: bool,
}

/// The blocking decision sent back to Claude Code.
#[derive(Serialize)]
struct BlockDecision {
    decision: &'static str,
    reason: String,
}

/// Run the stop-hook command.
///
/// When `stop_hook_active` is false (first stop of a turn), outputs a JSON
/// blocking decision that prompts the agent to write a session log. When true
/// (the agent was already continued by a stop hook), outputs nothing so the
/// stop proceeds.
pub fn run(input: &mut impl Read, out: &mut impl Write) -> Result<()> {
    let mut raw = String::new();
    input.read_to_string(&mut raw).context("failed to read stdin")?;

    let hook_input: StopHookInput =
        serde_json::from_str(&raw).context("failed to parse stop hook JSON")?;

    if !hook_input.stop_hook_active {
        let reason = STOP_HOOK_PROMPT_TEMPLATE.replace("{session_id}", &hook_input.session_id);
        let decision = BlockDecision {
            decision: "block",
            reason,
        };
        let json = serde_json::to_string(&decision).context("failed to serialize decision")?;
        writeln!(out, "{json}")?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn run_stop_hook(json: &str) -> Result<String> {
        let mut input = Cursor::new(json.as_bytes());
        let mut out = Vec::new();
        run(&mut input, &mut out)?;
        Ok(String::from_utf8(out).unwrap())
    }

    #[test]
    fn inactive_produces_block_decision() {
        let output = run_stop_hook(
            r#"{"session_id":"abc123","stop_hook_active":false}"#,
        )
        .unwrap();

        let parsed: serde_json::Value = serde_json::from_str(output.trim()).unwrap();
        assert_eq!(parsed["decision"], "block");
    }

    #[test]
    fn block_reason_contains_session_id() {
        let output = run_stop_hook(
            r#"{"session_id":"my-sess-42","stop_hook_active":false}"#,
        )
        .unwrap();

        let parsed: serde_json::Value = serde_json::from_str(output.trim()).unwrap();
        let reason = parsed["reason"].as_str().unwrap();
        assert!(reason.contains("my-sess-42"));
    }

    #[test]
    fn block_reason_contains_log_command() {
        let output = run_stop_hook(
            r#"{"session_id":"abc123","stop_hook_active":false}"#,
        )
        .unwrap();

        let parsed: serde_json::Value = serde_json::from_str(output.trim()).unwrap();
        let reason = parsed["reason"].as_str().unwrap();
        assert!(reason.contains("leiter log --session-id"));
    }

    #[test]
    fn active_produces_no_output() {
        let output = run_stop_hook(
            r#"{"session_id":"abc123","stop_hook_active":true}"#,
        )
        .unwrap();

        assert!(output.is_empty());
    }

    #[test]
    fn extra_fields_ignored() {
        let output = run_stop_hook(
            r#"{"session_id":"abc123","stop_hook_active":false,"extra":"stuff","count":42}"#,
        )
        .unwrap();

        let parsed: serde_json::Value = serde_json::from_str(output.trim()).unwrap();
        assert_eq!(parsed["decision"], "block");
    }

    #[test]
    fn missing_session_id_errors() {
        let result = run_stop_hook(r#"{"stop_hook_active":false}"#);
        assert!(result.is_err());
    }

    #[test]
    fn missing_stop_hook_active_errors() {
        let result = run_stop_hook(r#"{"session_id":"abc123"}"#);
        assert!(result.is_err());
    }

    #[test]
    fn invalid_json_errors() {
        let result = run_stop_hook("not json at all");
        assert!(result.is_err());
    }
}
