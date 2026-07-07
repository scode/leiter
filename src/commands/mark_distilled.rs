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
use chrono::{DateTime, SubsecRound, Utc};

use crate::config::load_config_best_effort;
use crate::paths;
use crate::sync::{SyncHomes, resync_and_warn};
use crate::validation::{ValidationStatus, validate_state};

/// State produced by committing a successful distillation run.
pub struct DistillCommit {
    /// State after pending watermarks have been promoted and saved.
    pub state: crate::state::LeiterState,
    /// Soul body loaded from the same validation pass as the committed state.
    pub soul: String,
    /// The timestamp written to `last_distilled`.
    pub last_distilled: DateTime<Utc>,
}

/// Promote staged distillation watermarks and persist the new cutoff once.
///
/// This is the sole implementation of the distillation commit transition. The
/// lower-level `leiter soul mark-distilled` command and the headless
/// `leiter distill` path both rely on it so the scan-start cutoff, Claude
/// always-on promotion, Codex gate, and atomic save cannot drift apart.
///
/// `expected_scan_started`, when present, is an ownership check for unattended
/// headless runs. A newer distill may have restaged pending watermarks while an
/// older agent was still running; in that case the older run must not promote
/// state it no longer owns.
pub fn commit_staged_distillation(
    state_dir: &Path,
    codex_enabled: bool,
    expected_scan_started: Option<DateTime<Utc>>,
) -> Result<DistillCommit> {
    let (mut state, soul) = match validate_state(state_dir) {
        ValidationStatus::Incompatible(reason) => bail!("{}", reason.agent_message()),
        ValidationStatus::Compatible { state, soul, .. } => (state, soul),
    };

    if let Some(expected) = expected_scan_started
        && state.pending_scan_started_utc != Some(expected)
    {
        bail!("concurrent distill superseded this run; nothing committed");
    }

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
    if codex_enabled && !state.codex.pending.is_empty() {
        let pending = std::mem::take(&mut state.codex.pending);
        state.codex.committed.extend(pending);
    }

    state.save(&paths::state_path(state_dir))?;
    let last_distilled = state.last_distilled;

    Ok(DistillCommit {
        state,
        soul,
        last_distilled,
    })
}

/// Format the confirmation line shared by both distillation commit surfaces.
pub fn confirmation_line(last_distilled: DateTime<Utc>) -> String {
    format!(
        "last_distilled set to {}",
        last_distilled.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    )
}

/// Run the mark-distilled command.
///
/// `claude_home_override`/`codex_home_override` exist only for testability:
/// production callers (`main.rs`) pass `None` so the opportunistic resync
/// below resolves the real default homes through
/// [`crate::paths::resolve_claude_home`]/[`crate::paths::resolve_codex_home`].
/// Neither home has a CLI flag on this command — SPEC.md does not document
/// one — so a non-`None` override only ever comes from a unit test injecting
/// a temp directory in place of the caller's real `~/.claude`/`~/.codex`.
pub fn run(
    state_dir: &Path,
    out: &mut impl Write,
    claude_home_override: Option<&Path>,
    codex_home_override: Option<&Path>,
) -> Result<()> {
    let config = load_config_best_effort(state_dir);
    let DistillCommit {
        mut state,
        soul,
        last_distilled,
    } = commit_staged_distillation(state_dir, config.codex, None)?;

    writeln!(out, "{}", confirmation_line(last_distilled))?;

    if let Some(claude_home) = paths::resolve_claude_home(claude_home_override) {
        let codex_home = paths::resolve_codex_home(codex_home_override, config.codex);
        resync_and_warn(
            out,
            state_dir,
            &mut state,
            &soul,
            SyncHomes {
                claude_home: Some(&claude_home),
                codex_home: codex_home.as_deref(),
            },
            config.codex,
        )?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::test_support::{
        bytes_to_string, setup_state_dir, update_state, write_state_with_epochs,
    };
    use crate::config::LeiterConfig;
    use crate::managed_block::{compose_block, sha256_hex};
    use crate::state::{LeiterState, SessionWatermark, SyncHashes};
    use crate::templates::{SETUP_HARD_EPOCH, SETUP_SOFT_EPOCH};
    use chrono::{SubsecRound, TimeZone, Utc};
    use std::fs;

    fn run_mark_distilled(state_dir: &Path) -> String {
        let mut out = Vec::new();
        run(state_dir, &mut out, None, None).unwrap();
        bytes_to_string(out)
    }

    fn set_codex_enabled(state_dir: &Path, enabled: bool) {
        let config = LeiterConfig {
            codex: enabled,
            ..Default::default()
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
        let result = run(tmp.path(), &mut out, None, None);
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
        let err = run(tmp.path(), &mut out, None, None).unwrap_err();
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
        let err = run(tmp.path(), &mut out, None, None).unwrap_err();
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
        let err = run(tmp.path(), &mut out, None, None).unwrap_err();
        assert!(err.to_string().contains("leiter claude install"));
    }

    /// Regression coverage for home-directory dependency injection: passing an
    /// override makes the opportunistic resync operate on the injected temp
    /// home. There's no direct way to assert an absence of activity in the real
    /// home from a unit test, so this instead proves the healing happened in the
    /// home we handed in — the production code path only reaches a real default
    /// when the override is `None` (see `paths::resolve_claude_home`).
    #[test]
    fn opportunistic_resync_heals_stale_block_in_injected_home() {
        let tmp = setup_state_dir();
        let claude_home = tempfile::tempdir().unwrap();

        // Seed a recorded sync entry that matches a block already on disk
        // (so the clobber guard does not refuse it) but whose soul hash is
        // stale relative to the soul body this test is about to write.
        let soul_path = paths::soul_path(tmp.path());
        let old_soul = fs::read_to_string(&soul_path).unwrap();
        let old_block = compose_block(&soul_path, &old_soul);
        fs::write(paths::claude_md_path(claude_home.path()), &old_block).unwrap();
        update_state(tmp.path(), |state| {
            state.sync.claude_md = Some(SyncHashes {
                soul_hash: sha256_hex(&old_soul),
                block_hash: sha256_hex(&old_block),
            });
        });
        fs::write(&soul_path, "new soul body\n").unwrap();

        let mut out = Vec::new();
        run(tmp.path(), &mut out, Some(claude_home.path()), None).unwrap();

        let healed = fs::read_to_string(paths::claude_md_path(claude_home.path())).unwrap();
        assert!(healed.contains("new soul body"));
    }

    /// Opportunistic re-sync is a backstop for targets leiter already
    /// materialized, not a first install: a target with no recorded sync
    /// hashes must stay untouched even when the command otherwise succeeds.
    #[test]
    fn opportunistic_resync_never_installs_a_never_synced_target() {
        let tmp = tempfile::tempdir().unwrap();
        write_state_with_epochs(tmp.path(), SETUP_SOFT_EPOCH, SETUP_HARD_EPOCH);
        let claude_home = tempfile::tempdir().unwrap();

        let mut out = Vec::new();
        run(tmp.path(), &mut out, Some(claude_home.path()), None).unwrap();

        assert!(!paths::claude_md_path(claude_home.path()).exists());
    }
}
