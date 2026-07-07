#![cfg(feature = "e2e")]

mod harness;

use std::process::Output;

use harness::RemoteHost;
use tracing::info;

const BEGIN_SENTINEL: &str = "<!-- SCODE_LEITER_BEGIN -->";
const END_SENTINEL: &str = "<!-- SCODE_LEITER_END -->";
const SOUL_DELIVERY_MARKER: &str = "LEITER_E2E_HOOKLESS_SOUL_DELIVERY_MARKER_20260707";
const INSTILL_MARKER: &str = "LEITER_E2E_HOOKLESS_INSTILL_MARKER_20260707";
const LEGACY_MARKER: &str = "LEITER_E2E_LEGACY_MIGRATION_MARKER_20260707";

/// Ordered E2E suite for the hookless lifecycle on a remote host.
///
/// The harness cross-compiles and deploys the current binary, runs a fresh
/// `leiter claude install`, then walks the same state a real user would:
/// managed-block delivery, consolidated-skill instill/upgrade, direct
/// transcript scanning, explicit sync/distill/status, tombstones, and legacy
/// migration. The suite deliberately keeps the LLM-dependent checks narrow so
/// deterministic state assertions do most of the verification.
#[test]
fn e2e_suite() {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_max_level(tracing::Level::INFO)
        .init();

    let mut host = match RemoteHost::from_env() {
        Some(h) => h,
        None => {
            eprintln!("LEITER_E2E_DEST not set, skipping E2E tests");
            return;
        }
    };
    host.setup();

    step_1_install_verification(&host);
    step_2_soul_delivery_via_block(&host);
    step_3_instill_preference(&host);
    step_4_headless_distill(&host);
    step_5_status_after_distill(&host);
    step_6_clobber_guard(&host);
    step_7_soul_upgrade(&host);
    step_8_hard_epoch_mismatch_blocks_commands(&host);
    step_9_session_end_exempt_from_epoch_checks(&host);
    step_10_soft_epoch_mismatch_status_advisory(&host);
    step_11_legacy_layout_migration(&host);
    step_12_uninstall_reinstall_convergence(&host);
    step_13_retention_warning(&host);
    step_14_staleness_and_opportunistic_heal(&host);
    step_15_error_paths(&host);
}

/// Verifies the deterministic hookless install artifacts.
///
/// A fresh install should create only the pure soul, state metadata, one
/// consolidated skill, and the managed `CLAUDE.md` block. Hook-era artifacts
/// are either absent or left only as user-owned `settings.json` content that
/// install did not mutate.
fn step_1_install_verification(host: &RemoteHost) {
    info!("Step 1: Install verification");

    assert!(
        host.file_exists("~/.leiter/soul.md"),
        "soul.md should exist"
    );
    assert!(
        host.file_exists("~/.leiter/state.toml"),
        "state.toml should exist"
    );
    assert!(
        !directory_exists(host, "~/.leiter/logs"),
        "install must not create the hook-era logs directory"
    );

    let soul = host.read_file("~/.leiter/soul.md");
    assert!(soul.contains("Communication Style"));
    assert!(
        !soul.starts_with("---"),
        "soul.md should not use frontmatter"
    );
    assert!(!soul.contains("soul_version"));

    let state = host.read_file("~/.leiter/state.toml");
    for field in [
        "soul_version",
        "setup_soft_epoch",
        "setup_hard_epoch",
        "last_distilled",
        "[sync.claude_md]",
    ] {
        assert!(state.contains(field), "state.toml missing {field}");
    }

    let skill_dirs = leiter_skill_dirs(host);
    assert_eq!(
        skill_dirs,
        vec!["leiter"],
        "install should leave exactly one leiter skill directory"
    );
    let skill = host.read_file("~/.claude/skills/leiter/SKILL.md");
    assert!(skill.contains("SCODE_LEITER_INSTALLED"));
    assert!(skill.contains("leiter distill"));
    assert!(!skill.contains("/leiter-setup"));

    let claude_md = host.read_file("~/.claude/CLAUDE.md");
    assert!(claude_md.contains(BEGIN_SENTINEL));
    assert!(claude_md.contains(END_SENTINEL));
    assert!(claude_md.contains("machine-managed by leiter"));
    assert!(claude_md.contains(".leiter/soul.md"));
    assert!(claude_md.contains("Communication Style"));

    let settings_snapshot_path = host.settings_snapshot_path();
    if !settings_snapshot_path.is_empty()
        && host
            .run(&format!("test -f {settings_snapshot_path}"))
            .status
            .success()
    {
        let compare = host.run(&format!(
            "cmp -s ~/.claude/settings.json {settings_snapshot_path}"
        ));
        assert_success(
            &compare,
            "settings.json should be byte-unchanged by leiter claude install",
        );
        let settings = host.read_file("~/.claude/settings.json");
        assert!(
            !settings.contains("leiter hook"),
            "install must not add or preserve leiter hook commands in the clean setup"
        );
    } else {
        assert!(
            !host.file_exists("~/.claude/settings.json"),
            "settings.json was absent before install and should remain absent"
        );
    }
    if !settings_snapshot_path.is_empty() {
        host.run_ok(&format!("rm -f {settings_snapshot_path}"));
    }

    info!("Step 1 passed");
}

/// Proves the managed block is the live soul-delivery path.
///
/// The test writes a marker into `soul.md`, syncs the block, then starts a real
/// Claude session and asks for that exact marker. No hook is configured or
/// invoked for this delivery path.
fn step_2_soul_delivery_via_block(host: &RemoteHost) {
    info!("Step 2: Soul delivery via managed block");

    host.run_ok(&format!(
        "printf '\\n# E2E Delivery Marker\\n{}\\n' >> ~/.leiter/soul.md",
        shell_quote(SOUL_DELIVERY_MARKER)
    ));

    let sync = host.run("leiter sync");
    assert_success(&sync, "leiter sync after planting delivery marker");

    let claude_md = host.read_file("~/.claude/CLAUDE.md");
    assert!(
        claude_md.contains(SOUL_DELIVERY_MARKER),
        "CLAUDE.md block should contain the synced marker"
    );

    let stdout = host.claude_prompt_ok(
        "Quote the exact leiter marker string from your startup instructions. The marker starts with LEITER_E2E_HOOKLESS_SOUL_DELIVERY_MARKER. Return only the marker.",
        5,
    );
    assert!(
        stdout.contains(SOUL_DELIVERY_MARKER),
        "Claude should see the marker from the managed block. Got: {stdout}"
    );

    info!("Step 2 passed");
}

/// Exercises the consolidated skill's instill route with a real Claude session.
///
/// The prompt uses "remember" language so Claude should route through the
/// single `leiter` skill, edit `soul.md`, and then sync. The explicit sync here
/// is still run afterward because the block state is what later steps depend
/// on, not Claude's wording about whether it already synced.
fn step_3_instill_preference(host: &RemoteHost) {
    info!("Step 3: Instill preference");

    let soul_before = host.read_file("~/.leiter/soul.md");
    host.claude_prompt_ok(
        &format!(
            "Remember this exact leiter preference marker in my soul: {INSTILL_MARKER}. Include the exact marker string in the soul. Do not stop until the preference is saved."
        ),
        20,
    );

    let soul_after = host.read_file("~/.leiter/soul.md");
    assert_ne!(soul_before, soul_after, "instill should modify the soul");
    assert!(
        soul_after.contains(INSTILL_MARKER),
        "soul should contain the exact instilled marker. Got:\n{soul_after}"
    );

    let sync = host.run("leiter sync");
    assert_success(&sync, "leiter sync after instill");
    let claude_md = host.read_file("~/.claude/CLAUDE.md");
    assert!(
        claude_md.contains(INSTILL_MARKER),
        "CLAUDE.md block should reflect the instilled preference"
    );
    if host.file_exists("~/.codex/AGENTS.md") {
        let agents_md = host.read_file("~/.codex/AGENTS.md");
        assert!(
            !agents_md.contains(BEGIN_SENTINEL),
            "codex is off, so claude-side flows must not create an AGENTS.md managed block"
        );
    }

    info!("Step 3 passed");
}

/// Runs the hookless headless distill path against real Claude transcripts.
///
/// The prior Claude sessions live under `~/.claude/projects`, so `leiter
/// distill` should scan them directly, invoke `claude -p --model opus` through
/// `agent_command`, and commit `[claude.committed]` watermarks itself.
fn step_4_headless_distill(host: &RemoteHost) {
    info!("Step 4: Headless distill");

    write_remote_file(
        host,
        "~/.leiter/leiter.toml",
        r#"agent_command = ["claude", "-p", "--model", "opus", "--allowedTools", "Read(~/.leiter/soul.md),Edit(~/.leiter/soul.md),Write(~/.leiter/soul.md)"]
"#,
    );

    let state_before = host.read_file("~/.leiter/state.toml");
    let ts_before = extract_last_distilled(&state_before);
    info!(ts_before, "last_distilled before headless distill");

    let output = host.run("timeout 300 leiter distill");
    assert_success(&output, "leiter distill");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("last_distilled set to "),
        "distill should confirm the committed timestamp. Got: {stdout}"
    );

    let state_after = host.read_file("~/.leiter/state.toml");
    let ts_after = extract_last_distilled(&state_after);
    info!(ts_after, "last_distilled after headless distill");
    assert!(
        ts_after > ts_before,
        "last_distilled should advance after distill (before={ts_before}, after={ts_after})"
    );
    assert!(
        state_after.contains("[claude.committed."),
        "distill should commit Claude watermarks. Got:\n{state_after}"
    );
    let soul_after = host.read_file("~/.leiter/soul.md");
    assert!(
        soul_after.contains(INSTILL_MARKER),
        "distill agent dropped the instilled marker"
    );

    info!("Step 4 passed");
}

/// Checks the read-only status report after distillation.
///
/// A just-run headless distill may itself leave one new Claude transcript
/// behind, so this treats 0 or 1 as healthy and still requires the managed
/// block to report in sync.
fn step_5_status_after_distill(host: &RemoteHost) {
    info!("Step 5: Status after distill");

    let output = host.run("leiter status");
    assert_success(&output, "leiter status after distill");
    let stdout = String::from_utf8(output.stdout).expect("non-UTF8 status stdout");
    let count = extract_count(&stdout, "Claude undistilled sessions:");
    assert!(
        count <= 1,
        "status should report 0/low undistilled Claude sessions. Got:\n{stdout}"
    );
    assert!(
        stdout.contains("CLAUDE.md: in sync"),
        "status should report the managed block in sync. Got:\n{stdout}"
    );

    info!("Step 5 passed");
}

/// Verifies that `leiter sync` refuses hand edits inside the managed block.
fn step_6_clobber_guard(host: &RemoteHost) {
    info!("Step 6: Clobber guard");

    host.run_ok(
        r#"python3 - <<'PY'
from pathlib import Path
path = Path.home() / ".claude" / "CLAUDE.md"
text = path.read_text()
path.write_text(text.replace("<!-- SCODE_LEITER_BEGIN -->", "<!-- SCODE_LEITER_BEGIN -->\nE2E HAND EDIT INSIDE MANAGED BLOCK", 1))
PY"#,
    );

    let refused = host.run("leiter sync");
    assert!(
        !refused.status.success(),
        "sync should fail after a hand edit inside the managed block"
    );
    let combined = combined_output(&refused);
    assert!(
        combined.contains("--force"),
        "sync refusal should mention --force. Got:\n{combined}"
    );
    let refused_claude_md = host.read_file("~/.claude/CLAUDE.md");
    assert!(
        refused_claude_md.contains("E2E HAND EDIT INSIDE MANAGED BLOCK"),
        "refused sync must leave the hand-edited managed block untouched"
    );

    let forced = host.run("leiter sync --force");
    assert_success(&forced, "leiter sync --force after hand edit");
    let healed = host.read_file("~/.claude/CLAUDE.md");
    assert!(!healed.contains("E2E HAND EDIT INSIDE MANAGED BLOCK"));
    assert!(healed.contains(INSTILL_MARKER));

    info!("Step 6 passed");
}

/// Keeps the real Claude-driven soul upgrade route covered.
fn step_7_soul_upgrade(host: &RemoteHost) {
    info!("Step 7: Soul upgrade");

    set_state_number(host, "soul_version", 1);
    let state_check = host.read_file("~/.leiter/state.toml");
    assert!(state_check.contains("soul_version = 1"));

    host.claude_prompt_ok(
        "Upgrade the leiter soul. Use the installed leiter skill route and do not stop until `leiter soul mark-upgraded` has succeeded.",
        20,
    );

    let state_after = host.read_file("~/.leiter/state.toml");
    assert!(
        state_after.contains("soul_version = 2"),
        "soul_version should be back to 2 after upgrade. Got:\n{state_after}"
    );

    info!("Step 7 passed");
}

/// A state hard epoch ahead of the binary must block validating commands.
fn step_8_hard_epoch_mismatch_blocks_commands(host: &RemoteHost) {
    info!("Step 8: Hard epoch mismatch blocks commands");

    host.run_ok("cp ~/.leiter/state.toml ~/.leiter/state.toml.bak");
    set_state_number(host, "setup_hard_epoch", 99);

    let output = host.run("leiter status");
    assert!(
        !output.status.success(),
        "status should fail when state hard epoch is ahead of the binary"
    );
    let combined = combined_output(&output).to_lowercase();
    assert!(
        combined.contains("binary is outdated") && combined.contains("upgrade leiter"),
        "hard-epoch failure should use the binary-outdated message. Got:\n{combined}"
    );

    let hook_context = host.run("leiter hook context");
    assert_success(
        &hook_context,
        "leiter hook context under hard epoch mismatch",
    );
    let hook_combined = combined_output(&hook_context).to_lowercase();
    assert!(
        hook_combined.contains("binary is older than your soul file"),
        "hook context should use the agent-relay hard-epoch message. Got:\n{hook_combined}"
    );

    host.run_ok("mv ~/.leiter/state.toml.bak ~/.leiter/state.toml");

    info!("Step 8 passed");
}

/// The SessionEnd tombstone must keep archiving despite epoch mismatch.
fn step_9_session_end_exempt_from_epoch_checks(host: &RemoteHost) {
    info!("Step 9: Session-end tombstone epoch exemption");

    host.run_ok("cp ~/.leiter/state.toml ~/.leiter/state.toml.bak");
    set_state_number(host, "setup_hard_epoch", 99);
    host.run_ok("rm -rf ~/.leiter/logs");
    write_remote_file(
        host,
        "~/.leiter/e2e-session-end-transcript.jsonl",
        "{\"type\":\"user\",\"message\":{\"content\":\"session end tombstone\"}}\n",
    );

    let output = host.run(
        r#"printf '%s' "{\"session_id\":\"e2e-session-end\",\"transcript_path\":\"$HOME/.leiter/e2e-session-end-transcript.jsonl\"}" | leiter hook session-end"#,
    );
    assert_success(&output, "leiter hook session-end under hard epoch mismatch");
    assert!(
        count_log_files(host) > 0,
        "session-end should save the transcript despite epoch mismatch"
    );

    host.run_ok("mv ~/.leiter/state.toml.bak ~/.leiter/state.toml");

    info!("Step 9 passed");
}

/// Soft epoch drift is advisory-only and appears in `leiter status`.
fn step_10_soft_epoch_mismatch_status_advisory(host: &RemoteHost) {
    info!("Step 10: Soft epoch mismatch status advisory");

    host.run_ok("cp ~/.leiter/state.toml ~/.leiter/state.toml.bak");
    set_state_number(host, "setup_soft_epoch", 1);

    let output = host.run("leiter status");
    assert_success(&output, "leiter status with soft epoch behind");
    let stdout = String::from_utf8(output.stdout).expect("non-UTF8 status stdout");
    assert!(
        stdout.contains("Setup advisory") && stdout.contains("leiter claude install"),
        "status should report the soft epoch advisory. Got:\n{stdout}"
    );

    host.run_ok("mv ~/.leiter/state.toml.bak ~/.leiter/state.toml");

    info!("Step 10 passed");
}

/// Deterministically migrates a pre-0.9.0 hook-based layout.
///
/// This constructs the old frontmatter soul, codex-meta sidecar, legacy config
/// key, and mixed hook settings, then verifies that install migrates leiter
/// state without editing `settings.json` itself.
fn step_11_legacy_layout_migration(host: &RemoteHost) {
    info!("Step 11: Legacy layout migration");

    host.run_ok("if [ -f ~/.claude/settings.json ]; then cp ~/.claude/settings.json ~/.claude/settings.json.leiter-e2e-bak; else rm -f ~/.claude/settings.json.leiter-e2e-bak; fi");
    host.run_ok(
        "rm -rf ~/.leiter ~/.claude/skills/leiter ~/.claude/skills/leiter-* ~/.codex/AGENTS.md",
    );
    host.run_ok("mkdir -p ~/.leiter ~/.claude ~/.codex");
    write_remote_file(
        host,
        "~/.leiter/soul.md",
        &format!(
            "---\nlast_distilled: 2026-01-02T03:04:05Z\nsoul_version: 1\nsetup_soft_epoch: 1\nsetup_hard_epoch: 1\n---\n# Legacy E2E Soul\n\n{LEGACY_MARKER}\n"
        ),
    );
    write_remote_file(
        host,
        "~/.leiter/codex-meta.toml",
        r#"version = 1

[committed."codex-e2e"]
path = "/tmp/codex-e2e.jsonl"
size_bytes = 12
mtime_utc = "2026-03-07T18:00:00Z"
session_timestamp_utc = "2026-03-07T17:59:00Z"
latest_event_timestamp_utc = "2026-03-07T18:00:00Z"
"#,
    );
    write_remote_file(
        host,
        "~/.leiter/leiter.toml",
        r#"enable_codex_experimental = true
agent_command = ["claude", "-p", "--model", "opus", "--allowedTools", "Read(~/.leiter/soul.md),Edit(~/.leiter/soul.md),Write(~/.leiter/soul.md)"]
"#,
    );
    host.run_ok("mkdir -p ~/.leiter/logs ~/.claude/skills/leiter-distill");
    write_remote_file(
        host,
        "~/.leiter/logs/20260101T000000Z-obsolete-legacy.jsonl",
        "{\"type\":\"user\",\"message\":{\"content\":\"obsolete legacy e2e log\"}}\n",
    );
    write_remote_file(
        host,
        "~/.leiter/logs/20260102T030406Z-legacy-drain.jsonl",
        "{\"type\":\"user\",\"message\":{\"content\":\"legacy drain e2e log\"}}\n",
    );
    write_remote_file(
        host,
        "~/.claude/skills/leiter-distill/SKILL.md",
        "legacy distill skill\n<!-- SCODE_LEITER_INSTALLED -->\n",
    );
    write_remote_file(
        host,
        "~/.claude/settings.json",
        r#"{
  "hooks": {
    "SessionStart": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "leiter hook context"
          },
          {
            "type": "command",
            "command": "echo decoy hook"
          }
        ]
      }
    ]
  }
}
"#,
    );
    let settings_before_install = host.read_file("~/.claude/settings.json");

    let output = host.run("leiter claude install");
    assert_success(&output, "leiter claude install legacy migration");
    let stdout = String::from_utf8(output.stdout).expect("non-UTF8 install stdout");
    assert!(stdout.contains("Migrated the legacy layout"));
    assert!(stdout.contains("Legacy hook cleanup required"));
    assert!(stdout.contains("Behavior change to relay to the user"));
    assert!(stdout.contains("0 */6 * * * leiter distill"));
    assert!(stdout.contains("cleanupPeriodDays"));

    let state = host.read_file("~/.leiter/state.toml");
    assert!(state.contains("last_distilled = 2026-01-02T03:04:05Z"));
    assert!(state.contains("soul_version = 1"));
    assert!(state.contains("setup_soft_epoch = 2"));
    assert!(state.contains("setup_hard_epoch = 2"));
    assert!(state.contains("[codex.committed.codex-e2e]"));
    assert!(
        !host.file_exists("~/.claude/skills/leiter-distill/SKILL.md"),
        "install should sweep legacy sentinel-bearing leiter skill directories"
    );

    let soul = host.read_file("~/.leiter/soul.md");
    assert!(!soul.starts_with("---"), "migrated soul should be stripped");
    assert!(soul.contains(LEGACY_MARKER));
    assert!(
        !host.file_exists("~/.leiter/codex-meta.toml"),
        "codex-meta.toml should be absorbed and removed"
    );

    let config = host.read_file("~/.leiter/leiter.toml");
    assert!(config.contains("codex = true"));
    assert!(!config.contains("enable_codex_experimental"));

    let claude_md = host.read_file("~/.claude/CLAUDE.md");
    assert!(claude_md.contains(BEGIN_SENTINEL));
    assert!(claude_md.contains(LEGACY_MARKER));
    let agents_md = host.read_file("~/.codex/AGENTS.md");
    assert!(agents_md.contains(BEGIN_SENTINEL));
    assert!(agents_md.contains(LEGACY_MARKER));

    let settings = host.read_file("~/.claude/settings.json");
    assert_eq!(
        settings_before_install, settings,
        "install must inspect but not edit settings.json during legacy migration"
    );
    assert!(settings.contains("leiter hook context"));
    assert!(settings.contains("echo decoy hook"));

    let distill = host.run("timeout 300 leiter distill");
    assert_success(&distill, "leiter distill legacy log drain");
    assert!(
        !directory_exists(host, "~/.leiter/logs"),
        "legacy logs directory should be removed after the first successful drain"
    );
    // No watermark is asserted for the legacy log's session id: watermarks
    // track external transcripts only, and a legacy-only session is drained
    // by deletion (dedup happens via accounted ids, not a committed entry).
    // The drain's success is visible as last_distilled advancing off the
    // fixture value while the logs directory disappears.
    let drained_state = host.read_file("~/.leiter/state.toml");
    assert!(
        !drained_state.contains("last_distilled = 2026-01-02T03:04:05Z"),
        "post-migration distill should advance last_distilled off the fixture value. Got:\n{drained_state}"
    );

    let cleanup = host.run("leiter codex uninstall");
    assert_success(
        &cleanup,
        "cleanup codex block after legacy migration fixture",
    );
    host.run_ok("if [ -f ~/.claude/settings.json.leiter-e2e-bak ]; then mv ~/.claude/settings.json.leiter-e2e-bak ~/.claude/settings.json; else rm -f ~/.claude/settings.json; fi");

    info!("Step 11 passed");
}

/// Covers uninstall/reinstall convergence after the legacy migration path.
///
/// The legacy step restores `settings.json` and removes the temporary Codex
/// block, so this step first recreates Codex delivery through the public
/// command. Uninstall must remove only leiter-owned delivery surfaces while
/// leaving `~/.leiter/` intact for a later install to converge from.
fn step_12_uninstall_reinstall_convergence(host: &RemoteHost) {
    info!("Step 12: Uninstall/reinstall convergence");

    write_remote_file(
        host,
        "~/.claude/CLAUDE.md",
        "user-authored claude preface\n",
    );
    write_remote_file(host, "~/.codex/AGENTS.md", "user-authored codex preface\n");
    let codex_install = host.run("leiter codex install");
    assert_success(
        &codex_install,
        "leiter codex install before uninstall convergence",
    );

    let claude_uninstall = host.run("leiter claude uninstall");
    assert_success(&claude_uninstall, "leiter claude uninstall");
    let claude_md = host.read_file("~/.claude/CLAUDE.md");
    assert!(claude_md.contains("user-authored claude preface"));
    assert!(
        !claude_md.contains(BEGIN_SENTINEL),
        "claude uninstall should remove the managed block"
    );
    assert!(
        !host.file_exists("~/.claude/skills/leiter/SKILL.md"),
        "claude uninstall should remove the consolidated skill"
    );
    assert!(
        host.file_exists("~/.leiter/soul.md") && host.file_exists("~/.leiter/state.toml"),
        "claude uninstall must leave ~/.leiter intact"
    );

    let codex_uninstall = host.run("leiter codex uninstall");
    assert_success(&codex_uninstall, "leiter codex uninstall");
    let agents_md = host.read_file("~/.codex/AGENTS.md");
    assert!(agents_md.contains("user-authored codex preface"));
    assert!(
        !agents_md.contains(BEGIN_SENTINEL),
        "codex uninstall should remove the managed block"
    );
    let config = host.read_file("~/.leiter/leiter.toml");
    assert!(
        config.contains("codex = false"),
        "codex uninstall should persist codex=false. Got:\n{config}"
    );

    let reinstall = host.run("leiter claude install");
    assert_success(&reinstall, "leiter claude install after uninstall");
    let claude_md = host.read_file("~/.claude/CLAUDE.md");
    assert!(claude_md.contains(BEGIN_SENTINEL));
    assert!(
        host.file_exists("~/.claude/skills/leiter/SKILL.md"),
        "reinstall should recreate the consolidated skill"
    );

    info!("Step 12 passed");
}

/// Exercises the status retention warning against controlled transcript mtimes.
fn step_13_retention_warning(host: &RemoteHost) {
    info!("Step 13: Retention warning");

    host.run_ok(
        r#"ts=$(date -u -d '2 days ago' '+%Y-%m-%dT%H:%M:%SZ') && sed -E -i "s/^last_distilled = .*/last_distilled = $ts/" ~/.leiter/state.toml"#,
    );
    write_remote_file(
        host,
        "~/.leiter/leiter.toml",
        r#"codex = false
retention_warn_days = 1
agent_command = ["claude", "-p", "--model", "opus", "--allowedTools", "Read(~/.leiter/soul.md),Edit(~/.leiter/soul.md),Write(~/.leiter/soul.md)"]
"#,
    );
    let old_session = "11111111-1111-4111-8111-111111111111";
    let fresh_session = "22222222-2222-4222-8222-222222222222";
    write_claude_project_session(host, old_session, "old retention warning e2e session");
    host.run_ok(&format!(
        "touch -d '36 hours ago' ~/.claude/projects/leiter-e2e/{old_session}.jsonl"
    ));

    let warned = host.run("leiter status");
    assert_success(&warned, "leiter status with old undistilled transcript");
    let warned_stdout = String::from_utf8(warned.stdout).expect("non-UTF8 status stdout");
    assert!(
        warned_stdout.contains("Retention warning")
            && warned_stdout.contains(&format!("{old_session}.jsonl")),
        "status should name the at-risk transcript. Got:\n{warned_stdout}"
    );

    write_claude_project_session(host, fresh_session, "fresh retention e2e session");
    let first_distill = host.run("timeout 300 leiter distill");
    assert_success(&first_distill, "leiter distill after retention warning");
    write_claude_project_session(host, fresh_session, "fresh retention e2e session amended");
    let fresh_status = host.run("leiter status");
    assert_success(&fresh_status, "leiter status with only fresh transcripts");
    let fresh_stdout = String::from_utf8(fresh_status.stdout).expect("non-UTF8 status stdout");
    assert!(
        !fresh_stdout.contains("Retention warning"),
        "fresh undistilled transcripts should not warn. Got:\n{fresh_stdout}"
    );

    info!("Step 13 passed");
}

/// Proves status reports stale blocks without healing them, while a mutating
/// participant opportunistically repairs the managed block.
fn step_14_staleness_and_opportunistic_heal(host: &RemoteHost) {
    info!("Step 14: Staleness and opportunistic heal");

    host.run_ok(
        "printf '\\n# E2E Opportunistic Heal\\nLEITER_E2E_OPPORTUNISTIC_HEAL_20260707\\n' >> ~/.leiter/soul.md",
    );
    let stale = host.run("leiter status");
    assert_success(&stale, "leiter status after direct soul edit");
    let stale_stdout = String::from_utf8(stale.stdout).expect("non-UTF8 status stdout");
    assert!(
        stale_stdout.contains("CLAUDE.md: stale"),
        "status should report the block stale before a mutating command heals it. Got:\n{stale_stdout}"
    );

    let off = host.run("leiter config set codex false");
    assert_success(&off, "leiter config set codex false");
    let on = host.run("leiter config set codex true");
    assert_success(&on, "leiter config set codex true");

    let healed = host.run("leiter status");
    assert_success(&healed, "leiter status after opportunistic heal");
    let healed_stdout = String::from_utf8(healed.stdout).expect("non-UTF8 status stdout");
    assert!(
        healed_stdout.contains("CLAUDE.md: in sync"),
        "mutating command should opportunistically heal the stale block. Got:\n{healed_stdout}"
    );

    info!("Step 14 passed");
}

/// Covers deterministic validation and failed-agent error paths.
fn step_15_error_paths(host: &RemoteHost) {
    info!("Step 15: Error paths");

    host.run_ok("cp ~/.leiter/state.toml ~/.leiter/state.toml.e2e-good");
    write_remote_file(host, "~/.leiter/state.toml", "this is not valid toml\n");
    let corrupt = host.run("leiter status");
    assert!(
        !corrupt.status.success(),
        "status should fail when state.toml is corrupt"
    );
    let corrupt_combined = combined_output(&corrupt);
    assert!(
        corrupt_combined.contains("leiter state file")
            && corrupt_combined.contains("is corrupt")
            && corrupt_combined.contains("leiter claude install"),
        "corrupt state should surface the user recovery message. Got:\n{corrupt_combined}"
    );
    host.run_ok("mv ~/.leiter/state.toml.e2e-good ~/.leiter/state.toml");

    set_last_distilled(host, "1970-01-01T00:00:00Z");
    write_claude_project_session(
        host,
        "33333333-3333-4333-8333-333333333333",
        "missing agent e2e session",
    );
    let state_before = host.read_file("~/.leiter/state.toml");
    let ts_before = extract_last_distilled(&state_before);
    let config_before = host.read_file("~/.leiter/leiter.toml");
    write_remote_file(
        host,
        "~/.leiter/leiter.toml",
        "codex = true\nagent_command = [\"/nonexistent/leiter-e2e-agent\"]\n",
    );

    let failed = host.run("leiter distill");
    assert!(
        !failed.status.success(),
        "distill should fail when agent_command points at a missing binary"
    );
    let failed_combined = combined_output(&failed);
    assert!(
        failed_combined.contains("failed to spawn agent command"),
        "missing agent failure should explain the spawn error. Got:\n{failed_combined}"
    );
    let state_after = host.read_file("~/.leiter/state.toml");
    assert_eq!(
        ts_before,
        extract_last_distilled(&state_after),
        "failed distill must not advance last_distilled"
    );
    write_remote_file(host, "~/.leiter/leiter.toml", &config_before);

    info!("Step 15 passed");
}

fn assert_success(output: &Output, context: &str) {
    assert!(
        output.status.success(),
        "{context} failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn combined_output(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn directory_exists(host: &RemoteHost, path: &str) -> bool {
    host.run(&format!("test -d {path}")).status.success()
}

fn leiter_skill_dirs(host: &RemoteHost) -> Vec<String> {
    host.run_ok(
        r#"for d in ~/.claude/skills/leiter*; do [ -e "$d" ] && basename "$d"; done | sort"#,
    )
    .lines()
    .filter(|line| !line.is_empty())
    .map(ToOwned::to_owned)
    .collect()
}

fn write_remote_file(host: &RemoteHost, path: &str, content: &str) {
    host.run_ok(&format!(
        "mkdir -p \"$(dirname {path})\" && printf %s {} > {path}",
        shell_quote(content)
    ));
}

fn set_state_number(host: &RemoteHost, key: &str, value: u32) {
    host.run_ok(&format!(
        "sed -E -i {} ~/.leiter/state.toml",
        shell_quote(&format!("s/^{key} = [0-9]+/{key} = {value}/"))
    ));
}

fn set_last_distilled(host: &RemoteHost, value: &str) {
    host.run_ok(&format!(
        "sed -E -i {} ~/.leiter/state.toml",
        shell_quote(&format!("s/^last_distilled = .*/last_distilled = {value}/"))
    ));
}

fn write_claude_project_session(host: &RemoteHost, session_id: &str, text: &str) {
    let path = format!("~/.claude/projects/leiter-e2e/{session_id}.jsonl");
    write_remote_file(
        host,
        &path,
        &format!(
            "{{\"type\":\"user\",\"message\":{{\"content\":\"{}\"}}}}\n",
            json_string_content(text)
        ),
    );
}

fn count_log_files(host: &RemoteHost) -> usize {
    let output = host.run("ls -1 ~/.leiter/logs/ 2>/dev/null");
    if !output.status.success() {
        return 0;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.lines().filter(|l| !l.is_empty()).count()
}

fn extract_last_distilled(state: &str) -> String {
    for line in state.lines() {
        if let Some(rest) = line.strip_prefix("last_distilled =") {
            return rest.trim().to_string();
        }
    }
    panic!("last_distilled not found in state:\n{state}");
}

fn extract_count(report: &str, prefix: &str) -> usize {
    report
        .lines()
        .find_map(|line| line.strip_prefix(prefix))
        .and_then(|rest| rest.trim().parse().ok())
        .unwrap_or_else(|| panic!("could not find count line {prefix:?} in:\n{report}"))
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn json_string_content(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}
