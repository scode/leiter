//! End-to-end integration tests that exercise the full CLI binary.
//!
//! Each test sets `LEITER_HOME` to a temp directory so state is isolated
//! from the user's real `~/.leiter/`. Tests that touch `~/.claude/` pass
//! `--claude-home` pointing to a temp directory.

use assert_cmd::Command;
use assert_cmd::cargo::cargo_bin_cmd;
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

fn set_last_distilled(dir: &Path, timestamp: &str) {
    let soul_path = dir.join("soul.md");
    let original = fs::read_to_string(&soul_path).unwrap();
    let updated = original.replace(
        "last_distilled: 1970-01-01T00:00:00Z",
        &format!("last_distilled: {timestamp}"),
    );
    assert_ne!(updated, original, "last_distilled replacement must match");
    fs::write(&soul_path, updated).unwrap();
}

fn tamper_soul_epoch(state_dir: &Path, field: &str, value: u32) {
    let soul_path = state_dir.join("soul.md");
    let original = fs::read_to_string(&soul_path).unwrap();
    let pattern = format!("{field}: 1");
    let replacement = format!("{field}: {value}");
    let updated = original.replacen(&pattern, &replacement, 1);
    assert_ne!(updated, original, "{field} replacement must match");
    fs::write(&soul_path, updated).unwrap();
}

fn corrupt_soul(state_dir: &Path) {
    let soul_path = state_dir.join("soul.md");
    fs::write(&soul_path, "not valid frontmatter\n").unwrap();
}

fn install(state_dir: &Path, claude_home: &Path) {
    leiter(state_dir)
        .args(["claude", &claude_home_flag(claude_home), "install"])
        .assert()
        .success();
}

#[test]
fn config_set_persists_experimental_codex_flag() {
    let tmp = tempfile::tempdir().unwrap();

    leiter(tmp.path())
        .args(["config", "set", "enable_codex_experimental", "true"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "enable_codex_experimental set to true",
        ));

    let config = fs::read_to_string(tmp.path().join("leiter.toml")).unwrap();
    assert!(config.contains("enable_codex_experimental = true"));
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
        .args(["config", "set", "enable_codex_experimental", "true"])
        .assert()
        .success();

    leiter(dir)
        .args(["soul", "distill"])
        .assert()
        .success()
        .stdout(predicate::str::contains("codex hello"));
}

#[test]
fn claude_install_then_context_injects_soul() {
    let tmp = tempfile::tempdir().unwrap();
    let claude_tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    install(dir, claude_tmp.path());

    leiter(dir)
        .args(["hook", "context"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Leiter is a self-training system"))
        .stdout(predicate::str::contains("# Communication Style"))
        .stdout(predicate::str::contains("/leiter-instill"));
}

#[test]
fn claude_install_creates_skill_files() {
    let tmp = tempfile::tempdir().unwrap();
    let claude_tmp = tempfile::tempdir().unwrap();

    install(tmp.path(), claude_tmp.path());

    for name in &[
        "leiter-setup",
        "leiter-distill",
        "leiter-instill",
        "leiter-soul-upgrade",
        "leiter-teardown",
    ] {
        let skill_md = claude_tmp.path().join("skills").join(name).join("SKILL.md");
        assert!(skill_md.is_file(), "missing skill file: {name}");
    }
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

    for name in &[
        "leiter-setup",
        "leiter-distill",
        "leiter-instill",
        "leiter-soul-upgrade",
        "leiter-teardown",
    ] {
        assert!(!claude_tmp.path().join("skills").join(name).exists());
    }

    // State dir is untouched.
    assert!(dir.join("soul.md").is_file());
    assert!(dir.join("logs").is_dir());
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
fn distill_with_epoch_last_distilled_includes_all_logs() {
    let tmp = tempfile::tempdir().unwrap();
    let claude_tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    install(dir, claude_tmp.path());

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

    assert!(stdout.contains("Leiter is a self-training system"));
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
fn nudge_outputs_message_when_stale_logs_exist() {
    let tmp = tempfile::tempdir().unwrap();
    let claude_tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    install(dir, claude_tmp.path());

    let stale_filename = "20260101T000000Z-stale-sess.jsonl";
    let logs_dir = dir.join("logs");
    fs::write(logs_dir.join(stale_filename), "stale log content\n").unwrap();

    leiter(dir)
        .args(["hook", "nudge"])
        .assert()
        .success()
        .stdout(predicate::str::contains("undistilled leiter session logs"));
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
    let obsolete_name = "20250101T000000Z-old-sess.jsonl";
    fs::write(logs_dir.join(obsolete_name), "obsolete content\n").unwrap();

    leiter(dir).args(["soul", "distill"]).assert().success();

    assert!(!logs_dir.join(obsolete_name).exists());
}

#[test]
fn agent_setup_instructions_contain_hook_commands() {
    let tmp = tempfile::tempdir().unwrap();
    let claude_tmp = tempfile::tempdir().unwrap();

    install(tmp.path(), claude_tmp.path());

    leiter(tmp.path())
        .args(["claude", "agent-setup-instructions"])
        .assert()
        .success()
        .stdout(predicate::str::contains("leiter hook context"))
        .stdout(predicate::str::contains("leiter hook nudge"))
        .stdout(predicate::str::contains("leiter hook session-end"));
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
        .stdout(predicate::str::contains("leiter hook context"))
        .stdout(predicate::str::contains("leiter hook nudge"))
        .stdout(predicate::str::contains("leiter hook session-end"))
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

    leiter(dir)
        .args(["soul", "mark-distilled"])
        .assert()
        .success()
        .stdout(predicate::str::contains("last_distilled set to "));

    let content = fs::read_to_string(dir.join("soul.md")).unwrap();
    assert!(
        !content.contains("last_distilled: 1970-01-01T00:00:00Z"),
        "timestamp should have been updated from epoch"
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
fn auto_distill_with_stale_log_outputs_message() {
    let tmp = tempfile::tempdir().unwrap();
    let claude_tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    install(dir, claude_tmp.path());

    let stale_filename = "20260101T000000Z-stale-sess.jsonl";
    let logs_dir = dir.join("logs");
    fs::write(logs_dir.join(stale_filename), "stale log content\n").unwrap();

    leiter(dir)
        .args(["hook", "nudge", "--auto-distill"])
        .assert()
        .success()
        .stdout(predicate::str::contains("/leiter-distill"));
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
    tamper_soul_epoch(dir, "setup_hard_epoch", 2);

    leiter(dir)
        .args(["hook", "context"])
        .assert()
        .success()
        .stdout(predicate::str::contains("ACTION REQUIRED"))
        .stdout(predicate::str::contains("Leiter is a self-training system").not());
}

#[test]
fn context_soft_epoch_mismatch_nudges_and_injects() {
    let tmp = tempfile::tempdir().unwrap();
    let claude_tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    install(dir, claude_tmp.path());
    tamper_soul_epoch(dir, "setup_soft_epoch", 2);

    leiter(dir)
        .args(["hook", "context"])
        .assert()
        .success()
        .stdout(predicate::str::contains("binary is slightly behind"))
        .stdout(predicate::str::contains("Leiter is a self-training system"));
}

#[test]
fn context_corrupt_frontmatter_blocks_soul() {
    let tmp = tempfile::tempdir().unwrap();
    let claude_tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    install(dir, claude_tmp.path());
    corrupt_soul(dir);

    leiter(dir)
        .args(["hook", "context"])
        .assert()
        .success()
        .stdout(predicate::str::contains("ACTION REQUIRED"))
        .stdout(predicate::str::contains("invalid YAML"))
        .stdout(predicate::str::contains("Leiter is a self-training system").not());
}

#[test]
fn session_end_succeeds_despite_hard_epoch_mismatch() {
    let tmp = tempfile::tempdir().unwrap();
    let claude_tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    install(dir, claude_tmp.path());
    tamper_soul_epoch(dir, "setup_hard_epoch", 2);

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
    tamper_soul_epoch(dir, "setup_hard_epoch", 2);

    leiter(dir)
        .args(["soul", "distill"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("incompatible"));
}
