//! End-to-end integration tests that exercise the full CLI binary.
//!
//! Each test sets `LEITER_HOME` to a temp directory so state is isolated
//! from the user's real `~/.leiter/`. Since `LEITER_HOME` points directly
//! to the state directory, files live at `$LEITER_HOME/soul.md`,
//! `$LEITER_HOME/logs/`, etc.

use assert_cmd::Command;
use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;
use std::fs;
use std::path::Path;

fn leiter(state_dir: &Path) -> Command {
    let mut cmd = cargo_bin_cmd!("leiter");
    cmd.env("LEITER_HOME", state_dir.as_os_str());
    cmd
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

#[test]
fn setup_install_then_context_injects_soul() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    leiter(dir).args(["setup", "install"]).assert().success();

    leiter(dir)
        .args(["hook", "context"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Leiter is a self-training system"))
        .stdout(predicate::str::contains("# Communication Style"))
        .stdout(predicate::str::contains("leiter soul instill"));
}

#[test]
fn session_end_saves_transcript() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    leiter(dir).args(["setup", "install"]).assert().success();

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
        .stdout(predicate::str::contains("Transcript saved"));

    let logs_dir = dir.join("logs");
    let entries: Vec<_> = fs::read_dir(&logs_dir).unwrap().collect();
    assert_eq!(entries.len(), 1);

    let saved = fs::read_to_string(entries[0].as_ref().unwrap().path()).unwrap();
    assert_eq!(saved, "{\"role\":\"user\",\"message\":\"hello\"}\n");
}

#[test]
fn setup_install_then_session_end_then_distill() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    leiter(dir).args(["setup", "install"]).assert().success();

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
        .stdout(predicate::str::contains("Transcript saved"));

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
    let dir = tmp.path();

    leiter(dir).args(["setup", "install"]).assert().success();

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
fn setup_install_twice_does_not_overwrite_soul() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    leiter(dir).args(["setup", "install"]).assert().success();

    let soul_path = dir.join("soul.md");
    let original = fs::read_to_string(&soul_path).unwrap();
    let modified = format!("{original}\n# Custom Section\n");
    fs::write(&soul_path, &modified).unwrap();

    leiter(dir).args(["setup", "install"]).assert().success();

    let after = fs::read_to_string(&soul_path).unwrap();
    assert_eq!(after, modified);
}

#[test]
fn stdout_stderr_separation() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    leiter(dir).args(["setup", "install"]).assert().success();

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
    let dir = tmp.path();

    leiter(dir).args(["setup", "install"]).assert().success();

    leiter(dir)
        .args(["hook", "nudge"])
        .assert()
        .success()
        .stdout(predicate::str::is_empty());
}

#[test]
fn nudge_outputs_message_when_stale_logs_exist() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    leiter(dir).args(["setup", "install"]).assert().success();

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
    let dir = tmp.path();

    leiter(dir).args(["setup", "install"]).assert().success();

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
    let dir = tmp.path();

    leiter(dir).args(["setup", "install"]).assert().success();
    set_last_distilled(dir, "2026-01-01T00:00:00Z");

    let logs_dir = dir.join("logs");
    let obsolete_name = "20250101T000000Z-old-sess.jsonl";
    fs::write(logs_dir.join(obsolete_name), "obsolete content\n").unwrap();

    // Create a new log via session-end (timestamp is now, after last_distilled)
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

    // Dry-run should report the obsolete file
    leiter(dir)
        .args(["soul", "distill", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("would be deleted"))
        .stdout(predicate::str::contains(obsolete_name));

    // File should still exist
    assert!(logs_dir.join(obsolete_name).exists());
}

#[test]
fn distill_deletes_obsolete_logs() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    leiter(dir).args(["setup", "install"]).assert().success();
    set_last_distilled(dir, "2026-01-01T00:00:00Z");

    let logs_dir = dir.join("logs");
    let obsolete_name = "20250101T000000Z-old-sess.jsonl";
    fs::write(logs_dir.join(obsolete_name), "obsolete content\n").unwrap();

    leiter(dir).args(["soul", "distill"]).assert().success();

    // Obsolete file should be deleted
    assert!(!logs_dir.join(obsolete_name).exists());
}

#[test]
fn setup_uninstall_outputs_hook_removal_instructions() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    leiter(dir)
        .args(["setup", "uninstall"])
        .assert()
        .success()
        .stdout(predicate::str::contains("leiter hook context"))
        .stdout(predicate::str::contains("leiter hook nudge"))
        .stdout(predicate::str::contains("leiter hook session-end"))
        .stdout(predicate::str::contains(format!("{}/", dir.display())))
        .stdout(predicate::str::contains("leiter setup install"));
}

#[test]
fn soul_upgrade_reports_up_to_date_after_setup() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    leiter(dir).args(["setup", "install"]).assert().success();

    leiter(dir)
        .arg("soul-upgrade")
        .assert()
        .success()
        .stdout(predicate::str::contains("up to date"));
}
