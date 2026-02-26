//! End-to-end integration tests that exercise the full CLI binary.
//!
//! Each test sets `LEITER_HOME` to a temp directory so state is isolated
//! from the user's real `~/.leiter/`.

use assert_cmd::Command;
use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;
use std::fs;
use std::path::Path;

use serde_json;

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
        .stdout(predicate::str::contains("# Communication Style"))
        .stdout(predicate::str::contains("leiter instill"));
}

#[test]
fn session_end_saves_transcript() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();

    leiter(home).arg("agent-setup").assert().success();

    // Create a fake transcript file.
    let transcript = tmp.path().join("transcript.jsonl");
    fs::write(&transcript, "{\"role\":\"user\",\"message\":\"hello\"}\n").unwrap();

    let json = serde_json::json!({
        "session_id": "integ-sess",
        "transcript_path": transcript.to_str().unwrap(),
    });

    leiter(home)
        .arg("session-end")
        .write_stdin(json.to_string())
        .assert()
        .success()
        .stdout(predicate::str::contains("Transcript saved"));

    // Verify the log file was created in the logs directory.
    let logs_dir = home.join(".leiter").join("logs");
    let entries: Vec<_> = fs::read_dir(&logs_dir).unwrap().collect();
    assert_eq!(entries.len(), 1);

    let saved = fs::read_to_string(entries[0].as_ref().unwrap().path()).unwrap();
    assert_eq!(saved, "{\"role\":\"user\",\"message\":\"hello\"}\n");
}

#[test]
fn agent_setup_then_session_end_then_distill() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();

    leiter(home).arg("agent-setup").assert().success();

    let transcript = tmp.path().join("transcript.jsonl");
    fs::write(&transcript, "Integration test transcript.\n").unwrap();

    let json = serde_json::json!({
        "session_id": "integ-sess",
        "transcript_path": transcript.to_str().unwrap(),
    });

    leiter(home)
        .arg("session-end")
        .write_stdin(json.to_string())
        .assert()
        .success()
        .stdout(predicate::str::contains("Transcript saved"));

    leiter(home)
        .arg("distill")
        .assert()
        .success()
        .stdout(predicate::str::contains("Integration test transcript."))
        .stdout(predicate::str::contains("integ-sess"))
        .stdout(predicate::str::contains("Soul-writing guidelines"));
}

#[test]
fn distill_with_epoch_last_distilled_includes_all_logs() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();

    leiter(home).arg("agent-setup").assert().success();

    // agent-setup sets last_distilled to epoch, so all logs should appear.
    let transcript1 = tmp.path().join("t1.jsonl");
    fs::write(&transcript1, "First log.\n").unwrap();
    let json1 = serde_json::json!({
        "session_id": "first",
        "transcript_path": transcript1.to_str().unwrap(),
    });
    leiter(home)
        .arg("session-end")
        .write_stdin(json1.to_string())
        .assert()
        .success();

    let transcript2 = tmp.path().join("t2.jsonl");
    fs::write(&transcript2, "Second log.\n").unwrap();
    let json2 = serde_json::json!({
        "session_id": "second",
        "transcript_path": transcript2.to_str().unwrap(),
    });
    leiter(home)
        .arg("session-end")
        .write_stdin(json2.to_string())
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
    let assert = leiter(home).args(["-v", "context"]).assert().success();

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
fn nudge_outputs_nothing_when_no_stale_logs() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();

    leiter(home).arg("agent-setup").assert().success();

    // No logs exist, so nudge should output nothing.
    leiter(home)
        .arg("nudge")
        .assert()
        .success()
        .stdout(predicate::str::is_empty());
}

#[test]
fn nudge_outputs_message_when_stale_logs_exist() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();

    leiter(home).arg("agent-setup").assert().success();

    // Create a log file with a timestamp >24h ago by writing directly to logs dir.
    // agent-setup sets last_distilled to epoch, so any log >= epoch is undistilled.
    let stale_filename = "20260101T000000Z-stale-sess.jsonl";
    let logs_dir = home.join(".leiter").join("logs");
    fs::write(logs_dir.join(stale_filename), "stale log content\n").unwrap();

    leiter(home)
        .arg("nudge")
        .assert()
        .success()
        .stdout(predicate::str::contains("undistilled leiter session logs"));
}

#[test]
fn instill_outputs_guidelines_and_preference() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();

    leiter(home).arg("agent-setup").assert().success();

    leiter(home)
        .args(["instill", "always use snake_case"])
        .assert()
        .success()
        .stdout(predicate::str::contains("always use snake_case"))
        .stdout(predicate::str::contains("Soul-writing guidelines"));
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
