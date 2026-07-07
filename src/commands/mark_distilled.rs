//! `leiter soul mark-distilled` — commit the last distill run's cutoff.
//!
//! Deterministically updates `state.toml` so the agent never has to edit
//! `last_distilled` by hand. The same atomic write always promotes pending
//! Claude watermarks and, when Codex distillation is enabled, pending Codex
//! watermarks.
//!
//! `last_distilled` advances to the scan-start time the distill run staged,
//! not to the mark-time wall clock. Sessions born between the scan and the
//! mark would otherwise sit below the external scan's floor forever — no
//! committed watermark, mtime older than a mark-time cutoff — and the same
//! window would silently discard hook-copied logs as obsolete.

use std::io::Write;
use std::path::Path;

use anyhow::{Result, bail};
use chrono::{SubsecRound, Utc};
use tracing::warn;

use crate::config::LeiterConfig;
use crate::paths;
use crate::validation::{ValidationStatus, validate_state};

pub fn run(state_dir: &Path, out: &mut impl Write) -> Result<()> {
    let mut state = match validate_state(state_dir) {
        ValidationStatus::Incompatible(reason) => bail!("{}", reason.agent_message()),
        ValidationStatus::Compatible { state, .. } => state,
    };

    let config = load_config_best_effort(state_dir);
    if !state.claude.pending.is_empty() {
        let pending = std::mem::take(&mut state.claude.pending);
        state.claude.committed.extend(pending);
    }
    // Fall back to the wall clock only when nothing was staged (a mark run
    // without a preceding non-dry-run distill).
    state.last_distilled = state
        .pending_scan_started_utc
        .take()
        .unwrap_or_else(|| Utc::now().trunc_subsecs(0));
    if config.enable_codex_experimental && !state.codex.pending.is_empty() {
        let pending = std::mem::take(&mut state.codex.pending);
        state.codex.committed.extend(pending);
    }

    state.save(&paths::state_path(state_dir))?;

    writeln!(
        out,
        "last_distilled set to {}",
        state
            .last_distilled
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    )?;

    Ok(())
}

fn load_config_best_effort(state_dir: &Path) -> LeiterConfig {
    let config_path = paths::leiter_config_path(state_dir);
    match LeiterConfig::load(&config_path) {
        Ok(config) => config,
        Err(err) => {
            warn!("failed to load leiter config, using defaults: {err}");
            LeiterConfig::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::test_support::{
        bytes_to_string, setup_state_dir, write_state_with_epochs,
    };
    use crate::config::LeiterConfig;
    use crate::state::{LeiterState, SessionWatermark};
    use crate::templates::{SETUP_HARD_EPOCH, SETUP_SOFT_EPOCH};
    use chrono::{SubsecRound, TimeZone, Utc};
    use std::fs;

    fn run_mark_distilled(state_dir: &Path) -> String {
        let mut out = Vec::new();
        run(state_dir, &mut out).unwrap();
        bytes_to_string(out)
    }

    fn set_codex_enabled(state_dir: &Path, enabled: bool) {
        let config = LeiterConfig {
            enable_codex_experimental: enabled,
        };
        config.save(&paths::leiter_config_path(state_dir)).unwrap();
    }

    #[test]
    fn sets_last_distilled_to_approximately_now() {
        let tmp = setup_state_dir();
        let before = Utc::now().trunc_subsecs(0);
        run_mark_distilled(tmp.path());
        let after = Utc::now();

        let state = LeiterState::load(&paths::state_path(tmp.path())).unwrap();
        assert!(state.last_distilled >= before);
        assert!(state.last_distilled <= after);
    }

    #[test]
    fn preserves_soul_body_bytes() {
        let tmp = setup_state_dir();
        let soul_path = paths::soul_path(tmp.path());

        let original = fs::read_to_string(&soul_path).unwrap();
        run_mark_distilled(tmp.path());

        let updated = fs::read_to_string(&soul_path).unwrap();
        assert_eq!(updated, original);
    }

    #[test]
    fn preserves_other_state_fields() {
        let tmp = setup_state_dir();
        let original = LeiterState::load(&paths::state_path(tmp.path())).unwrap();

        run_mark_distilled(tmp.path());

        let updated = LeiterState::load(&paths::state_path(tmp.path())).unwrap();
        assert_eq!(updated.soul_version, original.soul_version);
        assert_eq!(updated.setup_soft_epoch, original.setup_soft_epoch);
        assert_eq!(updated.setup_hard_epoch, original.setup_hard_epoch);
    }

    #[test]
    fn missing_soul_file_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let mut out = Vec::new();
        let result = run(tmp.path(), &mut out);
        assert!(result.is_err());
    }

    #[test]
    fn outputs_confirmation_with_timestamp() {
        let tmp = setup_state_dir();
        let output = run_mark_distilled(tmp.path());
        assert!(output.starts_with("last_distilled set to "));
    }

    #[test]
    fn confirmation_matches_stored_value() {
        let tmp = setup_state_dir();
        let output = run_mark_distilled(tmp.path());
        let displayed_ts = output
            .trim()
            .strip_prefix("last_distilled set to ")
            .unwrap();

        let state = LeiterState::load(&paths::state_path(tmp.path())).unwrap();
        let stored_ts = state
            .last_distilled
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        assert_eq!(displayed_ts, stored_ts);
    }

    #[test]
    fn corrupt_state_errors() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(paths::soul_path(tmp.path()), "body\n").unwrap();
        fs::write(paths::state_path(tmp.path()), "not = valid = toml").unwrap();

        let mut out = Vec::new();
        let err = run(tmp.path(), &mut out).unwrap_err();
        assert!(err.to_string().contains("state file"));
    }

    /// `mark-distilled` is the commit point for staged Codex watermarks: once
    /// the user-visible distill flow succeeds, pending session snapshots move
    /// into committed so unchanged Codex sessions are skipped next time.
    #[test]
    fn promotes_pending_codex_metadata() {
        let tmp = setup_state_dir();
        set_codex_enabled(tmp.path(), true);
        let ts = Utc.with_ymd_and_hms(2026, 3, 7, 18, 0, 0).unwrap();

        let mut state = LeiterState::load(&paths::state_path(tmp.path())).unwrap();
        state.codex.pending.insert(
            "sess".to_string(),
            SessionWatermark {
                path: "/tmp/session.jsonl".to_string(),
                size_bytes: 99,
                mtime_utc: ts,
                session_timestamp_utc: Some(ts),
                latest_event_timestamp_utc: Some(ts),
            },
        );
        state.save(&paths::state_path(tmp.path())).unwrap();

        run_mark_distilled(tmp.path());

        let updated = LeiterState::load(&paths::state_path(tmp.path())).unwrap();
        assert!(updated.codex.pending.is_empty());
        assert_eq!(updated.codex.committed.len(), 1);
        assert!(updated.codex.committed.contains_key("sess"));
    }

    #[test]
    fn codex_metadata_untouched_when_experimental_gate_is_disabled() {
        let tmp = setup_state_dir();
        let ts = Utc.with_ymd_and_hms(2026, 3, 7, 18, 0, 0).unwrap();

        let mut state = LeiterState::load(&paths::state_path(tmp.path())).unwrap();
        state.codex.pending.insert(
            "sess".to_string(),
            SessionWatermark {
                path: "/tmp/session.jsonl".to_string(),
                size_bytes: 99,
                mtime_utc: ts,
                session_timestamp_utc: Some(ts),
                latest_event_timestamp_utc: Some(ts),
            },
        );
        state.save(&paths::state_path(tmp.path())).unwrap();

        run_mark_distilled(tmp.path());

        let updated = LeiterState::load(&paths::state_path(tmp.path())).unwrap();
        assert_eq!(updated.codex.pending.len(), 1);
        assert!(updated.codex.committed.is_empty());
    }

    #[test]
    fn promotes_pending_claude_metadata_when_codex_gate_is_disabled() {
        let tmp = setup_state_dir();
        let ts = Utc.with_ymd_and_hms(2026, 3, 7, 18, 0, 0).unwrap();

        let mut state = LeiterState::load(&paths::state_path(tmp.path())).unwrap();
        state.claude.pending.insert(
            "claude-sess".to_string(),
            SessionWatermark {
                path: "/tmp/claude.jsonl".to_string(),
                size_bytes: 99,
                mtime_utc: ts,
                session_timestamp_utc: Some(ts),
                latest_event_timestamp_utc: None,
            },
        );
        state.codex.pending.insert(
            "codex-sess".to_string(),
            SessionWatermark {
                path: "/tmp/codex.jsonl".to_string(),
                size_bytes: 100,
                mtime_utc: ts,
                session_timestamp_utc: Some(ts),
                latest_event_timestamp_utc: Some(ts),
            },
        );
        state.save(&paths::state_path(tmp.path())).unwrap();

        run_mark_distilled(tmp.path());

        let updated = LeiterState::load(&paths::state_path(tmp.path())).unwrap();
        assert!(updated.claude.pending.is_empty());
        assert!(updated.claude.committed.contains_key("claude-sess"));
        assert_eq!(updated.codex.pending.len(), 1);
        assert!(updated.codex.committed.is_empty());
    }

    #[test]
    fn hard_epoch_mismatch_new_state_errors() {
        let tmp = tempfile::tempdir().unwrap();
        write_state_with_epochs(tmp.path(), SETUP_SOFT_EPOCH, SETUP_HARD_EPOCH + 1);

        let mut out = Vec::new();
        let err = run(tmp.path(), &mut out).unwrap_err();
        assert!(
            err.to_string()
                .contains("binary is older than your soul file")
        );
    }

    #[test]
    fn hard_epoch_mismatch_old_state_errors() {
        let tmp = tempfile::tempdir().unwrap();
        write_state_with_epochs(
            tmp.path(),
            SETUP_SOFT_EPOCH,
            SETUP_HARD_EPOCH.saturating_sub(1),
        );

        let mut out = Vec::new();
        let err = run(tmp.path(), &mut out).unwrap_err();
        assert!(err.to_string().contains("leiter claude install"));
    }
}
