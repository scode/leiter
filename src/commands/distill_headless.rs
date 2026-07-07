//! `leiter distill` — run transcript distillation through a headless agent.
//!
//! This is the unattended path: leiter gathers the same transcript payload as
//! `leiter soul distill`, stages the pending watermarks observed by that scan,
//! pipes a full prompt to an agent command, and promotes the staged state only
//! after the child exits successfully. The child owns soul editing; leiter owns
//! watermarks, timestamps, and managed-block resync.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command as ProcessCommand, Stdio};
use std::thread;

use anyhow::{Context, Result, bail};
use tracing::warn;

use crate::commands::distill::{gather, retention_risk_sessions};
use crate::commands::mark_distilled::{
    DistillCommit, commit_staged_distillation, confirmation_line,
};
use crate::config::{LeiterConfig, load_config_best_effort};
use crate::paths;
use crate::sync::{SyncHomes, resync_and_warn};
use crate::templates::HEADLESS_DISTILL_EDIT_INSTRUCTION;
use crate::validation::{ValidationStatus, validate_state};

/// Run the top-level `leiter distill` command against the default homes.
pub fn run(
    state_dir: &Path,
    out: &mut impl Write,
    err: &mut impl Write,
    dry_run: bool,
) -> Result<()> {
    if let ValidationStatus::Incompatible(reason) = validate_state(state_dir) {
        bail!("{}", reason.user_message());
    }
    let config = load_config_best_effort(state_dir);
    let claude_home = paths::resolve_claude_home(None);
    let codex_home = paths::resolve_codex_home(None, config.codex);
    run_with_homes(
        state_dir,
        out,
        err,
        dry_run,
        &config,
        claude_home.as_deref(),
        codex_home.as_deref(),
    )
}

/// Run `leiter distill` with already-resolved homes.
///
/// This is not a CLI surface. Tests use it to keep all file activity inside
/// temp directories while the real command still has no home override flags.
pub fn run_with_homes(
    state_dir: &Path,
    out: &mut impl Write,
    err: &mut impl Write,
    dry_run: bool,
    config: &LeiterConfig,
    claude_home: Option<&Path>,
    codex_home: Option<&Path>,
) -> Result<()> {
    let gathered = gather(
        state_dir,
        claude_home,
        codex_home,
        config.codex,
        dry_run,
        false,
    )?;
    let prompt = compose_prompt(state_dir, &gathered.payload);

    if dry_run {
        out.write_all(prompt.as_bytes())?;
        return Ok(());
    }

    if !gathered.has_emissions {
        if gathered.staged_session_count > 0 {
            let DistillCommit {
                mut state,
                soul,
                last_distilled,
            } = commit_staged_distillation(
                state_dir,
                config.codex,
                Some(gathered.scan_started_utc),
            )?;
            if let Some(claude_home) = claude_home {
                resync_and_warn(
                    err,
                    state_dir,
                    &mut state,
                    &soul,
                    SyncHomes {
                        claude_home: Some(claude_home),
                        codex_home,
                    },
                    config.codex,
                )?;
            }
            writeln!(
                out,
                "{} empty sessions marked processed",
                gathered.staged_session_count
            )?;
            writeln!(out, "{}", confirmation_line(last_distilled))?;
            remove_empty_legacy_logs_dir(state_dir);
            return Ok(());
        }
        writeln!(out, "nothing to distill")?;
        return Ok(());
    }

    let at_risk = retention_risk_sessions(
        &gathered.external_claude,
        config.retention_warn_days,
        chrono::Utc::now(),
    );
    if !at_risk.is_empty() {
        let labels = at_risk
            .iter()
            .map(|session| session.file_label.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        writeln!(
            err,
            "undistilled Claude sessions are older than retention_warn_days={} and may be close to pruning; distill more often: {}",
            config.retention_warn_days, labels
        )?;
    }

    let command_line = agent_command(config, state_dir);
    let output = run_agent_command(&command_line, prompt.into_bytes())?;

    if !output.status.success() {
        out.write_all(&output.stdout)?;
        err.write_all(&output.stderr)?;
        bail!("agent command exited with {}", output.status);
    }

    let writer_result = match output.writer_result {
        Ok(result) => result,
        Err(_) => {
            out.write_all(&output.stdout)?;
            err.write_all(&output.stderr)?;
            bail!("agent stdin writer thread panicked");
        }
    };
    if let Err(write_err) = writer_result {
        out.write_all(&output.stdout)?;
        err.write_all(&output.stderr)?;
        bail!("failed to write prompt to agent stdin: {write_err}");
    }

    let DistillCommit {
        mut state,
        soul,
        last_distilled,
    } = commit_staged_distillation(state_dir, config.codex, Some(gathered.scan_started_utc))?;
    if let Some(claude_home) = claude_home {
        resync_and_warn(
            err,
            state_dir,
            &mut state,
            &soul,
            SyncHomes {
                claude_home: Some(claude_home),
                codex_home,
            },
            config.codex,
        )?;
    }
    out.write_all(&output.stdout)?;
    err.write_all(&output.stderr)?;
    writeln!(out, "{}", confirmation_line(last_distilled))?;
    remove_empty_legacy_logs_dir(state_dir);

    Ok(())
}

/// Compose the prompt sent to the child agent.
///
/// The transcript payload is already bounded as historical data by
/// `leiter soul distill`; this function appends only the concrete soul path
/// and the edit-only instruction needed for the headless child. It must never
/// add a mark or sync instruction because the parent command performs those
/// bookkeeping steps after a successful child exit.
fn compose_prompt(state_dir: &Path, payload: &[u8]) -> String {
    let payload = String::from_utf8_lossy(payload);
    let soul_path = paths::soul_path(state_dir);
    format!(
        "{payload}\nSoul file: `{}`\n\n{HEADLESS_DISTILL_EDIT_INSTRUCTION}",
        soul_path.display()
    )
}

/// Resolve the command line used for the child agent.
fn agent_command(config: &LeiterConfig, state_dir: &Path) -> Vec<String> {
    if let Some(command) = &config.agent_command {
        return command.clone();
    }

    let soul_perm_path = paths::permission_path(&paths::soul_path(state_dir));
    vec![
        "claude".to_string(),
        "-p".to_string(),
        "--allowedTools".to_string(),
        format!("Read({soul_perm_path}),Edit({soul_perm_path}),Write({soul_perm_path})"),
    ]
}

/// Captured child output plus the result of the stdin writer thread.
struct AgentOutput {
    status: std::process::ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    writer_result: thread::Result<std::io::Result<()>>,
}

/// Spawn the agent command and feed the prompt through stdin concurrently.
fn run_agent_command(command_line: &[String], prompt: Vec<u8>) -> Result<AgentOutput> {
    let Some((program, args)) = command_line.split_first() else {
        bail!("agent_command must contain at least one element");
    };

    let mut child = match ProcessCommand::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(spawn_err) => bail!("failed to spawn agent command `{program}`: {spawn_err}"),
    };

    let mut stdin = child
        .stdin
        .take()
        .context("spawned agent command without piped stdin")?;
    let writer = thread::spawn(move || stdin.write_all(&prompt));
    let output = child
        .wait_with_output()
        .context("failed to wait for agent command")?;
    let writer_result = writer.join();

    Ok(AgentOutput {
        status: output.status,
        stdout: strip_relayed_c0_controls(&output.stdout),
        stderr: strip_relayed_c0_controls(&output.stderr),
        writer_result,
    })
}

/// Remove terminal-control bytes from child output before relaying it.
///
/// The child summary is produced from transcript-derived content. Keeping
/// newline and tab preserves ordinary text formatting, while stripping the
/// rest of the C0 range prevents escape sequences and other control bytes
/// from reaching the user's terminal or cron mail.
fn strip_relayed_c0_controls(bytes: &[u8]) -> Vec<u8> {
    bytes
        .iter()
        .copied()
        .filter(|byte| *byte >= 0x20 || *byte == b'\n' || *byte == b'\t')
        .collect()
}

/// Remove the legacy logs directory once file cleanup has drained it.
fn remove_empty_legacy_logs_dir(state_dir: &Path) {
    let logs_dir = paths::logs_dir(state_dir);
    let is_empty = match fs::read_dir(&logs_dir) {
        Ok(mut entries) => entries.next().is_none(),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => false,
        Err(err) => {
            warn!(
                "failed to inspect legacy logs directory {}: {err}",
                logs_dir.display()
            );
            false
        }
    };

    if is_empty && let Err(err) = fs::remove_dir(&logs_dir) {
        warn!(
            "failed to remove empty legacy logs directory {}: {err}",
            logs_dir.display()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::distill::EmittedExternalClaudeSession;
    use crate::commands::test_support::{bytes_to_string, setup_state_dir, update_state};
    use crate::log_filename::generate_log_filename;
    use crate::managed_block::{compose_block, sha256_hex};
    use crate::state::{LeiterState, SyncHashes};
    use chrono::{Duration, TimeZone, Utc};
    use std::fs;
    use std::path::{Path, PathBuf};

    const SESSION_ID: &str = "11111111-1111-4111-8111-111111111111";

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

    fn success_config(
        script: &Path,
        prompt: &Path,
        soul: &Path,
        state: &Path,
        staged: &Path,
    ) -> LeiterConfig {
        LeiterConfig {
            agent_command: Some(vec![
                script.display().to_string(),
                prompt.display().to_string(),
                soul.display().to_string(),
                state.display().to_string(),
                staged.display().to_string(),
            ]),
            ..Default::default()
        }
    }

    fn run_capture(
        state_dir: &Path,
        config: &LeiterConfig,
        claude_home: &Path,
        dry_run: bool,
    ) -> (Result<()>, String, String) {
        run_capture_with_homes(state_dir, config, claude_home, None, dry_run)
    }

    fn run_capture_with_homes(
        state_dir: &Path,
        config: &LeiterConfig,
        claude_home: &Path,
        codex_home: Option<&Path>,
        dry_run: bool,
    ) -> (Result<()>, String, String) {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let result = run_with_homes(
            state_dir,
            &mut out,
            &mut err,
            dry_run,
            config,
            Some(claude_home),
            codex_home,
        );
        (result, bytes_to_string(out), bytes_to_string(err))
    }

    fn write_progress_only_claude_session(claude_home: &Path, session_id: &str) {
        let path = claude_home
            .join("projects")
            .join("-tmp-proj")
            .join(format!("{session_id}.jsonl"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            path,
            "{\"timestamp\":\"2026-07-01T12:00:00Z\",\"type\":\"progress\",\"data\":{\"type\":\"agent_progress\"}}\n",
        )
        .unwrap();
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

    fn set_mtime(path: &Path, ts: chrono::DateTime<Utc>) {
        let time = filetime::FileTime::from_unix_time(ts.timestamp(), 0);
        filetime::set_file_mtime(path, time).unwrap();
    }

    #[cfg(unix)]
    fn write_script(path: &Path, body: &str) {
        use std::os::unix::fs::PermissionsExt;

        fs::write(path, body).unwrap();
        let mut permissions = fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn successful_agent_run_commits_state_and_resyncs_blocks() {
        let tmp = setup_state_dir();
        write_claude_session(tmp.claude.path(), SESSION_ID, "prefer precise output");
        let script_dir = tempfile::tempdir().unwrap();
        let script = script_dir.path().join("agent.sh");
        let prompt_path = script_dir.path().join("prompt.txt");
        let staged_path = script_dir.path().join("staged.txt");
        write_script(
            &script,
            "#!/bin/sh\ncat > \"$1\"\nprintf '\\n- distilled by fake agent\\n' >> \"$2\"\ngrep '^pending_scan_started_utc = ' \"$3\" > \"$4\"\nprintf 'fake summary\\n'\n",
        );

        let config = success_config(
            &script,
            &prompt_path,
            &paths::soul_path(tmp.path()),
            &paths::state_path(tmp.path()),
            &staged_path,
        );
        let (result, out, err) = run_capture(tmp.path(), &config, tmp.claude.path(), false);
        result.unwrap();
        assert!(err.is_empty());

        let prompt = fs::read_to_string(&prompt_path).unwrap();
        assert!(prompt.contains("Soul-writing guidelines"));
        assert!(prompt.contains("HISTORICAL DATA"));
        assert!(prompt.contains("<session-transcripts>"));
        assert!(prompt.contains(&paths::soul_path(tmp.path()).display().to_string()));
        assert!(prompt.contains(HEADLESS_DISTILL_EDIT_INSTRUCTION));
        assert!(!prompt.contains("mark-distilled"));
        assert!(!prompt.contains("leiter sync"));

        let staged_line = fs::read_to_string(&staged_path).unwrap();
        let staged_ts = staged_line
            .trim()
            .strip_prefix("pending_scan_started_utc = ")
            .unwrap();
        let state = LeiterState::load(&paths::state_path(tmp.path())).unwrap();
        assert!(state.pending_scan_started_utc.is_none());
        assert!(state.claude.pending.is_empty());
        assert!(state.claude.committed.contains_key(SESSION_ID));
        assert_eq!(
            state
                .last_distilled
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            staged_ts
        );

        assert!(out.starts_with("fake summary\n"));
        assert!(out.contains(&format!("last_distilled set to {staged_ts}")));
        assert!(
            fs::read_to_string(paths::claude_md_path(tmp.claude.path()))
                .unwrap()
                .contains("distilled by fake agent")
        );
    }

    #[cfg(unix)]
    #[test]
    fn dry_run_prompt_matches_real_agent_stdin() {
        let tmp = setup_state_dir();
        update_state(tmp.path(), |state| {
            state.last_distilled = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        });
        write_claude_session(tmp.claude.path(), SESSION_ID, "same prompt bytes");
        let obsolete =
            generate_log_filename(Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap(), "old");
        fs::create_dir_all(paths::logs_dir(tmp.path())).unwrap();
        fs::write(paths::logs_dir(tmp.path()).join(obsolete), "old").unwrap();
        let script_dir = tempfile::tempdir().unwrap();
        let script = script_dir.path().join("agent.sh");
        let prompt_path = script_dir.path().join("prompt.txt");
        write_script(
            &script,
            "#!/bin/sh\ncat > \"$1\"\nprintf 'fake summary\\n'\n",
        );
        let config = LeiterConfig {
            agent_command: Some(vec![
                script.display().to_string(),
                prompt_path.display().to_string(),
            ]),
            ..Default::default()
        };

        let (dry_result, dry_out, dry_err) =
            run_capture(tmp.path(), &config, tmp.claude.path(), true);
        dry_result.unwrap();
        assert!(dry_err.is_empty());
        assert!(!dry_out.contains("Obsolete logs that would be deleted"));

        let (real_result, _real_out, real_err) =
            run_capture(tmp.path(), &config, tmp.claude.path(), false);
        real_result.unwrap();
        assert!(real_err.is_empty());
        assert_eq!(dry_out.as_bytes(), fs::read(&prompt_path).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn dry_run_empty_scan_still_prints_prompt() {
        let tmp = setup_state_dir();
        let script_dir = tempfile::tempdir().unwrap();
        let script = script_dir.path().join("agent.sh");
        write_script(&script, "#!/bin/sh\nexit 9\n");
        let config = LeiterConfig {
            agent_command: Some(vec![script.display().to_string()]),
            ..Default::default()
        };

        let (result, out, err) = run_capture(tmp.path(), &config, tmp.claude.path(), true);
        result.unwrap();
        assert!(err.is_empty());
        assert!(out.contains("No new session logs to process."));
        assert!(out.contains(HEADLESS_DISTILL_EDIT_INSTRUCTION));
        assert!(!out.contains("nothing to distill"));
    }

    #[cfg(unix)]
    #[test]
    fn empty_rendered_sessions_are_marked_processed_without_spawn() {
        let tmp = setup_state_dir();
        write_progress_only_claude_session(tmp.claude.path(), SESSION_ID);
        let before = LeiterState::load(&paths::state_path(tmp.path()))
            .unwrap()
            .last_distilled;
        let script_dir = tempfile::tempdir().unwrap();
        let script = script_dir.path().join("agent.sh");
        let marker = script_dir.path().join("spawned");
        write_script(
            &script,
            &format!("#!/bin/sh\ntouch '{}'\n", marker.display()),
        );
        let config = LeiterConfig {
            agent_command: Some(vec![script.display().to_string()]),
            ..Default::default()
        };

        let (result, out, err) = run_capture(tmp.path(), &config, tmp.claude.path(), false);
        result.unwrap();
        assert!(err.is_empty());
        assert!(out.contains("1 empty sessions marked processed"));
        assert!(!marker.exists());

        let state = LeiterState::load(&paths::state_path(tmp.path())).unwrap();
        assert!(state.last_distilled > before);
        assert!(state.pending_scan_started_utc.is_none());
        assert!(state.claude.pending.is_empty());
        assert!(state.claude.committed.contains_key(SESSION_ID));
    }

    #[cfg(unix)]
    #[test]
    fn failing_agent_leaves_pending_uncommitted_and_surfaces_stderr() {
        let tmp = setup_state_dir();
        write_claude_session(tmp.claude.path(), SESSION_ID, "keep pending");
        let before = LeiterState::load(&paths::state_path(tmp.path()))
            .unwrap()
            .last_distilled;
        let script_dir = tempfile::tempdir().unwrap();
        let script = script_dir.path().join("agent.sh");
        write_script(
            &script,
            "#!/bin/sh\ncat > /dev/null\nprintf 'child stderr\\n' >&2\nexit 1\n",
        );
        let config = LeiterConfig {
            agent_command: Some(vec![script.display().to_string()]),
            ..Default::default()
        };

        let (result, _out, err) = run_capture(tmp.path(), &config, tmp.claude.path(), false);
        assert!(result.is_err());
        assert!(err.contains("child stderr"));

        let state = LeiterState::load(&paths::state_path(tmp.path())).unwrap();
        assert_eq!(state.last_distilled, before);
        assert!(state.claude.pending.contains_key(SESSION_ID));
        assert!(!state.claude.committed.contains_key(SESSION_ID));
    }

    #[cfg(unix)]
    #[test]
    fn successful_agent_run_relays_stderr() {
        let tmp = setup_state_dir();
        write_claude_session(tmp.claude.path(), SESSION_ID, "stderr still matters");
        let script_dir = tempfile::tempdir().unwrap();
        let script = script_dir.path().join("agent.sh");
        write_script(
            &script,
            "#!/bin/sh\ncat > /dev/null\nprintf 'child diagnostic\\n' >&2\nprintf 'summary\\n'\n",
        );
        let config = LeiterConfig {
            agent_command: Some(vec![script.display().to_string()]),
            ..Default::default()
        };

        let (result, out, err) = run_capture(tmp.path(), &config, tmp.claude.path(), false);
        result.unwrap();
        assert!(out.contains("summary"));
        assert!(err.contains("child diagnostic"));
    }

    #[cfg(unix)]
    #[test]
    fn superseded_headless_run_does_not_commit_pending() {
        let tmp = setup_state_dir();
        write_claude_session(tmp.claude.path(), SESSION_ID, "superseded content");
        let before = LeiterState::load(&paths::state_path(tmp.path()))
            .unwrap()
            .last_distilled;
        let script_dir = tempfile::tempdir().unwrap();
        let script = script_dir.path().join("agent.sh");
        write_script(
            &script,
            "#!/bin/sh\ncat > /dev/null\nawk '{ if ($1 == \"pending_scan_started_utc\") print \"pending_scan_started_utc = 2030-01-01T00:00:00Z\"; else print }' \"$1\" > \"$1.tmp\" && mv \"$1.tmp\" \"$1\"\nprintf 'summary\\n'\n",
        );
        let config = LeiterConfig {
            agent_command: Some(vec![
                script.display().to_string(),
                paths::state_path(tmp.path()).display().to_string(),
            ]),
            ..Default::default()
        };

        let (result, _out, _err) = run_capture(tmp.path(), &config, tmp.claude.path(), false);
        let err = result.unwrap_err().to_string();
        assert!(err.contains("concurrent distill superseded this run"));

        let state = LeiterState::load(&paths::state_path(tmp.path())).unwrap();
        assert_eq!(state.last_distilled, before);
        assert!(state.claude.pending.contains_key(SESSION_ID));
        assert!(!state.claude.committed.contains_key(SESSION_ID));
    }

    #[cfg(unix)]
    #[test]
    fn missing_agent_binary_leaves_pending_uncommitted() {
        let tmp = setup_state_dir();
        write_claude_session(tmp.claude.path(), SESSION_ID, "missing binary");
        let before = LeiterState::load(&paths::state_path(tmp.path()))
            .unwrap()
            .last_distilled;
        let config = LeiterConfig {
            agent_command: Some(vec!["/nonexistent/xyz".to_string()]),
            ..Default::default()
        };

        let (result, _out, err) = run_capture(tmp.path(), &config, tmp.claude.path(), false);
        assert!(result.is_err());
        assert!(err.is_empty());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("failed to spawn agent command")
        );

        let state = LeiterState::load(&paths::state_path(tmp.path())).unwrap();
        assert_eq!(state.last_distilled, before);
        assert!(state.claude.pending.contains_key(SESSION_ID));
        assert!(!state.claude.committed.contains_key(SESSION_ID));
    }

    #[cfg(unix)]
    #[test]
    fn empty_scan_does_not_spawn_or_advance_last_distilled() {
        let tmp = setup_state_dir();
        let before = LeiterState::load(&paths::state_path(tmp.path()))
            .unwrap()
            .last_distilled;
        let script_dir = tempfile::tempdir().unwrap();
        let script = script_dir.path().join("agent.sh");
        let marker = script_dir.path().join("spawned");
        write_script(
            &script,
            &format!("#!/bin/sh\ntouch '{}'\n", marker.display()),
        );
        let config = LeiterConfig {
            agent_command: Some(vec![script.display().to_string()]),
            ..Default::default()
        };

        let (result, out, err) = run_capture(tmp.path(), &config, tmp.claude.path(), false);
        result.unwrap();
        assert_eq!(out, "nothing to distill\n");
        assert!(err.is_empty());
        assert!(!marker.exists());

        let state = LeiterState::load(&paths::state_path(tmp.path())).unwrap();
        assert_eq!(state.last_distilled, before);
        assert!(state.claude.committed.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn dry_run_prints_prompt_without_spawn_or_staging() {
        let tmp = setup_state_dir();
        write_claude_session(tmp.claude.path(), SESSION_ID, "dry run content");
        let before = fs::read(paths::state_path(tmp.path())).unwrap();
        let script_dir = tempfile::tempdir().unwrap();
        let script = script_dir.path().join("agent.sh");
        let marker = script_dir.path().join("spawned");
        write_script(
            &script,
            &format!("#!/bin/sh\ntouch '{}'\n", marker.display()),
        );
        let config = LeiterConfig {
            agent_command: Some(vec![script.display().to_string()]),
            ..Default::default()
        };

        let (result, out, err) = run_capture(tmp.path(), &config, tmp.claude.path(), true);
        result.unwrap();
        assert!(out.contains("dry run content"));
        assert!(out.contains(HEADLESS_DISTILL_EDIT_INSTRUCTION));
        assert!(err.is_empty());
        assert!(!marker.exists());
        assert_eq!(fs::read(paths::state_path(tmp.path())).unwrap(), before);
    }

    #[cfg(unix)]
    #[test]
    fn stdin_write_failure_does_not_commit() {
        let tmp = setup_state_dir();
        let huge = "x".repeat(2 * 1024 * 1024);
        write_claude_session(tmp.claude.path(), SESSION_ID, &huge);
        let before = LeiterState::load(&paths::state_path(tmp.path()))
            .unwrap()
            .last_distilled;
        let script_dir = tempfile::tempdir().unwrap();
        let script = script_dir.path().join("agent.sh");
        write_script(
            &script,
            "#!/bin/sh\nexec 0<&-\nsleep 1\nprintf 'partial summary\\n'\n",
        );
        let config = LeiterConfig {
            agent_command: Some(vec![script.display().to_string()]),
            ..Default::default()
        };

        let (result, out, _err) = run_capture(tmp.path(), &config, tmp.claude.path(), false);
        let err = result.unwrap_err().to_string();
        assert!(err.contains("failed to write prompt to agent stdin"));
        assert!(out.contains("partial summary"));

        let state = LeiterState::load(&paths::state_path(tmp.path())).unwrap();
        assert_eq!(state.last_distilled, before);
        assert!(state.claude.pending.contains_key(SESSION_ID));
        assert!(!state.claude.committed.contains_key(SESSION_ID));
    }

    #[cfg(unix)]
    #[test]
    fn retention_warning_is_written_to_stderr_for_old_emitted_session() {
        let tmp = setup_state_dir();
        let old_path = write_claude_session(tmp.claude.path(), SESSION_ID, "old enough");
        let fresh_id = "22222222-2222-4222-8222-222222222222";
        let fresh_path = write_claude_session(tmp.claude.path(), fresh_id, "fresh enough");
        let now = Utc::now();
        set_mtime(&old_path, now - Duration::days(22));
        set_mtime(&fresh_path, now - Duration::days(1));
        let script_dir = tempfile::tempdir().unwrap();
        let script = script_dir.path().join("agent.sh");
        write_script(&script, "#!/bin/sh\ncat > /dev/null\nprintf 'summary\\n'\n");
        let config = LeiterConfig {
            retention_warn_days: 21,
            agent_command: Some(vec![script.display().to_string()]),
            ..Default::default()
        };

        let (result, _out, err) = run_capture(tmp.path(), &config, tmp.claude.path(), false);
        result.unwrap();
        assert!(err.contains(SESSION_ID));
        assert!(!err.contains(fresh_id));
    }

    #[cfg(unix)]
    #[test]
    fn codex_headless_distill_promotes_and_resyncs_agents_block() {
        let tmp = setup_state_dir();
        let codex_home = tempfile::tempdir().unwrap();
        write_codex_session(codex_home.path(), "codex-session", "codex preference");
        let soul_path = paths::soul_path(tmp.path());
        let old_soul = fs::read_to_string(&soul_path).unwrap();
        let old_block = compose_block(&soul_path, &old_soul);
        fs::write(paths::agents_md_path(codex_home.path()), &old_block).unwrap();
        update_state(tmp.path(), |state| {
            state.sync.agents_md = Some(SyncHashes {
                soul_hash: sha256_hex(&old_soul),
                block_hash: sha256_hex(&old_block),
            });
        });

        let script_dir = tempfile::tempdir().unwrap();
        let script = script_dir.path().join("agent.sh");
        write_script(
            &script,
            "#!/bin/sh\ncat > /dev/null\nprintf '\\n- codex distilled\\n' >> \"$1\"\nprintf 'summary\\n'\n",
        );
        let config = LeiterConfig {
            codex: true,
            agent_command: Some(vec![
                script.display().to_string(),
                soul_path.display().to_string(),
            ]),
            ..Default::default()
        };

        let (result, _out, err) = run_capture_with_homes(
            tmp.path(),
            &config,
            tmp.claude.path(),
            Some(codex_home.path()),
            false,
        );
        result.unwrap();
        assert!(err.is_empty());

        let state = LeiterState::load(&paths::state_path(tmp.path())).unwrap();
        assert!(state.codex.pending.is_empty());
        assert!(state.codex.committed.contains_key("codex-session"));
        let agents = fs::read_to_string(paths::agents_md_path(codex_home.path())).unwrap();
        assert!(agents.contains("codex distilled"));
    }

    #[cfg(unix)]
    #[test]
    fn successful_run_removes_empty_legacy_logs_dir() {
        let tmp = setup_state_dir();
        write_claude_session(tmp.claude.path(), SESSION_ID, "external content");
        update_state(tmp.path(), |state| {
            state.last_distilled = Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap();
        });
        let obsolete =
            generate_log_filename(Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(), "old");
        fs::create_dir_all(paths::logs_dir(tmp.path())).unwrap();
        fs::write(paths::logs_dir(tmp.path()).join(obsolete), "old").unwrap();

        let script_dir = tempfile::tempdir().unwrap();
        let script = script_dir.path().join("agent.sh");
        let prompt_path = script_dir.path().join("prompt.txt");
        let staged_path = script_dir.path().join("staged.txt");
        write_script(
            &script,
            "#!/bin/sh\ncat > \"$1\"\nprintf '\\n- update\\n' >> \"$2\"\ngrep '^pending_scan_started_utc = ' \"$3\" > \"$4\"\nprintf 'summary\\n'\n",
        );
        let config = success_config(
            &script,
            &prompt_path,
            &paths::soul_path(tmp.path()),
            &paths::state_path(tmp.path()),
            &staged_path,
        );

        run_capture(tmp.path(), &config, tmp.claude.path(), false)
            .0
            .unwrap();
        assert!(!paths::logs_dir(tmp.path()).exists());
    }

    #[cfg(unix)]
    #[test]
    fn successful_run_keeps_non_empty_legacy_logs_dir() {
        let tmp = setup_state_dir();
        write_claude_session(tmp.claude.path(), SESSION_ID, "external content");
        update_state(tmp.path(), |state| {
            state.last_distilled = Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap();
        });
        fs::create_dir_all(paths::logs_dir(tmp.path())).unwrap();
        fs::write(
            paths::logs_dir(tmp.path()).join("keep.txt"),
            "not a leiter log",
        )
        .unwrap();

        let script_dir = tempfile::tempdir().unwrap();
        let script = script_dir.path().join("agent.sh");
        let prompt_path = script_dir.path().join("prompt.txt");
        let staged_path = script_dir.path().join("staged.txt");
        write_script(
            &script,
            "#!/bin/sh\ncat > \"$1\"\nprintf '\\n- update\\n' >> \"$2\"\ngrep '^pending_scan_started_utc = ' \"$3\" > \"$4\"\nprintf 'summary\\n'\n",
        );
        let config = success_config(
            &script,
            &prompt_path,
            &paths::soul_path(tmp.path()),
            &paths::state_path(tmp.path()),
            &staged_path,
        );

        run_capture(tmp.path(), &config, tmp.claude.path(), false)
            .0
            .unwrap();
        assert!(paths::logs_dir(tmp.path()).is_dir());
    }

    #[test]
    fn retention_risk_uses_mtime_threshold() {
        let now = Utc.with_ymd_and_hms(2026, 7, 7, 12, 0, 0).unwrap();
        let sessions = vec![
            EmittedExternalClaudeSession {
                file_label: "old.jsonl".to_string(),
                mtime_utc: Utc.with_ymd_and_hms(2026, 6, 1, 12, 0, 0).unwrap(),
            },
            EmittedExternalClaudeSession {
                file_label: "fresh.jsonl".to_string(),
                mtime_utc: Utc.with_ymd_and_hms(2026, 7, 1, 12, 0, 0).unwrap(),
            },
            EmittedExternalClaudeSession {
                file_label: "boundary.jsonl".to_string(),
                mtime_utc: now - Duration::days(21),
            },
        ];

        let at_risk = retention_risk_sessions(&sessions, 21, now);
        assert_eq!(at_risk.len(), 1);
        assert_eq!(at_risk[0].file_label, "old.jsonl");
    }

    #[test]
    fn c0_controls_are_stripped_from_relayed_child_output() {
        let cleaned = strip_relayed_c0_controls(b"\x1b[31mred\x07\n\tok\r");
        assert_eq!(cleaned, b"[31mred\n\tok");
    }
}
