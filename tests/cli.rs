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

fn claude_home_flag(claude_home: &Path) -> String {
    format!("--claude-home={}", claude_home.display())
}

#[test]
fn parses_claude_install() {
    let tmp = tempfile::tempdir().unwrap();
    let claude_tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    leiter(dir)
        .args(["claude", &claude_home_flag(claude_tmp.path()), "install"])
        .assert()
        .success()
        .stderr(predicate::str::contains("installed successfully"));

    assert!(dir.join("soul.md").is_file());
    assert!(dir.join("logs").is_dir());
}

#[test]
fn parses_claude_uninstall() {
    let tmp = tempfile::tempdir().unwrap();
    let claude_tmp = tempfile::tempdir().unwrap();

    leiter(tmp.path())
        .args(["claude", &claude_home_flag(claude_tmp.path()), "install"])
        .assert()
        .success();

    leiter(tmp.path())
        .args(["claude", &claude_home_flag(claude_tmp.path()), "uninstall"])
        .assert()
        .success()
        .stderr(predicate::str::contains("removed"));
}

#[test]
fn parses_claude_agent_setup_instructions() {
    let tmp = tempfile::tempdir().unwrap();
    leiter(tmp.path())
        .args(["claude", "agent-setup-instructions"])
        .assert()
        .success()
        .stdout(predicate::str::contains("leiter hook context"));
}

#[test]
fn parses_claude_agent_teardown_instructions() {
    let tmp = tempfile::tempdir().unwrap();
    leiter(tmp.path())
        .args(["claude", "agent-teardown-instructions"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Remove leiter hooks"));
}

#[test]
fn legacy_setup_subcommand_is_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    leiter(tmp.path())
        .args(["setup", "install"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unrecognized subcommand"));
}

#[test]
fn parses_hook_context() {
    let tmp = tempfile::tempdir().unwrap();
    leiter(tmp.path())
        .args(["hook", "context"])
        .assert()
        .success()
        .stdout(predicate::str::contains("not initialized"));
}

#[test]
fn parses_soul_distill() {
    let tmp = tempfile::tempdir().unwrap();
    leiter(tmp.path())
        .args(["soul", "distill"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("soul file not found"));
}

#[test]
fn parses_hook_nudge() {
    let tmp = tempfile::tempdir().unwrap();
    leiter(tmp.path())
        .args(["hook", "nudge"])
        .assert()
        .success()
        .stdout(predicate::str::is_empty());
}

#[test]
fn parses_session_end() {
    let tmp = tempfile::tempdir().unwrap();
    fs::create_dir_all(tmp.path().join("logs")).unwrap();
    leiter(tmp.path())
        .args(["hook", "session-end"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("failed to parse session-end JSON"));
}

#[test]
fn parses_soul_upgrade() {
    let tmp = tempfile::tempdir().unwrap();
    leiter(tmp.path())
        .args(["soul", "upgrade"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("soul file not found"));
}

// Verbosity tests use "dispatching command" (emitted at DEBUG) to verify levels.

#[test]
fn default_level_is_info_no_debug_output() {
    let tmp = tempfile::tempdir().unwrap();
    leiter(tmp.path())
        .args(["hook", "context"])
        .assert()
        .success()
        .stderr(predicate::str::contains("dispatching command").not());
}

#[test]
fn verbose_sets_debug() {
    let tmp = tempfile::tempdir().unwrap();
    leiter(tmp.path())
        .args(["-v", "hook", "context"])
        .assert()
        .success()
        .stderr(predicate::str::contains("dispatching command"));
}

#[test]
fn double_verbose_sets_trace() {
    let tmp = tempfile::tempdir().unwrap();
    leiter(tmp.path())
        .args(["-vv", "hook", "context"])
        .assert()
        .success()
        .stderr(predicate::str::contains("dispatching command"));
}

#[test]
fn quiet_sets_warn() {
    let tmp = tempfile::tempdir().unwrap();
    leiter(tmp.path())
        .args(["-q", "hook", "context"])
        .assert()
        .success()
        .stderr(predicate::str::contains("dispatching command").not());
}

#[test]
fn double_quiet_sets_error() {
    let tmp = tempfile::tempdir().unwrap();
    leiter(tmp.path())
        .args(["-qq", "hook", "context"])
        .assert()
        .success()
        .stderr(predicate::str::contains("dispatching command").not());
}

#[test]
fn log_level_trace_overrides_quiet() {
    let tmp = tempfile::tempdir().unwrap();
    leiter(tmp.path())
        .args(["--log-level=TRACE", "-q", "hook", "context"])
        .assert()
        .success()
        .stderr(predicate::str::contains("dispatching command"));
}

#[test]
fn log_level_warn_overrides_verbose() {
    let tmp = tempfile::tempdir().unwrap();
    leiter(tmp.path())
        .args(["--log-level=WARN", "-v", "hook", "context"])
        .assert()
        .success()
        .stderr(predicate::str::contains("dispatching command").not());
}

#[test]
fn parses_soul_instill() {
    let tmp = tempfile::tempdir().unwrap();
    leiter(tmp.path())
        .args(["soul", "instill", "test preference"])
        .assert()
        .success()
        .stdout(predicate::str::contains("test preference"))
        .stdout(predicate::str::contains("Soul-writing guidelines"));
}

#[test]
fn parses_soul_mark_distilled() {
    let tmp = tempfile::tempdir().unwrap();
    leiter(tmp.path())
        .args(["soul", "mark-distilled"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("soul file not found"));
}

#[test]
fn version_flag_prints_version() {
    let tmp = tempfile::tempdir().unwrap();
    leiter(tmp.path())
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::starts_with("leiter "));
}

#[test]
fn unknown_subcommand_errors() {
    let tmp = tempfile::tempdir().unwrap();
    leiter(tmp.path())
        .arg("nonexistent")
        .assert()
        .failure()
        .stderr(predicate::str::contains("unrecognized subcommand"));
}
