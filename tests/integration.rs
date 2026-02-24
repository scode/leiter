//! End-to-end integration tests that exercise the full CLI binary.
//!
//! Each test sets `LEITER_HOME` to a temp directory so state is isolated
//! from the user's real `~/.leiter/`.

use assert_cmd::cargo::cargo_bin_cmd;
use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::Path;

fn leiter(home: &Path) -> Command {
    let mut cmd = cargo_bin_cmd!("leiter");
    cmd.env("LEITER_HOME", home.as_os_str());
    cmd
}

#[test]
fn agent_setup_then_context_injects_soul() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();

    leiter(home).arg("agent-setup").assert().success();

    leiter(home)
        .arg("context")
        .assert()
        .success()
        .stdout(predicate::str::contains("Leiter is a self-training system"))
        .stdout(predicate::str::contains("# Communication Style"));
}

#[test]
fn agent_setup_then_log_then_distill() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();

    leiter(home).arg("agent-setup").assert().success();

    leiter(home)
        .args(["log", "--session-id", "integ-sess"])
        .write_stdin("Integration test log entry.\n")
        .assert()
        .success()
        .stdout(predicate::str::contains("Session log saved"));

    leiter(home)
        .arg("distill")
        .assert()
        .success()
        .stdout(predicate::str::contains("Integration test log entry."))
        .stdout(predicate::str::contains("integ-sess"));
}

#[test]
fn distill_with_epoch_last_distilled_includes_all_logs() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();

    leiter(home).arg("agent-setup").assert().success();

    // agent-setup sets last_distilled to epoch, so all logs should appear.
    leiter(home)
        .args(["log", "--session-id", "first"])
        .write_stdin("First log.\n")
        .assert()
        .success();

    leiter(home)
        .args(["log", "--session-id", "second"])
        .write_stdin("Second log.\n")
        .assert()
        .success();

    leiter(home)
        .arg("distill")
        .assert()
        .success()
        .stdout(predicate::str::contains("First log."))
        .stdout(predicate::str::contains("Second log."));
}

#[test]
fn stop_hook_block_then_log_then_allow() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();

    leiter(home).arg("agent-setup").assert().success();

    // First stop: stop_hook_active=false → block with session logging prompt.
    // Need raw stdout to parse JSON structure, so extract bytes here.
    let block_output = leiter(home)
        .arg("stop-hook")
        .write_stdin(r#"{"session_id":"hook-sess","stop_hook_active":false}"#)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let block_stdout = String::from_utf8(block_output).unwrap();

    let parsed: serde_json::Value = serde_json::from_str(block_stdout.trim()).unwrap();
    assert_eq!(parsed["decision"], "block");
    assert!(parsed["reason"].as_str().unwrap().contains("hook-sess"));

    // Agent writes the session log as instructed.
    leiter(home)
        .args(["log", "--session-id", "hook-sess"])
        .write_stdin("Session summary from stop hook flow.\n")
        .assert()
        .success();

    // Second stop: stop_hook_active=true → allow (empty stdout).
    leiter(home)
        .arg("stop-hook")
        .write_stdin(r#"{"session_id":"hook-sess","stop_hook_active":true}"#)
        .assert()
        .success()
        .stdout(predicate::str::is_empty());
}

#[test]
fn agent_setup_twice_does_not_overwrite_soul() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();

    leiter(home).arg("agent-setup").assert().success();

    // Modify the soul to detect overwrites.
    let soul_path = home.join(".leiter").join("soul.md");
    let original = fs::read_to_string(&soul_path).unwrap();
    let modified = format!("{original}\n# Custom Section\n");
    fs::write(&soul_path, &modified).unwrap();

    leiter(home).arg("agent-setup").assert().success();

    let after = fs::read_to_string(&soul_path).unwrap();
    assert_eq!(after, modified);
}

#[test]
fn stdout_stderr_separation() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();

    leiter(home).arg("agent-setup").assert().success();

    // With -v, tracing goes to stderr; contractual output goes to stdout.
    let assert = leiter(home)
        .args(["-v", "context"])
        .assert()
        .success();

    let output = assert.get_output();
    let stdout = String::from_utf8(output.stdout.clone()).unwrap();
    let stderr = String::from_utf8(output.stderr.clone()).unwrap();

    // Contractual output is on stdout.
    assert!(stdout.contains("Leiter is a self-training system"));

    // Tracing is on stderr.
    assert!(stderr.contains("dispatching command"));

    // Tracing does NOT leak to stdout.
    assert!(!stdout.contains("dispatching command"));
}

#[test]
fn soul_upgrade_reports_up_to_date_after_setup() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();

    leiter(home).arg("agent-setup").assert().success();

    leiter(home)
        .arg("soul-upgrade")
        .assert()
        .success()
        .stdout(predicate::str::contains("up to date"));
}
