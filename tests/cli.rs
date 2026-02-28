use assert_cmd::Command;
use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;

fn leiter() -> Command {
    cargo_bin_cmd!("leiter")
}

#[test]
fn parses_setup_install() {
    // setup install tries to create ~/.leiter/ which may fail in sandboxed
    // environments, so we just verify the subcommand is recognized (no
    // "unrecognized subcommand" error).
    let assert = leiter().args(["setup", "install"]).assert();
    assert.stderr(predicate::str::contains("unrecognized subcommand").not());
}

#[test]
fn parses_setup_uninstall() {
    leiter().args(["setup", "uninstall"]).assert().success();
}

#[test]
fn parses_hook_context() {
    leiter().args(["hook", "context"]).assert().success();
}

#[test]
fn parses_soul_distill() {
    // soul distill requires ~/.leiter/soul.md which may not exist.
    let assert = leiter().args(["soul", "distill"]).assert();
    assert.stderr(predicate::str::contains("unrecognized subcommand").not());
}

#[test]
fn parses_hook_nudge() {
    // nudge reads ~/.leiter/soul.md which may not exist in sandboxed environments.
    let assert = leiter().args(["hook", "nudge"]).assert();
    assert.stderr(predicate::str::contains("unrecognized subcommand").not());
}

#[test]
fn parses_session_end() {
    // session-end reads JSON from stdin, so it will fail with empty input,
    // but we verify the subcommand is recognized (no "unrecognized subcommand").
    let assert = leiter().args(["hook", "session-end"]).assert();
    assert.stderr(predicate::str::contains("unrecognized subcommand").not());
}

#[test]
fn parses_soul_upgrade() {
    // soul-upgrade reads ~/.leiter/soul.md which may not exist in sandboxed environments.
    let assert = leiter().arg("soul-upgrade").assert();
    assert.stderr(predicate::str::contains("unrecognized subcommand").not());
}

// Verbosity tests use "dispatching command" (emitted at DEBUG) to verify levels.

#[test]
fn default_level_is_info_no_debug_output() {
    leiter()
        .args(["hook", "context"])
        .assert()
        .success()
        .stderr(predicate::str::contains("dispatching command").not());
}

#[test]
fn verbose_sets_debug() {
    leiter()
        .args(["-v", "hook", "context"])
        .assert()
        .success()
        .stderr(predicate::str::contains("dispatching command"));
}

#[test]
fn double_verbose_sets_trace() {
    leiter()
        .args(["-vv", "hook", "context"])
        .assert()
        .success()
        .stderr(predicate::str::contains("dispatching command"));
}

#[test]
fn quiet_sets_warn() {
    leiter()
        .args(["-q", "hook", "context"])
        .assert()
        .success()
        .stderr(predicate::str::contains("dispatching command").not());
}

#[test]
fn double_quiet_sets_error() {
    leiter()
        .args(["-qq", "hook", "context"])
        .assert()
        .success()
        .stderr(predicate::str::contains("dispatching command").not());
}

#[test]
fn log_level_trace_overrides_quiet() {
    leiter()
        .args(["--log-level=TRACE", "-q", "hook", "context"])
        .assert()
        .success()
        .stderr(predicate::str::contains("dispatching command"));
}

#[test]
fn log_level_warn_overrides_verbose() {
    leiter()
        .args(["--log-level=WARN", "-v", "hook", "context"])
        .assert()
        .success()
        .stderr(predicate::str::contains("dispatching command").not());
}

#[test]
fn parses_soul_instill() {
    leiter()
        .args(["soul", "instill", "test preference"])
        .assert()
        .success();
}

#[test]
fn unknown_subcommand_errors() {
    leiter()
        .arg("nonexistent")
        .assert()
        .failure()
        .stderr(predicate::str::contains("unrecognized subcommand"));
}
