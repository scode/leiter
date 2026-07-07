//! End-to-end integration tests that exercise the full CLI binary.
//!
//! Each test sets `LEITER_HOME` to a temp directory so state is isolated
//! from the user's real `~/.leiter/`. Tests that touch `~/.claude/` pass
//! `--claude-home` pointing to a temp directory.

use assert_cmd::Command;
use assert_cmd::cargo::cargo_bin_cmd;
use chrono::{DateTime, Utc};
use predicates::prelude::*;
use std::fs;
use std::path::Path;

fn leiter(state_dir: &Path) -> Command {
    let mut cmd = cargo_bin_cmd!("leiter");
    cmd.env("LEITER_HOME", state_dir.as_os_str());
    cmd.env("HOME", state_dir.as_os_str());
    cmd
}

fn claude_home_flag(claude_home: &Path) -> String {
    format!("--claude-home={}", claude_home.display())
}

fn codex_home_flag(codex_home: &Path) -> String {
    format!("--codex-home={}", codex_home.display())
}

fn read_last_distilled(dir: &Path) -> DateTime<Utc> {
    let state = fs::read_to_string(dir.join("state.toml")).unwrap();
    let raw = state
        .lines()
        .find_map(|line| line.strip_prefix("last_distilled = "))
        .expect("last_distilled line must exist");
    DateTime::parse_from_rfc3339(raw)
        .unwrap()
        .with_timezone(&Utc)
}

fn set_last_distilled(dir: &Path, timestamp: &str) {
    let state_path = dir.join("state.toml");
    let original = fs::read_to_string(&state_path).unwrap();
    let line = original
        .lines()
        .find(|line| line.starts_with("last_distilled = "))
        .expect("last_distilled line must exist");
    let updated = original.replacen(line, &format!("last_distilled = {timestamp}"), 1);
    assert_ne!(updated, original, "last_distilled replacement must match");
    fs::write(&state_path, updated).unwrap();
}

fn tamper_state_field(state_dir: &Path, field: &str, value: u32) {
    let state_path = state_dir.join("state.toml");
    let original = fs::read_to_string(&state_path).unwrap();
    // Find the current value of the field and replace it.
    let prefix = format!("{field} = ");
    let line = original
        .lines()
        .find(|l| l.starts_with(&prefix))
        .unwrap_or_else(|| panic!("{field} not found in state"));
    let replacement = format!("{field} = {value}");
    let updated = original.replacen(line, &replacement, 1);
    assert_ne!(updated, original, "{field} replacement must match");
    fs::write(&state_path, updated).unwrap();
}

fn corrupt_state(state_dir: &Path) {
    fs::write(state_dir.join("state.toml"), "not = valid = toml\n").unwrap();
}

fn write_legacy_soul(state_dir: &Path) {
    fs::write(
        state_dir.join("soul.md"),
        "---\nlast_distilled: 2026-01-01T00:00:00Z\nsoul_version: 1\nsetup_soft_epoch: 1\nsetup_hard_epoch: 1\n---\nlegacy body\n",
    )
    .unwrap();
}

fn install(state_dir: &Path, claude_home: &Path) {
    leiter(state_dir)
        .args(["claude", &claude_home_flag(claude_home), "install"])
        .assert()
        .success();
}

fn install_default_home(state_dir: &Path) -> std::path::PathBuf {
    let claude_home = state_dir.join(".claude");
    fs::create_dir_all(&claude_home).unwrap();
    install(state_dir, &claude_home);
    claude_home
}

#[test]
fn config_set_persists_codex_flag() {
    let tmp = tempfile::tempdir().unwrap();
    let claude_tmp = tempfile::tempdir().unwrap();
    install(tmp.path(), claude_tmp.path());

    leiter(tmp.path())
        .args(["config", "set", "codex", "true"])
        .assert()
        .success()
        .stdout(predicate::str::contains("codex set to true"));

    let config = fs::read_to_string(tmp.path().join("leiter.toml")).unwrap();
    assert!(config.contains("codex = true"));
    assert!(!config.contains("enable_codex_experimental"));
}

#[test]
fn config_set_legacy_key_prints_deprecation_and_rewrites_to_codex() {
    let tmp = tempfile::tempdir().unwrap();
    install_default_home(tmp.path());

    leiter(tmp.path())
        .args(["config", "set", "enable_codex_experimental", "true"])
        .assert()
        .success()
        .stdout(predicate::str::contains("codex set to true"))
        .stdout(predicate::str::contains("deprecated"));

    let config = fs::read_to_string(tmp.path().join("leiter.toml")).unwrap();
    assert!(config.contains("codex = true"));
    assert!(!config.contains("enable_codex_experimental"));
}

#[test]
fn codex_distill_is_gated_by_experimental_flag() {
    let tmp = tempfile::tempdir().unwrap();
    let claude_tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    install(dir, claude_tmp.path());

    let codex_path = dir.join(".codex").join("sessions").join("session.jsonl");
    fs::create_dir_all(codex_path.parent().unwrap()).unwrap();
    fs::write(
        &codex_path,
        concat!(
            "{\"timestamp\":\"2026-03-07T18:00:00Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"sess\",\"timestamp\":\"2026-03-07T18:00:00Z\"}}\n",
            "{\"timestamp\":\"2026-03-07T18:00:01Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"codex hello\"}]}}\n"
        ),
    )
    .unwrap();

    leiter(dir)
        .args(["soul", "distill"])
        .assert()
        .success()
        .stdout(predicate::str::contains("codex hello").not());
    assert!(!dir.join("codex-meta.toml").exists());

    leiter(dir)
        .args(["config", "set", "codex", "true"])
        .assert()
        .success();

    leiter(dir)
        .args(["soul", "distill"])
        .assert()
        .success()
        .stdout(predicate::str::contains("codex hello"));
}

#[test]
fn claude_install_then_context_reports_retired_hook() {
    let tmp = tempfile::tempdir().unwrap();
    let claude_tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    install(dir, claude_tmp.path());

    leiter(dir)
        .args(["hook", "context"])
        .assert()
        .success()
        .stdout(predicate::str::contains("hooks are no longer needed"))
        .stdout(predicate::str::contains("# Communication Style").not());
}

#[test]
fn claude_install_creates_skill_files() {
    let tmp = tempfile::tempdir().unwrap();
    let claude_tmp = tempfile::tempdir().unwrap();

    install(tmp.path(), claude_tmp.path());

    let skill_md = claude_tmp
        .path()
        .join("skills")
        .join("leiter")
        .join("SKILL.md");
    assert!(skill_md.is_file(), "missing consolidated skill file");
}

#[test]
fn claude_uninstall_removes_plugin_files() {
    let tmp = tempfile::tempdir().unwrap();
    let claude_tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    install(dir, claude_tmp.path());

    leiter(dir)
        .args(["claude", &claude_home_flag(claude_tmp.path()), "uninstall"])
        .assert()
        .success()
        .stderr(predicate::str::contains("removed"));

    assert!(!claude_tmp.path().join("skills").join("leiter").exists());

    // State dir is untouched.
    assert!(dir.join("soul.md").is_file());
    assert!(dir.join("state.toml").is_file());
}

#[test]
fn claude_uninstall_without_install_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let claude_tmp = tempfile::tempdir().unwrap();

    leiter(tmp.path())
        .args(["claude", &claude_home_flag(claude_tmp.path()), "uninstall"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not initialized"));
}

#[test]
fn sync_refuses_hand_edited_block_until_forced() {
    let tmp = tempfile::tempdir().unwrap();
    let claude_home = install_default_home(tmp.path());
    let claude_md = claude_home.join("CLAUDE.md");
    let tampered = fs::read_to_string(&claude_md)
        .unwrap()
        .replace("Communication Style", "Hand Edited Style");
    fs::write(&claude_md, tampered).unwrap();

    leiter(tmp.path())
        .args(["sync"])
        .assert()
        .failure()
        .stdout(predicate::str::contains("refused"))
        .stderr(predicate::str::contains("--force"));

    leiter(tmp.path())
        .args(["sync", "--force"])
        .assert()
        .success()
        .stdout(predicate::str::contains("CLAUDE.md synced"));
}

#[test]
fn mark_distilled_opportunistically_heals_stale_block() {
    let tmp = tempfile::tempdir().unwrap();
    let claude_home = install_default_home(tmp.path());
    let claude_md = claude_home.join("CLAUDE.md");
    fs::write(tmp.path().join("soul.md"), "updated soul\n").unwrap();

    leiter(tmp.path())
        .args(["soul", "mark-distilled"])
        .assert()
        .success();

    assert!(
        fs::read_to_string(&claude_md)
            .unwrap()
            .contains("updated soul")
    );
}

#[test]
fn mark_distilled_warns_but_succeeds_on_hand_edited_block() {
    let tmp = tempfile::tempdir().unwrap();
    let claude_home = install_default_home(tmp.path());
    let claude_md = claude_home.join("CLAUDE.md");
    let tampered = fs::read_to_string(&claude_md)
        .unwrap()
        .replace("Communication Style", "Hand Edited Style");
    fs::write(&claude_md, tampered).unwrap();
    fs::write(tmp.path().join("soul.md"), "new soul body\n").unwrap();

    leiter(tmp.path())
        .args(["soul", "mark-distilled"])
        .assert()
        .success()
        .stdout(predicate::str::contains("warning: CLAUDE.md refused"));

    let block = fs::read_to_string(&claude_md).unwrap();
    assert!(block.contains("Hand Edited Style"));
    assert!(!block.contains("new soul body"));
}

#[test]
fn session_end_saves_transcript() {
    let tmp = tempfile::tempdir().unwrap();
    let claude_tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    install(dir, claude_tmp.path());

    let transcript = tmp.path().join("transcript.jsonl");
    fs::write(&transcript, "{\"role\":\"user\",\"message\":\"hello\"}\n").unwrap();

    let json = serde_json::json!({
        "session_id": "integ-sess",
        "transcript_path": transcript.to_str().unwrap(),
    });

    leiter(dir)
        .args(["hook", "session-end"])
        .write_stdin(json.to_string())
        .assert()
        .success()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("Transcript saved"));

    let logs_dir = dir.join("logs");
    let entries: Vec<_> = fs::read_dir(&logs_dir).unwrap().collect();
    assert_eq!(entries.len(), 1);

    let saved = fs::read_to_string(entries[0].as_ref().unwrap().path()).unwrap();
    assert_eq!(saved, "{\"role\":\"user\",\"message\":\"hello\"}\n");
}

#[test]
fn claude_install_then_session_end_then_distill() {
    let tmp = tempfile::tempdir().unwrap();
    let claude_tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    install(dir, claude_tmp.path());

    let transcript = tmp.path().join("transcript.jsonl");
    fs::write(&transcript, "Integration test transcript.\n").unwrap();

    let json = serde_json::json!({
        "session_id": "integ-sess",
        "transcript_path": transcript.to_str().unwrap(),
    });

    leiter(dir)
        .args(["hook", "session-end"])
        .write_stdin(json.to_string())
        .assert()
        .success()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("Transcript saved"));

    leiter(dir)
        .args(["soul", "distill"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Integration test transcript."))
        .stdout(predicate::str::contains("integ-sess"))
        .stdout(predicate::str::contains("Soul-writing guidelines"));
}

#[test]
fn distill_with_old_last_distilled_includes_all_logs() {
    let tmp = tempfile::tempdir().unwrap();
    let claude_tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    install(dir, claude_tmp.path());
    set_last_distilled(dir, "1970-01-01T00:00:00Z");

    let transcript1 = tmp.path().join("t1.jsonl");
    fs::write(&transcript1, "First log.\n").unwrap();
    let json1 = serde_json::json!({
        "session_id": "first",
        "transcript_path": transcript1.to_str().unwrap(),
    });
    leiter(dir)
        .args(["hook", "session-end"])
        .write_stdin(json1.to_string())
        .assert()
        .success();

    let transcript2 = tmp.path().join("t2.jsonl");
    fs::write(&transcript2, "Second log.\n").unwrap();
    let json2 = serde_json::json!({
        "session_id": "second",
        "transcript_path": transcript2.to_str().unwrap(),
    });
    leiter(dir)
        .args(["hook", "session-end"])
        .write_stdin(json2.to_string())
        .assert()
        .success();

    leiter(dir)
        .args(["soul", "distill"])
        .assert()
        .success()
        .stdout(predicate::str::contains("First log."))
        .stdout(predicate::str::contains("Second log."));
}

#[test]
fn distill_accepts_external_claude_home_flag() {
    let tmp = tempfile::tempdir().unwrap();
    let claude_tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let session_id = "0198fb08-e6e4-7a41-8b3f-2fc8a9ee215d";

    install(dir, claude_tmp.path());
    set_last_distilled(dir, "2026-01-01T00:00:00Z");

    let session_path = claude_tmp
        .path()
        .join("projects")
        .join("proj")
        .join(format!("{session_id}.jsonl"));
    fs::create_dir_all(session_path.parent().unwrap()).unwrap();
    fs::write(
        &session_path,
        "{\"timestamp\":\"2026-07-01T12:00:00Z\",\"type\":\"user\",\"message\":{\"content\":\"external cli hello\"}}\n",
    )
    .unwrap();

    leiter(dir)
        .args(["soul", "distill", &claude_home_flag(claude_tmp.path())])
        .assert()
        .success()
        .stdout(predicate::str::contains("external cli hello"))
        .stdout(predicate::str::contains(format!(
            "projects/proj/{session_id}.jsonl"
        )));

    let state = fs::read_to_string(dir.join("state.toml")).unwrap();
    assert!(state.contains(&format!("[claude.pending.{session_id}]")));
}

#[test]
fn distill_accepts_external_codex_home_flag() {
    let tmp = tempfile::tempdir().unwrap();
    let claude_tmp = tempfile::tempdir().unwrap();
    let codex_tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    install(dir, claude_tmp.path());
    leiter(dir)
        .args(["config", "set", "codex", "true"])
        .assert()
        .success();

    let rollout_path = codex_tmp
        .path()
        .join("sessions")
        .join("2026")
        .join("03")
        .join("07")
        .join("rollout.jsonl");
    fs::create_dir_all(rollout_path.parent().unwrap()).unwrap();
    fs::write(
        &rollout_path,
        concat!(
            "{\"timestamp\":\"2026-03-07T18:00:00Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"codex-sess\",\"timestamp\":\"2026-03-07T18:00:00Z\"}}\n",
            "{\"timestamp\":\"2026-03-07T18:00:01Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"codex override hello\"}]}}\n"
        ),
    )
    .unwrap();

    leiter(dir)
        .args(["soul", "distill", &codex_home_flag(codex_tmp.path())])
        .assert()
        .success()
        .stdout(predicate::str::contains("codex override hello"))
        .stdout(predicate::str::contains(
            "sessions/2026/03/07/rollout.jsonl",
        ));
}

#[test]
fn claude_install_twice_does_not_overwrite_soul() {
    let tmp = tempfile::tempdir().unwrap();
    let claude_tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    install(dir, claude_tmp.path());

    let soul_path = dir.join("soul.md");
    let original = fs::read_to_string(&soul_path).unwrap();
    let modified = format!("{original}\n# Custom Section\n");
    fs::write(&soul_path, &modified).unwrap();

    install(dir, claude_tmp.path());

    let after = fs::read_to_string(&soul_path).unwrap();
    assert_eq!(after, modified);
}

#[test]
fn stdout_stderr_separation() {
    let tmp = tempfile::tempdir().unwrap();
    let claude_tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    install(dir, claude_tmp.path());

    let assert = leiter(dir)
        .args(["-v", "hook", "context"])
        .assert()
        .success();

    let output = assert.get_output();
    let stdout = String::from_utf8(output.stdout.clone()).unwrap();
    let stderr = String::from_utf8(output.stderr.clone()).unwrap();

    assert!(stdout.contains("hooks are no longer needed"));
    assert!(stderr.contains("dispatching command"));
    assert!(!stdout.contains("dispatching command"));
}

#[test]
fn nudge_outputs_nothing_when_no_stale_logs() {
    let tmp = tempfile::tempdir().unwrap();
    let claude_tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    install(dir, claude_tmp.path());

    leiter(dir)
        .args(["hook", "nudge"])
        .assert()
        .success()
        .stdout(predicate::str::is_empty());
}

#[test]
fn nudge_outputs_nothing_when_stale_logs_exist() {
    let tmp = tempfile::tempdir().unwrap();
    let claude_tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    install(dir, claude_tmp.path());
    set_last_distilled(dir, "2026-01-01T00:00:00Z");

    let stale_filename = "20260101T000000Z-stale-sess.jsonl";
    let logs_dir = dir.join("logs");
    fs::create_dir_all(&logs_dir).unwrap();
    fs::write(logs_dir.join(stale_filename), "stale log content\n").unwrap();

    leiter(dir)
        .args(["hook", "nudge"])
        .assert()
        .success()
        .stdout(predicate::str::is_empty());
}

#[test]
fn soul_show_outputs_wrapped_body() {
    let tmp = tempfile::tempdir().unwrap();
    let claude_tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    install(dir, claude_tmp.path());

    leiter(dir)
        .args(["soul", "show"])
        .assert()
        .success()
        .stdout(predicate::str::contains("<leiter-soul-content>"))
        .stdout(predicate::str::contains("</leiter-soul-content>"))
        .stdout(predicate::str::contains("# Communication Style"))
        .stdout(predicate::str::contains("last_distilled").not());
}

#[test]
fn soul_instill_outputs_guidelines_and_preference() {
    let tmp = tempfile::tempdir().unwrap();
    let claude_tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    install(dir, claude_tmp.path());

    leiter(dir)
        .args(["soul", "instill", "always use snake_case"])
        .assert()
        .success()
        .stdout(predicate::str::contains("always use snake_case"))
        .stdout(predicate::str::contains("Soul-writing guidelines"));
}

#[test]
fn distill_dry_run_reports_obsolete_without_deleting() {
    let tmp = tempfile::tempdir().unwrap();
    let claude_tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    install(dir, claude_tmp.path());
    set_last_distilled(dir, "2026-01-01T00:00:00Z");

    let logs_dir = dir.join("logs");
    fs::create_dir_all(&logs_dir).unwrap();
    let obsolete_name = "20250101T000000Z-old-sess.jsonl";
    fs::write(logs_dir.join(obsolete_name), "obsolete content\n").unwrap();

    let transcript = dir.join("transcript.jsonl");
    fs::write(&transcript, "Fresh content.\n").unwrap();
    let json = serde_json::json!({
        "session_id": "new-sess",
        "transcript_path": transcript.to_str().unwrap(),
    });
    leiter(dir)
        .args(["hook", "session-end"])
        .write_stdin(json.to_string())
        .assert()
        .success();

    leiter(dir)
        .args(["soul", "distill", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("would be deleted"))
        .stdout(predicate::str::contains(obsolete_name));

    assert!(logs_dir.join(obsolete_name).exists());
}

#[test]
fn distill_deletes_obsolete_logs() {
    let tmp = tempfile::tempdir().unwrap();
    let claude_tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    install(dir, claude_tmp.path());
    set_last_distilled(dir, "2026-01-01T00:00:00Z");

    let logs_dir = dir.join("logs");
    fs::create_dir_all(&logs_dir).unwrap();
    let obsolete_name = "20250101T000000Z-old-sess.jsonl";
    fs::write(logs_dir.join(obsolete_name), "obsolete content\n").unwrap();

    leiter(dir).args(["soul", "distill"]).assert().success();

    assert!(!logs_dir.join(obsolete_name).exists());
}

#[test]
fn agent_setup_instructions_are_tombstoned() {
    let tmp = tempfile::tempdir().unwrap();
    let claude_tmp = tempfile::tempdir().unwrap();

    install(tmp.path(), claude_tmp.path());

    leiter(tmp.path())
        .args(["claude", "agent-setup-instructions"])
        .assert()
        .success()
        .stdout(predicate::str::contains("hookless"))
        .stdout(predicate::str::contains("leiter claude install"))
        .stdout(predicate::str::contains("leiter hook context").not());
}

#[test]
fn agent_teardown_instructions_contain_hook_commands() {
    let tmp = tempfile::tempdir().unwrap();
    let claude_tmp = tempfile::tempdir().unwrap();

    install(tmp.path(), claude_tmp.path());

    leiter(tmp.path())
        .args(["claude", "agent-teardown-instructions"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "command` field contains `leiter hook`",
        ))
        .stdout(predicate::str::contains("Keep `Bash(leiter:*)`"))
        .stdout(predicate::str::contains(format!(
            "{}/",
            tmp.path().display()
        )));
}

#[test]
fn mark_distilled_updates_timestamp() {
    let tmp = tempfile::tempdir().unwrap();
    let claude_tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    install(dir, claude_tmp.path());
    set_last_distilled(dir, "1970-01-01T00:00:00Z");
    let before = read_last_distilled(dir);

    leiter(dir)
        .args(["soul", "mark-distilled"])
        .assert()
        .success()
        .stdout(predicate::str::contains("last_distilled set to "));

    let after = read_last_distilled(dir);
    assert!(
        after > before,
        "mark-distilled should advance last_distilled"
    );
}

#[test]
fn soul_upgrade_reports_up_to_date_after_claude_install() {
    let tmp = tempfile::tempdir().unwrap();
    let claude_tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    install(dir, claude_tmp.path());

    leiter(dir)
        .args(["soul", "upgrade"])
        .assert()
        .success()
        .stdout(predicate::str::contains("up to date"));
}

#[test]
fn auto_distill_with_stale_log_outputs_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let claude_tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    install(dir, claude_tmp.path());
    set_last_distilled(dir, "2026-01-01T00:00:00Z");

    let stale_filename = "20260101T000000Z-stale-sess.jsonl";
    let logs_dir = dir.join("logs");
    fs::create_dir_all(&logs_dir).unwrap();
    fs::write(logs_dir.join(stale_filename), "stale log content\n").unwrap();

    leiter(dir)
        .args(["hook", "nudge", "--auto-distill"])
        .assert()
        .success()
        .stdout(predicate::str::is_empty());
}

#[test]
fn auto_distill_with_no_stale_logs_outputs_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let claude_tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    install(dir, claude_tmp.path());

    leiter(dir)
        .args(["hook", "nudge", "--auto-distill"])
        .assert()
        .success()
        .stdout(predicate::str::is_empty());
}

#[test]
fn context_hard_epoch_mismatch_blocks_soul() {
    let tmp = tempfile::tempdir().unwrap();
    let claude_tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    install(dir, claude_tmp.path());
    tamper_state_field(dir, "setup_hard_epoch", 3);

    leiter(dir)
        .args(["hook", "context"])
        .assert()
        .success()
        .stdout(predicate::str::contains("ACTION REQUIRED"))
        .stdout(predicate::str::contains("Leiter is a self-training system").not());
}

#[test]
fn context_soft_epoch_mismatch_still_reports_retired_hook_only() {
    let tmp = tempfile::tempdir().unwrap();
    let claude_tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    install(dir, claude_tmp.path());
    tamper_state_field(dir, "setup_soft_epoch", 3);

    leiter(dir)
        .args(["hook", "context"])
        .assert()
        .success()
        .stdout(predicate::str::contains("hooks are no longer needed"))
        .stdout(predicate::str::contains("binary is a bit behind").not());
}

#[test]
fn context_corrupt_state_blocks_soul() {
    let tmp = tempfile::tempdir().unwrap();
    let claude_tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    install(dir, claude_tmp.path());
    corrupt_state(dir);

    leiter(dir)
        .args(["hook", "context"])
        .assert()
        .success()
        .stdout(predicate::str::contains("ACTION REQUIRED"))
        .stdout(predicate::str::contains("state file"))
        .stdout(predicate::str::contains("# Communication Style").not());
}

#[test]
fn session_end_succeeds_despite_hard_epoch_mismatch() {
    let tmp = tempfile::tempdir().unwrap();
    let claude_tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    install(dir, claude_tmp.path());
    tamper_state_field(dir, "setup_hard_epoch", 3);

    let transcript = tmp.path().join("transcript.jsonl");
    fs::write(&transcript, "{\"role\":\"user\",\"message\":\"hello\"}\n").unwrap();

    let json = serde_json::json!({
        "session_id": "epoch-mismatch-sess",
        "transcript_path": transcript.to_str().unwrap(),
    });

    leiter(dir)
        .args(["hook", "session-end"])
        .write_stdin(json.to_string())
        .assert()
        .success()
        .stderr(predicate::str::contains("Transcript saved"));

    let entries: Vec<_> = fs::read_dir(dir.join("logs")).unwrap().collect();
    assert_eq!(entries.len(), 1);
}

#[test]
fn distill_hard_epoch_mismatch_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let claude_tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    install(dir, claude_tmp.path());
    tamper_state_field(dir, "setup_hard_epoch", 3);

    leiter(dir)
        .args(["soul", "distill"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("incompatible"));
}

#[test]
fn user_facing_validating_commands_surface_legacy_user_message() {
    let cases: &[&[&str]] = &[&["distill", "--dry-run"], &["sync"], &["status"]];

    for args in cases {
        let tmp = tempfile::tempdir().unwrap();
        write_legacy_soul(tmp.path());

        leiter(tmp.path())
            .args(*args)
            .assert()
            .failure()
            .stderr(predicate::str::contains("legacy hook-based layout"))
            .stderr(predicate::str::contains("Leiter has moved to hookless operation").not());
    }
}

#[test]
fn agent_facing_validating_commands_surface_legacy_agent_message() {
    let failure_cases: &[&[&str]] = &[
        &["soul", "distill"],
        &["soul", "instill", "remember this"],
        &["claude", "agent-setup-instructions"],
        &["claude", "agent-teardown-instructions"],
    ];

    for args in failure_cases {
        let tmp = tempfile::tempdir().unwrap();
        write_legacy_soul(tmp.path());

        leiter(tmp.path())
            .args(*args)
            .assert()
            .failure()
            .stderr(predicate::str::contains(
                "Leiter has moved to hookless operation",
            ))
            .stderr(predicate::str::contains("legacy hook-based layout").not());
    }

    let tmp = tempfile::tempdir().unwrap();
    write_legacy_soul(tmp.path());
    leiter(tmp.path())
        .args(["hook", "context"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Leiter has moved to hookless operation",
        ))
        .stdout(predicate::str::contains("legacy hook-based layout").not());
}
