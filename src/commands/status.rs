//! `leiter status` — read-only distillation and delivery health report.
//!
//! Status intentionally does not repair anything. It uses the same scan logic
//! as distillation in dry-run mode, and the same managed-block hash semantics
//! as sync, but it never stages watermarks, deletes logs, creates blocks, or
//! adopts hashes.

use std::io::Write;
use std::path::Path;

use anyhow::{Result, bail};

use crate::commands::distill::{gather, retention_risk_sessions};
use crate::config::load_config_best_effort;
use crate::paths;
use crate::sync::{SyncTarget, inspect_target};
use crate::templates::SETUP_SOFT_EPOCH;
use crate::validation::{ValidationStatus, validate_state};

/// Run the top-level `leiter status` command against the default homes.
pub fn run(state_dir: &Path, out: &mut impl Write) -> Result<()> {
    if let ValidationStatus::Incompatible(reason) = validate_state(state_dir) {
        bail!("{}", reason.user_message());
    }
    let config = load_config_best_effort(state_dir);
    let claude_home = paths::resolve_claude_home(None);
    let codex_home = paths::resolve_codex_home(None, config.codex);
    run_with_homes(
        state_dir,
        out,
        claude_home.as_deref(),
        codex_home.as_deref(),
    )
}

/// Run status with already-resolved homes.
///
/// This keeps the CLI contract simple while letting tests inspect temp managed
/// block targets instead of the caller's real home directories.
pub fn run_with_homes(
    state_dir: &Path,
    out: &mut impl Write,
    claude_home: Option<&Path>,
    codex_home: Option<&Path>,
) -> Result<()> {
    let (state, soul) = match validate_state(state_dir) {
        ValidationStatus::Compatible { state, soul, .. } => (state, soul),
        ValidationStatus::Incompatible(reason) => bail!("{}", reason.user_message()),
    };
    let config = load_config_best_effort(state_dir);

    let gathered = match gather(
        state_dir,
        claude_home,
        codex_home,
        config.codex,
        true,
        false,
    ) {
        Ok(gathered) => Some(gathered),
        Err(err) => {
            writeln!(out, "Warning: could not scan: {err:#}")?;
            None
        }
    };

    writeln!(
        out,
        "Claude undistilled sessions: {}",
        gathered
            .as_ref()
            .map_or(0, |gathered| gathered.claude_count)
    )?;
    if config.codex {
        writeln!(
            out,
            "Codex undistilled sessions: {}",
            gathered.as_ref().map_or(0, |gathered| gathered.codex_count)
        )?;
    }

    if let Some(claude_home) = claude_home {
        let inspection = inspect_target(
            state_dir,
            &soul,
            SyncTarget::ClaudeMd,
            &paths::claude_md_path(claude_home),
            state.sync.claude_md.as_ref(),
        );
        writeln!(out, "{}", inspection.line())?;
    } else {
        writeln!(out, "CLAUDE.md: never synced (Claude home unavailable)")?;
    }

    if config.codex {
        if let Some(codex_home) = codex_home {
            let inspection = inspect_target(
                state_dir,
                &soul,
                SyncTarget::AgentsMd,
                &paths::agents_md_path(codex_home),
                state.sync.agents_md.as_ref(),
            );
            writeln!(out, "{}", inspection.line())?;
        } else {
            writeln!(out, "AGENTS.md: never synced (Codex home unavailable)")?;
        }
    }

    if state.setup_soft_epoch < SETUP_SOFT_EPOCH {
        writeln!(
            out,
            "Setup advisory: leiter setup has optional updates available; run `leiter claude install` when convenient."
        )?;
    } else if state.setup_soft_epoch > SETUP_SOFT_EPOCH {
        writeln!(
            out,
            "Setup advisory: this leiter binary is older than the installed setup expects; upgrade leiter when convenient."
        )?;
    }

    if let Some(gathered) = &gathered {
        let at_risk = retention_risk_sessions(
            &gathered.external_claude,
            config.retention_warn_days,
            chrono::Utc::now(),
        );
        if !at_risk.is_empty() {
            writeln!(
                out,
                "Retention warning: undistilled Claude sessions older than {} days:",
                config.retention_warn_days
            )?;
            for session in at_risk {
                writeln!(out, "  {}", session.file_label)?;
            }
        }
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
    use crate::log_filename::generate_log_filename;
    use crate::managed_block::{compose_block, sha256_hex};
    use crate::state::SyncHashes;
    use crate::templates::{SETUP_HARD_EPOCH, SETUP_SOFT_EPOCH};
    use chrono::{TimeZone, Utc};
    use std::fs;
    use std::path::{Path, PathBuf};

    const SESSION_ID: &str = "22222222-2222-4222-8222-222222222222";

    fn write_claude_session(claude_home: &Path, session_id: &str, text: &str) -> PathBuf {
        let path = claude_home
            .join("projects")
            .join("-tmp-proj")
            .join(format!("{session_id}.jsonl"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            format!(
                "{{\"timestamp\":\"2026-07-01T12:00:00Z\",\"type\":\"user\",\"message\":{{\"content\":\"{text}\"}}}}\n"
            ),
        )
        .unwrap();
        path
    }

    fn write_codex_session(codex_home: &Path, session_id: &str, text: &str) {
        let path = codex_home.join("sessions").join("rollout.jsonl");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            path,
            format!(
                "{{\"timestamp\":\"2026-07-01T12:00:00Z\",\"type\":\"session_meta\",\"payload\":{{\"id\":\"{session_id}\",\"timestamp\":\"2026-07-01T12:00:00Z\"}}}}\n\
                 {{\"timestamp\":\"2026-07-01T12:00:01Z\",\"type\":\"response_item\",\"payload\":{{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{{\"type\":\"output_text\",\"text\":\"{text}\"}}]}}}}\n"
            ),
        )
        .unwrap();
    }

    fn run_status_capture(state_dir: &Path, claude_home: &Path, codex_home: &Path) -> String {
        let mut out = Vec::new();
        run_with_homes(state_dir, &mut out, Some(claude_home), Some(codex_home)).unwrap();
        bytes_to_string(out)
    }

    #[test]
    fn reports_counts_after_external_legacy_dedupe_and_codex_scan() {
        let tmp = setup_state_dir();
        let codex_home = tempfile::tempdir().unwrap();
        LeiterConfig {
            codex: true,
            ..Default::default()
        }
        .save(&paths::leiter_config_path(tmp.path()))
        .unwrap();

        write_claude_session(tmp.claude.path(), SESSION_ID, "external preference");
        let duplicate = generate_log_filename(
            Utc.with_ymd_and_hms(2026, 7, 1, 12, 0, 0).unwrap(),
            SESSION_ID,
        );
        fs::create_dir_all(paths::logs_dir(tmp.path())).unwrap();
        fs::write(
            paths::logs_dir(tmp.path()).join(duplicate),
            "duplicate legacy",
        )
        .unwrap();
        let legacy = generate_log_filename(
            Utc.with_ymd_and_hms(2026, 7, 1, 13, 0, 0).unwrap(),
            "legacy-only",
        );
        fs::write(paths::logs_dir(tmp.path()).join(legacy), "legacy only").unwrap();
        write_codex_session(codex_home.path(), "codex-session", "codex preference");

        let before = fs::read(paths::state_path(tmp.path())).unwrap();
        let output = run_status_capture(tmp.path(), tmp.claude.path(), codex_home.path());

        assert!(output.contains("Claude undistilled sessions: 2"));
        assert!(output.contains("Codex undistilled sessions: 1"));
        assert_eq!(fs::read(paths::state_path(tmp.path())).unwrap(), before);
    }

    #[test]
    fn never_synced_status_does_not_create_blocks_or_write_state() {
        let tmp = tempfile::tempdir().unwrap();
        let claude_home = tempfile::tempdir().unwrap();
        let codex_home = tempfile::tempdir().unwrap();
        write_state_with_epochs(tmp.path(), SETUP_SOFT_EPOCH, SETUP_HARD_EPOCH);
        fs::create_dir_all(paths::logs_dir(tmp.path())).unwrap();

        let before = fs::read(paths::state_path(tmp.path())).unwrap();
        let output = run_status_capture(tmp.path(), claude_home.path(), codex_home.path());

        assert!(output.contains("CLAUDE.md: never synced"));
        assert!(!paths::claude_md_path(claude_home.path()).exists());
        assert!(!paths::agents_md_path(codex_home.path()).exists());
        assert_eq!(fs::read(paths::state_path(tmp.path())).unwrap(), before);
    }

    #[test]
    fn reports_in_sync_block() {
        let tmp = setup_state_dir();
        let codex_home = tempfile::tempdir().unwrap();

        let output = run_status_capture(tmp.path(), tmp.claude.path(), codex_home.path());

        assert!(output.contains("CLAUDE.md: in sync"));
    }

    #[test]
    fn soft_epoch_behind_reports_install_advisory() {
        let tmp = setup_state_dir();
        let codex_home = tempfile::tempdir().unwrap();
        update_state(tmp.path(), |state| {
            state.setup_soft_epoch = SETUP_SOFT_EPOCH.saturating_sub(1);
        });

        let output = run_status_capture(tmp.path(), tmp.claude.path(), codex_home.path());

        assert!(output.contains("Setup advisory:"));
        assert!(output.contains("leiter claude install"));
    }

    #[test]
    fn soft_epoch_ahead_reports_binary_upgrade_advisory() {
        let tmp = setup_state_dir();
        let codex_home = tempfile::tempdir().unwrap();
        update_state(tmp.path(), |state| {
            state.setup_soft_epoch = SETUP_SOFT_EPOCH + 1;
        });

        let output = run_status_capture(tmp.path(), tmp.claude.path(), codex_home.path());

        assert!(output.contains("Setup advisory:"));
        assert!(output.contains("upgrade leiter"));
    }

    #[test]
    fn matching_soft_epoch_omits_setup_advisory() {
        let tmp = setup_state_dir();
        let codex_home = tempfile::tempdir().unwrap();

        let output = run_status_capture(tmp.path(), tmp.claude.path(), codex_home.path());

        assert!(!output.contains("Setup advisory:"));
    }

    #[test]
    fn reports_stale_and_hand_edited_blocks() {
        let tmp = setup_state_dir();
        let codex_home = tempfile::tempdir().unwrap();

        fs::write(paths::soul_path(tmp.path()), "changed soul\n").unwrap();
        let stale = run_status_capture(tmp.path(), tmp.claude.path(), codex_home.path());
        assert!(stale.contains("CLAUDE.md: stale"));

        let original_soul = "old soul\n";
        let soul_path = paths::soul_path(tmp.path());
        let original_block = compose_block(&soul_path, original_soul);
        fs::write(
            paths::claude_md_path(tmp.claude.path()),
            original_block.replace("old", "hand"),
        )
        .unwrap();
        update_state(tmp.path(), |state| {
            state.sync.claude_md = Some(SyncHashes {
                soul_hash: sha256_hex(original_soul),
                block_hash: sha256_hex(&original_block),
            });
        });
        fs::write(paths::soul_path(tmp.path()), "changed soul\n").unwrap();

        let hand_edited = run_status_capture(tmp.path(), tmp.claude.path(), codex_home.path());
        assert!(hand_edited.contains("CLAUDE.md: hand-edited"));
    }

    #[test]
    fn reports_unreadable_block_separately_from_hand_edit() {
        let tmp = setup_state_dir();
        let codex_home = tempfile::tempdir().unwrap();
        fs::remove_file(paths::claude_md_path(tmp.claude.path())).unwrap();
        fs::create_dir(paths::claude_md_path(tmp.claude.path())).unwrap();

        let output = run_status_capture(tmp.path(), tmp.claude.path(), codex_home.path());

        assert!(output.contains("CLAUDE.md: unreadable"));
    }

    #[test]
    fn scan_failure_is_reported_without_failing_status() {
        let tmp = setup_state_dir();
        let codex_home = tempfile::tempdir().unwrap();
        // Fresh installs no longer create logs/; make the path a FILE so
        // the legacy-log scan fails deterministically.
        let _ = fs::remove_dir_all(paths::logs_dir(tmp.path()));
        fs::write(paths::logs_dir(tmp.path()), "not a directory").unwrap();

        let output = run_status_capture(tmp.path(), tmp.claude.path(), codex_home.path());

        assert!(output.contains("could not scan:"));
        assert!(output.contains("Claude undistilled sessions: 0"));
        assert!(output.contains("CLAUDE.md: in sync"));
    }

    #[test]
    fn corrupt_state_is_the_nonzero_status_case() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(paths::soul_path(tmp.path()), "body\n").unwrap();
        fs::write(paths::state_path(tmp.path()), "not = valid = toml").unwrap();
        fs::create_dir_all(paths::logs_dir(tmp.path())).unwrap();

        let mut out = Vec::new();
        let err = run_with_homes(tmp.path(), &mut out, None, None).unwrap_err();
        assert!(err.to_string().contains("state file"));
    }
}
