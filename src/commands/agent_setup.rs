//! `leiter claude install` — first-time initialization and plugin file installation.
//!
//! Creates the leiter state directory structure and initial soul file, writes
//! the consolidated skill to `~/.claude/skills/`, then materializes the
//! managed soul-delivery blocks.

use std::fs;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result, bail};
use tracing::info;

use crate::config::load_config_best_effort;
use crate::paths;
use crate::state::LeiterState;
use crate::sync::{SyncHomes, sync_blocks};
use crate::templates::{
    LEGACY_SKILL_DIRS, PLUGIN_SENTINEL, SETUP_SOFT_EPOCH, SKILL_CONTENTS, SOUL_TEMPLATE,
};
use crate::validation::{ValidationStatus, load_and_check_state, validate_state};

/// Run the `leiter claude install` command.
///
/// Creates directories and the initial soul file under `state_dir`, writes
/// skill file under `claude_home`, then writes the managed prompt blocks.
///
/// User-facing confirmation is written to `out` (production passes stdout),
/// matching every other command; diagnostics still go through `tracing`.
pub fn run(
    state_dir: &Path,
    claude_home: &Path,
    codex_home: &Path,
    out: &mut impl Write,
) -> Result<()> {
    init_filesystem(state_dir)?;

    if !claude_home.is_dir() {
        bail!(
            "`{}` does not exist. Is Claude Code installed?",
            claude_home.display()
        );
    }

    write_plugin_files(claude_home)?;
    remove_legacy_skill_dirs(claude_home)?;

    let (mut state, soul) = match validate_state(state_dir) {
        ValidationStatus::Compatible { state, soul, .. } => (state, soul),
        ValidationStatus::Incompatible(reason) => bail!("{}", reason.user_message()),
    };
    let config = load_config_best_effort(state_dir);
    // Install syncs with force on purpose (SPEC install step 6): it is the
    // explicit converge command the user just chose to run, and the clobber
    // guard would otherwise dead-end the documented corrupt-state recovery
    // (fresh state has no recorded hashes while CLAUDE.md still carries the
    // previous block) and dotfiles-restored boxes. Hand edits inside the
    // managed span do not survive an install; the sentinel text warns so.
    let _outcomes = sync_blocks(
        state_dir,
        &mut state,
        &soul,
        SyncHomes {
            claude_home: Some(claude_home),
            codex_home: Some(codex_home),
        },
        config.codex,
        true,
    )?;

    writeln!(out, "Leiter installed successfully.")?;
    writeln!(out, "Wrote the consolidated `leiter` skill.")?;
    writeln!(
        out,
        "Wrote the managed soul block in {}.",
        paths::claude_md_path(claude_home).display()
    )?;
    if config.codex {
        writeln!(
            out,
            "Wrote the managed soul block in {}.",
            paths::agents_md_path(codex_home).display()
        )?;
    }
    writeln!(
        out,
        "The soul is now delivered inline through managed blocks, so soul injection no longer depends on a hook."
    )?;
    writeln!(
        out,
        "The old /leiter-setup skill is gone. If you still want the transitional session-logging and nudge hooks, run `leiter claude agent-setup-instructions` directly."
    )?;

    Ok(())
}

/// Output the agent-setup instructions (hooks and permissions).
///
/// Used by `leiter claude agent-setup-instructions`.
pub fn agent_setup_instructions(state_dir: &Path, out: &mut impl Write) -> Result<()> {
    match validate_state(state_dir) {
        ValidationStatus::Incompatible(reason) => bail!("{}", reason.agent_message()),
        ValidationStatus::Compatible { .. } => {}
    }
    write!(
        out,
        "{}",
        crate::templates::agent_setup_instructions_text(state_dir)
    )?;
    Ok(())
}

/// Deterministic filesystem initialization: create dirs and seed soul file.
fn init_filesystem(state_dir: &Path) -> Result<()> {
    let logs_dir = paths::logs_dir(state_dir);
    let soul_path = paths::soul_path(state_dir);
    let state_path = paths::state_path(state_dir);

    fs::create_dir_all(state_dir)
        .with_context(|| format!("failed to create {}", state_dir.display()))?;
    fs::create_dir_all(&logs_dir)
        .with_context(|| format!("failed to create {}", logs_dir.display()))?;

    let soul_exists = soul_path.exists();
    let state_exists = state_path.exists();

    // Ordering rules that keep a torn install self-healing:
    //
    // - state.toml is always written BEFORE soul.md. If install dies between
    //   the two writes, the rerun lands in the (soul missing, state present)
    //   quadrant, which repairs the soul — instead of (soul present, state
    //   missing), which reads as an unmigrated legacy layout and dead-ends.
    // - Validation runs BEFORE any write in the quadrants that refuse, so a
    //   refused install leaves the layout untouched.
    match (soul_exists, state_exists) {
        (false, false) => {
            LeiterState::fresh().save(&state_path)?;
            fs::write(&soul_path, SOUL_TEMPLATE)
                .with_context(|| format!("failed to write {}", soul_path.display()))?;
            info!("created {}", state_path.display());
            info!("created {}", soul_path.display());
        }
        (true, false) => {
            // Two very different situations look like (soul, no state): a
            // pre-state.toml legacy install (soul still carries YAML
            // frontmatter) and the documented corrupt-state recovery path
            // (user deleted state.toml; soul is already frontmatter-free).
            // The frontmatter is the discriminator.
            if soul_has_legacy_frontmatter(&soul_path)? {
                bail!(
                    "existing leiter layout predates state.toml. Automatic migration is not wired into this revision yet; leave {} in place and upgrade with a later leiter revision that performs the migration.",
                    soul_path.display()
                );
            }
            LeiterState::fresh().save(&state_path)?;
            info!(
                "re-initialized {} (existing soul kept; distillation watermarks reset)",
                state_path.display()
            );
        }
        (false, true) => {
            verify_epochs(state_dir)?;
            fs::write(&soul_path, SOUL_TEMPLATE)
                .with_context(|| format!("failed to write {}", soul_path.display()))?;
            info!("created {}", soul_path.display());
        }
        (true, true) => {
            verify_epochs(state_dir)?;
        }
    }

    Ok(())
}

/// Detect the pre-state.toml layout: a soul that still opens with a YAML
/// frontmatter block.
///
/// A parse failure is treated as "no frontmatter" rather than an error — a
/// soul that merely starts with `---` (say, a horizontal rule) is the agent's
/// content to keep, not a legacy layout to migrate.
fn soul_has_legacy_frontmatter(soul_path: &Path) -> Result<bool> {
    let raw = fs::read_to_string(soul_path)
        .with_context(|| format!("failed to read {}", soul_path.display()))?;
    Ok(crate::frontmatter::parse_soul(&raw).is_ok())
}

/// Write skill files to the Claude Code home directory.
fn write_plugin_files(claude_home: &Path) -> Result<()> {
    for (name, content) in SKILL_CONTENTS {
        let skill_dir = paths::skill_dir(claude_home, name);
        fs::create_dir_all(&skill_dir)
            .with_context(|| format!("failed to create {}", skill_dir.display()))?;
        fs::write(skill_dir.join("SKILL.md"), content)
            .with_context(|| format!("failed to write {}/SKILL.md", skill_dir.display()))?;
        info!("wrote skill {name}");
    }

    Ok(())
}

/// Remove owned directories from the old six-skill install.
///
/// The sentinel check is the ownership boundary. A user-created directory with
/// an old leiter name but no sentinel is left alone because leiter cannot prove
/// it owns that content.
fn remove_legacy_skill_dirs(claude_home: &Path) -> Result<()> {
    for name in LEGACY_SKILL_DIRS {
        let skill_dir = paths::skill_dir(claude_home, name);
        let skill_md = skill_dir.join("SKILL.md");
        let has_sentinel = fs::read_to_string(&skill_md)
            .map(|content| content.contains(PLUGIN_SENTINEL))
            .unwrap_or(false);
        if has_sentinel {
            fs::remove_dir_all(&skill_dir)
                .with_context(|| format!("failed to remove {}", skill_dir.display()))?;
            info!("removed legacy skill {name}");
        }
    }
    Ok(())
}

/// Verify state epochs, migrating an older soft epoch forward on re-run.
///
/// Hard mismatches and corrupt state are delegated to `load_and_check_state`
/// so the direction-specific error texts (setup outdated vs binary outdated)
/// stay in one place. The state-only layer is deliberate: install calls this
/// in quadrants where the soul may not exist yet, and it must refuse or
/// proceed based on state compatibility alone, before any file is written.
/// The soft epoch is the only field install repairs itself: validation treats
/// a soft-ahead state as merely nudge-worthy, but install must refuse it
/// outright — rewriting the file here would downgrade it.
fn verify_epochs(state_dir: &Path) -> Result<()> {
    let mut state = match load_and_check_state(state_dir) {
        Ok(state) => state,
        Err(reason) => bail!("{}", reason.user_message()),
    };

    if state.setup_soft_epoch == SETUP_SOFT_EPOCH {
        info!("epochs already current");
    } else if state.setup_soft_epoch < SETUP_SOFT_EPOCH {
        let old_epoch = state.setup_soft_epoch;
        state.setup_soft_epoch = SETUP_SOFT_EPOCH;
        state.save(&paths::state_path(state_dir))?;
        info!("updated setup_soft_epoch from {old_epoch} to {SETUP_SOFT_EPOCH}");
    } else {
        bail!(
            "state was created by a newer version of leiter \
             (setup_soft_epoch: state={}, binary={SETUP_SOFT_EPOCH}). \
             Please upgrade leiter.",
            state.setup_soft_epoch
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LeiterConfig;
    use crate::state::LeiterState;
    use crate::templates::{SETUP_HARD_EPOCH, SKILL_CONTENTS, SOUL_TEMPLATE_VERSION};
    use chrono::{SubsecRound, TimeZone, Utc};

    fn run_setup(state_dir: &Path, claude_home: &Path) {
        let codex_tmp = tempfile::tempdir().unwrap();
        run(state_dir, claude_home, codex_tmp.path(), &mut Vec::new()).unwrap();
    }

    fn run_setup_result(state_dir: &Path, claude_home: &Path) -> Result<()> {
        let codex_tmp = tempfile::tempdir().unwrap();
        run(state_dir, claude_home, codex_tmp.path(), &mut Vec::new())
    }

    fn run_setup_with_claude_home(state_dir: &Path) -> tempfile::TempDir {
        let claude_tmp = tempfile::tempdir().unwrap();
        run_setup(state_dir, claude_tmp.path());
        claude_tmp
    }

    #[test]
    fn fresh_setup_creates_directories_and_soul() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let _claude_tmp = run_setup_with_claude_home(dir);

        assert!(dir.is_dir());
        assert!(paths::logs_dir(dir).is_dir());
        assert!(paths::soul_path(dir).is_file());
    }

    #[test]
    fn fresh_setup_writes_skill_files() {
        let tmp = tempfile::tempdir().unwrap();
        let claude_tmp = tempfile::tempdir().unwrap();
        run_setup(tmp.path(), claude_tmp.path());

        for (name, _) in SKILL_CONTENTS {
            let skill_md = paths::skill_dir(claude_tmp.path(), name).join("SKILL.md");
            assert!(skill_md.is_file(), "missing skill file: {name}");
        }
    }

    #[test]
    fn fresh_setup_creates_state_with_expected_metadata() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let before = Utc::now().trunc_subsecs(0);
        let _claude_tmp = run_setup_with_claude_home(dir);
        let after = Utc::now();

        let state = LeiterState::load(&paths::state_path(dir)).unwrap();
        assert!(state.last_distilled >= before);
        assert!(state.last_distilled <= after);
        assert_eq!(state.soul_version, SOUL_TEMPLATE_VERSION);
        assert_eq!(state.setup_soft_epoch, SETUP_SOFT_EPOCH);
        assert_eq!(state.setup_hard_epoch, SETUP_HARD_EPOCH);
    }

    #[test]
    fn soul_matches_template_without_frontmatter() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let _claude_tmp = run_setup_with_claude_home(dir);

        let content = fs::read_to_string(paths::soul_path(dir)).unwrap();
        assert_eq!(content, SOUL_TEMPLATE);
    }

    #[test]
    fn rerun_with_matching_epochs_succeeds() {
        let tmp = tempfile::tempdir().unwrap();
        let claude_tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        run_setup(dir, claude_tmp.path());

        let soul = paths::soul_path(dir);
        let before = fs::read_to_string(&soul).unwrap();

        run_setup(dir, claude_tmp.path());

        let after = fs::read_to_string(&soul).unwrap();
        assert_eq!(before, after);
    }

    #[test]
    fn rerun_with_mismatched_hard_epoch_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let claude_tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        run_setup(dir, claude_tmp.path());

        let state_path = paths::state_path(dir);
        let mut state = LeiterState::load(&state_path).unwrap();
        state.setup_hard_epoch = SETUP_HARD_EPOCH + 1;
        state.save(&state_path).unwrap();

        let err = run_setup_result(dir, claude_tmp.path()).unwrap_err();
        assert!(err.to_string().contains("binary is outdated"));
    }

    #[test]
    fn rerun_with_newer_soft_epoch_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let claude_tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        run_setup(dir, claude_tmp.path());

        let state_path = paths::state_path(dir);
        let mut state = LeiterState::load(&state_path).unwrap();
        state.setup_soft_epoch = SETUP_SOFT_EPOCH + 1;
        state.save(&state_path).unwrap();

        let err = run_setup_result(dir, claude_tmp.path()).unwrap_err();
        assert!(err.to_string().contains("newer version"));
    }

    #[test]
    fn rerun_with_older_soft_epoch_migrates_forward() {
        let tmp = tempfile::tempdir().unwrap();
        let claude_tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        run_setup(dir, claude_tmp.path());

        let state_path = paths::state_path(dir);
        let mut state = LeiterState::load(&state_path).unwrap();
        state.setup_soft_epoch = SETUP_SOFT_EPOCH - 1;
        state.save(&state_path).unwrap();

        run_setup(dir, claude_tmp.path());

        let updated = LeiterState::load(&state_path).unwrap();
        assert_eq!(updated.setup_soft_epoch, SETUP_SOFT_EPOCH);
    }

    #[test]
    fn soft_epoch_migration_preserves_body() {
        let tmp = tempfile::tempdir().unwrap();
        let claude_tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        run_setup(dir, claude_tmp.path());

        let soul = paths::soul_path(dir);
        let original_body = fs::read_to_string(&soul).unwrap();
        let state_path = paths::state_path(dir);
        let mut state = LeiterState::load(&state_path).unwrap();
        state.setup_soft_epoch = SETUP_SOFT_EPOCH - 1;
        state.save(&state_path).unwrap();

        run_setup(dir, claude_tmp.path());

        let updated_body = fs::read_to_string(&soul).unwrap();
        assert_eq!(updated_body, original_body);
    }

    #[test]
    fn rerun_with_older_hard_epoch_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let claude_tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        run_setup(dir, claude_tmp.path());

        let state_path = paths::state_path(dir);
        let mut state = LeiterState::load(&state_path).unwrap();
        state.setup_hard_epoch = 0;
        state.save(&state_path).unwrap();

        let err = run_setup_result(dir, claude_tmp.path()).unwrap_err();
        assert!(err.to_string().contains("setup is incompatible"));
        assert!(err.to_string().contains("leiter claude install"));
    }

    #[test]
    fn rerun_with_corrupt_state_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let claude_tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        run_setup(dir, claude_tmp.path());

        fs::write(paths::state_path(dir), "not = valid = toml").unwrap();

        let err = run_setup_result(dir, claude_tmp.path()).unwrap_err();
        assert!(err.to_string().contains("is corrupt"));
    }

    #[test]
    fn legacy_frontmatter_soul_without_state_reports_legacy_layout() {
        let tmp = tempfile::tempdir().unwrap();
        let claude_tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path()).unwrap();
        // A legacy soul is discriminated by its YAML frontmatter.
        let legacy_soul = format!(
            "---\nlast_distilled: 2026-01-01T00:00:00Z\nsoul_version: 1\n---\n{SOUL_TEMPLATE}"
        );
        fs::write(paths::soul_path(tmp.path()), legacy_soul).unwrap();

        let err = run_setup_result(tmp.path(), claude_tmp.path()).unwrap_err();
        assert!(err.to_string().contains("predates state.toml"));
    }

    /// The corrupt-state recovery path: the CLI's own error message tells the
    /// user to delete state.toml and re-run install, so install must accept a
    /// frontmatter-free soul with no state — keeping the soul, resetting
    /// watermarks — rather than misreading it as an unmigrated legacy layout.
    #[test]
    fn frontmatter_free_soul_without_state_recovers_with_fresh_state() {
        let tmp = tempfile::tempdir().unwrap();
        let claude_tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path()).unwrap();
        let user_soul = "# My soul\n\nLearned things live here.\n";
        fs::write(paths::soul_path(tmp.path()), user_soul).unwrap();

        run_setup(tmp.path(), claude_tmp.path());

        let state = LeiterState::load(&paths::state_path(tmp.path())).unwrap();
        assert_eq!(state.soul_version, SOUL_TEMPLATE_VERSION);
        assert_eq!(
            fs::read_to_string(paths::soul_path(tmp.path())).unwrap(),
            user_soul,
            "recovery must keep the user's soul, not overwrite it with the template"
        );
    }

    /// A torn first install can leave `state.toml` behind without `soul.md`.
    /// Re-running install must repair only the missing generated file; resetting
    /// state here would lose distillation watermarks that may already be valid.
    #[test]
    fn missing_soul_with_existing_state_recreates_soul_and_preserves_state() {
        let tmp = tempfile::tempdir().unwrap();
        let claude_tmp = tempfile::tempdir().unwrap();
        let state_dir = tmp.path();
        run_setup(state_dir, claude_tmp.path());

        let known_last_distilled = Utc.with_ymd_and_hms(2026, 8, 3, 9, 30, 0).unwrap();
        let state_path = paths::state_path(state_dir);
        let mut state = LeiterState::load(&state_path).unwrap();
        state.last_distilled = known_last_distilled;
        state.save(&state_path).unwrap();
        fs::remove_file(paths::soul_path(state_dir)).unwrap();

        run_setup(state_dir, claude_tmp.path());

        let soul = fs::read_to_string(paths::soul_path(state_dir)).unwrap();
        let state = LeiterState::load(&state_path).unwrap();
        assert_eq!(soul, SOUL_TEMPLATE);
        assert_eq!(state.last_distilled, known_last_distilled);
    }

    #[test]
    fn running_twice_still_creates_missing_directories() {
        let tmp = tempfile::tempdir().unwrap();
        let claude_tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        run_setup(dir, claude_tmp.path());

        fs::remove_dir(paths::logs_dir(dir)).unwrap();

        run_setup(dir, claude_tmp.path());

        assert!(paths::logs_dir(dir).is_dir());
    }

    #[test]
    fn init_failure_returns_error() {
        let bad_dir = Path::new("/dev/null/impossible");
        let claude_tmp = tempfile::tempdir().unwrap();
        let err = run_setup_result(bad_dir, claude_tmp.path()).unwrap_err();
        assert!(err.to_string().contains("failed to create"));
    }

    #[test]
    fn claude_home_missing_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        let err = run_setup_result(tmp.path(), Path::new("/nonexistent/claude")).unwrap_err();
        assert!(err.to_string().contains("does not exist"));
    }

    #[test]
    fn rerun_overwrites_skill_files() {
        let tmp = tempfile::tempdir().unwrap();
        let claude_tmp = tempfile::tempdir().unwrap();
        run_setup(tmp.path(), claude_tmp.path());

        let skill_md = paths::skill_dir(claude_tmp.path(), "leiter").join("SKILL.md");
        fs::write(&skill_md, "old content").unwrap();

        run_setup(tmp.path(), claude_tmp.path());

        let content = fs::read_to_string(skill_md).unwrap();
        assert_ne!(content, "old content");
    }

    #[test]
    fn install_removes_owned_legacy_skill_dirs_and_preserves_unowned() {
        let tmp = tempfile::tempdir().unwrap();
        let claude_tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(paths::skill_dir(claude_tmp.path(), "leiter-distill")).unwrap();
        fs::write(
            paths::skill_dir(claude_tmp.path(), "leiter-distill").join("SKILL.md"),
            format!("<!-- {PLUGIN_SENTINEL} -->\n"),
        )
        .unwrap();
        fs::create_dir_all(paths::skill_dir(claude_tmp.path(), "leiter-instill")).unwrap();
        fs::write(
            paths::skill_dir(claude_tmp.path(), "leiter-instill").join("SKILL.md"),
            "user content\n",
        )
        .unwrap();

        run_setup(tmp.path(), claude_tmp.path());

        assert!(!paths::skill_dir(claude_tmp.path(), "leiter-distill").exists());
        assert!(paths::skill_dir(claude_tmp.path(), "leiter-instill").exists());
        assert!(paths::skill_dir(claude_tmp.path(), "leiter").exists());
    }

    /// Install's confirmation now goes to an injected writer (SPEC install
    /// output). The message must name the skill, the `CLAUDE.md` path, and —
    /// when `codex = true` — the `AGENTS.md` path, so the user sees exactly
    /// what was written.
    #[test]
    fn install_output_names_skill_and_managed_block_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let claude_tmp = tempfile::tempdir().unwrap();
        let codex_tmp = tempfile::tempdir().unwrap();
        LeiterConfig { codex: true }
            .save(&paths::leiter_config_path(tmp.path()))
            .unwrap();

        let mut out = Vec::new();
        run(tmp.path(), claude_tmp.path(), codex_tmp.path(), &mut out).unwrap();
        let output = String::from_utf8(out).unwrap();

        assert!(output.contains("`leiter` skill"));
        assert!(
            output.contains(
                &paths::claude_md_path(claude_tmp.path())
                    .display()
                    .to_string()
            )
        );
        assert!(
            output.contains(
                &paths::agents_md_path(codex_tmp.path())
                    .display()
                    .to_string()
            )
        );
    }

    #[test]
    fn install_writes_managed_claude_block_and_sync_hashes() {
        let tmp = tempfile::tempdir().unwrap();
        let claude_tmp = tempfile::tempdir().unwrap();

        run_setup(tmp.path(), claude_tmp.path());

        let claude_md = fs::read_to_string(paths::claude_md_path(claude_tmp.path())).unwrap();
        assert!(claude_md.contains(crate::managed_block::BEGIN_SENTINEL));
        assert!(claude_md.contains("# Communication Style"));
        let state = LeiterState::load(&paths::state_path(tmp.path())).unwrap();
        assert!(state.sync.claude_md.is_some());
    }

    /// (C6a) With `codex = true` in the config, install materializes the
    /// AGENTS.md block into the Codex home and records its hashes.
    #[test]
    fn install_with_codex_writes_agents_block_and_hashes() {
        let tmp = tempfile::tempdir().unwrap();
        let claude_tmp = tempfile::tempdir().unwrap();
        let codex_tmp = tempfile::tempdir().unwrap();
        LeiterConfig { codex: true }
            .save(&paths::leiter_config_path(tmp.path()))
            .unwrap();

        run(
            tmp.path(),
            claude_tmp.path(),
            codex_tmp.path(),
            &mut Vec::new(),
        )
        .unwrap();

        let agents = fs::read_to_string(paths::agents_md_path(codex_tmp.path())).unwrap();
        assert!(agents.contains(crate::managed_block::BEGIN_SENTINEL));
        let state = LeiterState::load(&paths::state_path(tmp.path())).unwrap();
        assert!(state.sync.agents_md.is_some());
    }

    /// (C6b) Install uses force semantics (SPEC install step 6): a hand-tampered
    /// managed block is overwritten without error and the hashes are re-recorded.
    #[test]
    fn install_force_overwrites_hand_tampered_block() {
        let tmp = tempfile::tempdir().unwrap();
        let claude_tmp = tempfile::tempdir().unwrap();

        run_setup(tmp.path(), claude_tmp.path());
        let claude_md = paths::claude_md_path(claude_tmp.path());
        let tampered = fs::read_to_string(&claude_md)
            .unwrap()
            .replace("# Communication Style", "# TAMPERED HEADING");
        fs::write(&claude_md, tampered).unwrap();

        run_setup(tmp.path(), claude_tmp.path());

        let restored = fs::read_to_string(&claude_md).unwrap();
        assert!(restored.contains("# Communication Style"));
        assert!(!restored.contains("TAMPERED"));
        let state = LeiterState::load(&paths::state_path(tmp.path())).unwrap();
        assert!(state.sync.claude_md.is_some());
    }

    #[test]
    fn agent_setup_instructions_outputs_hook_commands() {
        let tmp = tempfile::tempdir().unwrap();
        let claude_tmp = tempfile::tempdir().unwrap();
        run_setup(tmp.path(), claude_tmp.path());

        let mut out = Vec::new();
        agent_setup_instructions(tmp.path(), &mut out).unwrap();
        let output = String::from_utf8(out).unwrap();
        assert!(output.contains("leiter hook context"));
        assert!(output.contains("leiter hook session-end"));
    }

    #[test]
    fn agent_setup_instructions_missing_soul_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let mut out = Vec::new();
        let result = agent_setup_instructions(tmp.path(), &mut out);
        assert!(result.is_err());
    }

    #[test]
    fn agent_setup_instructions_new_state_epoch_mismatch_errors() {
        let tmp = tempfile::tempdir().unwrap();
        crate::commands::test_support::write_state_with_epochs(
            tmp.path(),
            SETUP_SOFT_EPOCH,
            SETUP_HARD_EPOCH + 1,
        );

        let mut out = Vec::new();
        let err = agent_setup_instructions(tmp.path(), &mut out).unwrap_err();
        assert!(
            err.to_string()
                .contains("binary is older than your soul file")
        );
    }

    #[test]
    fn agent_setup_instructions_old_state_epoch_mismatch_errors() {
        let tmp = tempfile::tempdir().unwrap();
        crate::commands::test_support::write_state_with_epochs(
            tmp.path(),
            SETUP_SOFT_EPOCH,
            SETUP_HARD_EPOCH.saturating_sub(1),
        );

        let mut out = Vec::new();
        let err = agent_setup_instructions(tmp.path(), &mut out).unwrap_err();
        assert!(err.to_string().contains("leiter claude install"));
    }

    #[test]
    fn agent_setup_instructions_corrupt_state_errors() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path()).unwrap();
        fs::write(paths::soul_path(tmp.path()), "body\n").unwrap();
        fs::write(paths::state_path(tmp.path()), "not = valid = toml").unwrap();

        let mut out = Vec::new();
        let err = agent_setup_instructions(tmp.path(), &mut out).unwrap_err();
        assert!(err.to_string().contains("state file"));
    }

    #[test]
    fn rerun_with_plain_soul_succeeds() {
        let tmp = tempfile::tempdir().unwrap();
        let claude_tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        run_setup(dir, claude_tmp.path());

        let soul = paths::soul_path(dir);
        fs::write(&soul, "not frontmatter, just markdown").unwrap();

        run_setup_result(dir, claude_tmp.path()).unwrap();
        assert_eq!(
            fs::read_to_string(soul).unwrap(),
            "not frontmatter, just markdown"
        );
    }
}
