#![cfg(feature = "e2e")]

mod harness;

use harness::RemoteHost;
use tracing::info;

/// Ordered E2E test suite exercising leiter's full lifecycle through real
/// `claude -p` invocations on a remote host.
///
/// Deterministic setup runs first (cross-compile, deploy binary, clean state,
/// `leiter claude install`), then 7 steps that alternate between deterministic
/// file checks and agent-driven Claude prompts. Each step builds on the state
/// left by prior steps.
#[test]
fn e2e_suite() {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_max_level(tracing::Level::INFO)
        .init();

    let host = match RemoteHost::from_env() {
        Some(h) => h,
        None => {
            eprintln!("LEITER_E2E_DEST not set, skipping E2E tests");
            return;
        }
    };
    host.setup();

    step_1_install_verification(&host);
    step_2_agent_driven_setup(&host);
    step_3_soul_injection(&host);
    step_4_session_logging(&host);
    step_5_instill_preference(&host);
    step_6_distill(&host);
    step_7_soul_upgrade(&host);
    step_8_hard_epoch_mismatch_blocks_session(&host);
    step_9_session_end_exempt_from_epoch_checks(&host);
}

/// Fully deterministic. Verifies that `leiter claude install` left the right
/// artifacts: soul.md with expected frontmatter fields, logs/ directory, and
/// all 4 skill files containing the SCODE_LEITER_INSTALLED sentinel. No Claude
/// involvement — just SSH file checks.
fn step_1_install_verification(host: &RemoteHost) {
    info!("Step 1: Install verification");

    assert!(
        host.file_exists("~/.leiter/soul.md"),
        "soul.md should exist after install"
    );

    let soul = host.read_file("~/.leiter/soul.md");
    assert!(
        soul.contains("soul_version"),
        "soul.md should contain soul_version"
    );
    assert!(
        soul.contains("setup_soft_epoch"),
        "soul.md should contain setup_soft_epoch"
    );
    assert!(
        soul.contains("setup_hard_epoch"),
        "soul.md should contain setup_hard_epoch"
    );

    assert!(
        host.run("test -d ~/.leiter/logs").status.success(),
        "logs directory should exist"
    );

    for skill in &[
        "leiter-setup",
        "leiter-distill",
        "leiter-instill",
        "leiter-teardown",
    ] {
        let path = format!("~/.claude/skills/{skill}/SKILL.md");
        assert!(host.file_exists(&path), "missing skill file: {skill}");
        let content = host.read_file(&path);
        assert!(
            content.contains("SCODE_LEITER_INSTALLED"),
            "skill {skill} missing sentinel"
        );
    }

    info!("Step 1 passed");
}

/// Agent-driven. Prompts Claude to run /leiter-setup and accept all optional
/// features. Claude reads the skill file, calls `leiter claude
/// agent-setup-instructions`, gets the hook JSON and permissions prompt, then
/// edits settings.json itself. Deterministic assertions then verify
/// settings.json contains the expected hook commands and permission entries.
fn step_2_agent_driven_setup(host: &RemoteHost) {
    info!("Step 2: Agent-driven setup (/leiter-setup)");

    let output = host.claude_prompt(
        "Run /leiter-setup. Accept ALL optional features. Do not ask me which features I want — just choose all of them and proceed to completion. Do not stop until settings.json has been updated with hooks and permissions.",
        20,
    );

    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        output.status.success(),
        "claude prompt for /leiter-setup failed"
    );

    let settings = host.read_file("~/.claude/settings.json");
    info!(settings = %settings, "settings.json after /leiter-setup");

    let soul = host.read_file("~/.leiter/soul.md");
    info!(soul = %soul, "soul.md after /leiter-setup");

    assert!(
        settings.contains("leiter hook context"),
        "settings.json missing 'leiter hook context'\nclaude stdout: {stdout}\nsettings.json: {settings}"
    );
    assert!(
        settings.contains("leiter hook nudge"),
        "settings.json missing 'leiter hook nudge'\nclaude stdout: {stdout}\nsettings.json: {settings}"
    );
    assert!(
        settings.contains("leiter hook session-end"),
        "settings.json missing 'leiter hook session-end'\nclaude stdout: {stdout}\nsettings.json: {settings}"
    );
    assert!(
        settings.contains("leiter"),
        "settings.json should reference leiter in permissions\nclaude stdout: {stdout}\nsettings.json: {settings}"
    );

    info!("Step 2 passed");
}

/// Agent-driven. Asks Claude "what is leiter?" If the SessionStart hooks from
/// step 2 are working, `leiter hook context` injects the soul at session start
/// and Claude can answer from that context. Fuzzy-matches the response for
/// keywords like "learn", "preference", "soul", or "session".
fn step_3_soul_injection(host: &RemoteHost) {
    info!("Step 3: Soul injection");

    let stdout = host.claude_prompt_ok("What is leiter and what does it do? One sentence.", 5);

    let lower = stdout.to_lowercase();
    assert!(
        lower.contains("learn")
            || lower.contains("preference")
            || lower.contains("soul")
            || lower.contains("session"),
        "Agent should know about leiter from session-start hook. Got: {stdout}"
    );

    info!("Step 3 passed");
}

/// Deterministic setup + timing. Counts log files, sends a trivial prompt
/// ("say hello"), waits for the SessionEnd hook to fire asynchronously, then
/// counts again. The prompt is trivial — the real test is that the SessionEnd
/// hook copied the transcript to ~/.leiter/logs/.
fn step_4_session_logging(host: &RemoteHost) {
    info!("Step 4: Session logging");

    let before = count_log_files(host);
    info!(before, "log file count before prompt");

    host.claude_prompt_ok("Say hello.", 3);

    // SessionEnd hook fires asynchronously after the session terminates
    std::thread::sleep(std::time::Duration::from_secs(3));

    let after = count_log_files(host);
    info!(after, "log file count after prompt");

    assert!(
        after > before,
        "SessionEnd hook should have saved a transcript (before={before}, after={after})"
    );

    info!("Step 4 passed");
}

/// Agent-driven. Asks Claude to "remember" a preference (trigger keyword).
/// Claude auto-matches /leiter-instill, gets writing guidelines from `leiter
/// soul instill`, then edits soul.md. Deterministic assertions verify the soul
/// changed and contains something about the preference.
fn step_5_instill_preference(host: &RemoteHost) {
    info!("Step 5: Instill preference");

    let soul_before = host.read_file("~/.leiter/soul.md");

    host.claude_prompt_ok(
        "Instill that I always prefer 4-space indentation in Python.",
        15,
    );

    let soul_after = host.read_file("~/.leiter/soul.md");

    assert_ne!(
        soul_before, soul_after,
        "Soul should have been modified by instill"
    );

    let lower = soul_after.to_lowercase();
    assert!(
        lower.contains("4-space") || lower.contains("indentation") || lower.contains("python"),
        "Soul should contain the instilled preference. Got:\n{soul_after}"
    );

    info!("Step 5 passed");
}

/// Agent-driven. Asks Claude to distill session logs. Claude invokes
/// /leiter-distill, which spawns a sub-agent that runs `leiter soul distill`,
/// processes transcripts, edits the soul, then the main agent runs
/// `leiter soul mark-distilled`. Deterministic assertion checks that
/// last_distilled timestamp advanced.
fn step_6_distill(host: &RemoteHost) {
    info!("Step 6: Distill");

    let soul_before = host.read_file("~/.leiter/soul.md");
    let ts_before = extract_last_distilled(&soul_before);
    info!(ts_before, "last_distilled before");

    host.claude_prompt_ok("Distill my session logs.", 25);

    let soul_after = host.read_file("~/.leiter/soul.md");
    let ts_after = extract_last_distilled(&soul_after);
    info!(ts_after, "last_distilled after");

    assert!(
        ts_after > ts_before,
        "last_distilled should be newer after distill (before={ts_before}, after={ts_after})"
    );

    info!("Step 6 passed");
}

/// Deterministic setup + agent-driven. Downgrades soul_version to 1 via sed,
/// then asks Claude to upgrade. Claude runs `leiter soul upgrade`, gets the
/// changelog and new template, restructures the soul, and updates soul_version.
/// Deterministic assertion checks soul_version is back to 2.
fn step_7_soul_upgrade(host: &RemoteHost) {
    info!("Step 7: Soul upgrade (synthetic)");

    // Avoid sed -i which behaves differently on BSD sed (macOS remotes).
    host.run_ok("sed 's/soul_version: 2/soul_version: 1/' ~/.leiter/soul.md > ~/.leiter/soul.md.tmp && mv ~/.leiter/soul.md.tmp ~/.leiter/soul.md");

    let soul_check = host.read_file("~/.leiter/soul.md");
    assert!(
        soul_check.contains("soul_version: 1"),
        "soul_version should be 1 after sed"
    );

    host.claude_prompt_ok("Upgrade the leiter soul.", 15);

    let soul_after = host.read_file("~/.leiter/soul.md");
    assert!(
        soul_after.contains("soul_version: 2"),
        "soul_version should be back to 2 after upgrade. Got:\n{soul_after}"
    );

    info!("Step 7 passed");
}

/// Agent-driven. Bumps `setup_hard_epoch` to 99 so the binary sees a hard
/// mismatch. Asks Claude about session startup warnings — since the context
/// hook injects an "ACTION REQUIRED" error instead of the soul, Claude should
/// relay the upgrade/install warning. We ask specifically about hook warnings
/// rather than about leiter, because Claude already knows about leiter from
/// earlier steps and would answer from memory.
fn step_8_hard_epoch_mismatch_blocks_session(host: &RemoteHost) {
    info!("Step 8: Hard epoch mismatch blocks Claude session");

    host.run_ok("cp ~/.leiter/soul.md ~/.leiter/soul.md.bak");

    host.run_ok("sed 's/setup_hard_epoch: 1/setup_hard_epoch: 99/' ~/.leiter/soul.md > ~/.leiter/soul.md.tmp && mv ~/.leiter/soul.md.tmp ~/.leiter/soul.md");

    let soul_check = host.read_file("~/.leiter/soul.md");
    assert!(
        soul_check.contains("setup_hard_epoch: 99"),
        "setup_hard_epoch should be 99 after sed"
    );

    let stdout = host.claude_prompt_ok(
        "Are there any warnings or errors from session startup hooks? If so, tell me what they say.",
        3,
    );

    let lower = stdout.to_lowercase();
    assert!(
        lower.contains("upgrade")
            || lower.contains("older")
            || lower.contains("incompatible")
            || lower.contains("install")
            || lower.contains("action required")
            || lower.contains("epoch"),
        "Agent should relay epoch error to user. Got: {stdout}"
    );

    host.run_ok("mv ~/.leiter/soul.md.bak ~/.leiter/soul.md");

    info!("Step 8 passed");
}

/// Deterministic + timing. Bumps `setup_hard_epoch` to 99, sends a trivial
/// prompt, waits for the SessionEnd hook, and verifies a new log file was
/// saved — proving session-end is exempt from epoch checks.
fn step_9_session_end_exempt_from_epoch_checks(host: &RemoteHost) {
    info!("Step 9: Session-end exempt from epoch checks");

    host.run_ok("cp ~/.leiter/soul.md ~/.leiter/soul.md.bak");

    host.run_ok("sed 's/setup_hard_epoch: 1/setup_hard_epoch: 99/' ~/.leiter/soul.md > ~/.leiter/soul.md.tmp && mv ~/.leiter/soul.md.tmp ~/.leiter/soul.md");

    let before = count_log_files(host);
    info!(before, "log file count before prompt");

    host.claude_prompt_ok("Say hello.", 3);

    std::thread::sleep(std::time::Duration::from_secs(3));

    let after = count_log_files(host);
    info!(after, "log file count after prompt");

    assert!(
        after > before,
        "SessionEnd hook should save transcript despite epoch mismatch (before={before}, after={after})"
    );

    host.run_ok("mv ~/.leiter/soul.md.bak ~/.leiter/soul.md");

    info!("Step 9 passed");
}

fn count_log_files(host: &RemoteHost) -> usize {
    let output = host.run("ls -1 ~/.leiter/logs/ 2>/dev/null");
    if !output.status.success() {
        return 0;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.lines().filter(|l| !l.is_empty()).count()
}

fn extract_last_distilled(soul: &str) -> String {
    for line in soul.lines() {
        if let Some(rest) = line.strip_prefix("last_distilled:") {
            return rest.trim().to_string();
        }
    }
    panic!("last_distilled not found in soul:\n{soul}");
}
