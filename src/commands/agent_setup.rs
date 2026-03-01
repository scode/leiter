//! `leiter claude install` — first-time initialization and plugin file installation.
//!
//! Creates the leiter state directory structure and initial soul file, writes
//! skill files to `~/.claude/skills/`, then prints a success message listing
//! available skills.

use std::fs;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, TimeZone, Utc};
use tracing::info;

use crate::frontmatter::{SoulFrontmatter, parse_soul, serialize_soul};
use crate::paths;
use crate::templates::{
    SETUP_HARD_EPOCH, SETUP_SOFT_EPOCH, SKILL_CONTENTS, SOUL_TEMPLATE, SOUL_TEMPLATE_VERSION,
};

/// Run the `leiter claude install` command.
///
/// Creates directories and the initial soul file under `state_dir`, writes
/// skill files under `claude_home`, then outputs a success message listing
/// available skills.
pub fn run(state_dir: &Path, claude_home: &Path) -> Result<()> {
    init_filesystem(state_dir)?;

    if !claude_home.is_dir() {
        bail!(
            "`{}` does not exist. Is Claude Code installed?",
            claude_home.display()
        );
    }

    write_plugin_files(claude_home)?;

    info!("Leiter installed successfully");
    info!("Available skills:");
    info!("  /leiter-setup     — configure Claude Code hooks");
    info!("  /leiter-distill   — distill session logs into the soul");
    info!("  /leiter-instill   — record a preference in the soul");
    info!("  /leiter-teardown  — remove leiter hooks");
    info!("Start a new Claude Code session and run /leiter-setup to configure hooks");

    Ok(())
}

/// Output the agent-setup instructions (hooks and permissions).
///
/// Used by `leiter claude agent-setup-instructions`.
pub fn agent_setup_instructions(state_dir: &Path, out: &mut impl Write) -> Result<()> {
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

    fs::create_dir_all(state_dir)
        .with_context(|| format!("failed to create {}", state_dir.display()))?;
    fs::create_dir_all(&logs_dir)
        .with_context(|| format!("failed to create {}", logs_dir.display()))?;

    if !soul_path.exists() {
        let frontmatter = SoulFrontmatter {
            last_distilled: epoch(),
            soul_version: SOUL_TEMPLATE_VERSION,
            setup_soft_epoch: SETUP_SOFT_EPOCH,
            setup_hard_epoch: SETUP_HARD_EPOCH,
        };
        let content = serialize_soul(&frontmatter, SOUL_TEMPLATE);
        fs::write(&soul_path, &content)
            .with_context(|| format!("failed to write {}", soul_path.display()))?;
        info!("created {}", soul_path.display());
    } else {
        info!("soul file already exists, updating epochs");
        update_epochs(&soul_path)?;
    }

    Ok(())
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

/// Update setup epoch fields in an existing soul file's frontmatter.
///
/// Preserves all other frontmatter fields and the body. If the frontmatter
/// can't be parsed, silently skips — a corrupt soul shouldn't block setup.
fn update_epochs(soul_path: &Path) -> Result<()> {
    let content = fs::read_to_string(soul_path)
        .with_context(|| format!("failed to read {}", soul_path.display()))?;

    let (mut fm, body) = match parse_soul(&content) {
        Ok(parsed) => parsed,
        Err(e) => {
            info!("skipping epoch update, frontmatter unparseable: {e}");
            return Ok(());
        }
    };

    if fm.setup_soft_epoch == SETUP_SOFT_EPOCH && fm.setup_hard_epoch == SETUP_HARD_EPOCH {
        info!("epochs already current");
        return Ok(());
    }

    fm.setup_soft_epoch = SETUP_SOFT_EPOCH;
    fm.setup_hard_epoch = SETUP_HARD_EPOCH;
    let updated = serialize_soul(&fm, body);
    fs::write(soul_path, &updated)
        .with_context(|| format!("failed to write {}", soul_path.display()))?;
    info!("updated epochs to soft={SETUP_SOFT_EPOCH}, hard={SETUP_HARD_EPOCH}");

    Ok(())
}

fn epoch() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(1970, 1, 1, 0, 0, 0).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontmatter::{parse_soul, serialize_soul};
    use crate::templates::SKILL_CONTENTS;

    /// Return everything after the closing `---\n` frontmatter delimiter.
    fn raw_body(content: &str) -> &str {
        let after_opening = content.strip_prefix("---\n").unwrap();
        let (_, body) = after_opening.split_once("\n---\n").unwrap();
        body
    }

    fn run_setup(state_dir: &Path, claude_home: &Path) {
        run(state_dir, claude_home).unwrap();
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
    fn soul_has_expected_frontmatter() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let _claude_tmp = run_setup_with_claude_home(dir);

        let content = fs::read_to_string(paths::soul_path(dir)).unwrap();
        let (fm, _) = parse_soul(&content).unwrap();
        assert_eq!(fm.last_distilled, epoch());
        assert_eq!(fm.soul_version, SOUL_TEMPLATE_VERSION);
    }

    #[test]
    fn soul_body_matches_template() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let _claude_tmp = run_setup_with_claude_home(dir);

        let content = fs::read_to_string(paths::soul_path(dir)).unwrap();
        let (_, body) = parse_soul(&content).unwrap();
        assert_eq!(body, SOUL_TEMPLATE);
    }

    #[test]
    fn running_twice_does_not_overwrite_soul() {
        let tmp = tempfile::tempdir().unwrap();
        let claude_tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        run_setup(dir, claude_tmp.path());

        let soul = paths::soul_path(dir);
        fs::write(&soul, "modified").unwrap();

        run_setup(dir, claude_tmp.path());

        let content = fs::read_to_string(&soul).unwrap();
        assert_eq!(content, "modified");
    }

    #[test]
    fn rerun_updates_stale_epochs() {
        let tmp = tempfile::tempdir().unwrap();
        let claude_tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        run_setup(dir, claude_tmp.path());

        let soul = paths::soul_path(dir);
        let content = fs::read_to_string(&soul).unwrap();
        let (mut fm, body) = parse_soul(&content).unwrap();

        fm.setup_soft_epoch = 0;
        fm.setup_hard_epoch = 0;
        fs::write(&soul, serialize_soul(&fm, body)).unwrap();
        let before_body = raw_body(&fs::read_to_string(&soul).unwrap()).to_owned();

        run_setup(dir, claude_tmp.path());

        let updated = fs::read_to_string(&soul).unwrap();
        let (updated_fm, _) = parse_soul(&updated).unwrap();
        assert_eq!(updated_fm.setup_soft_epoch, SETUP_SOFT_EPOCH);
        assert_eq!(updated_fm.setup_hard_epoch, SETUP_HARD_EPOCH);
        assert_eq!(updated_fm.last_distilled, fm.last_distilled);
        assert_eq!(updated_fm.soul_version, fm.soul_version);
        assert_eq!(raw_body(&updated), before_body);
    }

    #[test]
    fn rerun_preserves_modified_body() {
        let tmp = tempfile::tempdir().unwrap();
        let claude_tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        run_setup(dir, claude_tmp.path());

        let soul = paths::soul_path(dir);
        let content = fs::read_to_string(&soul).unwrap();
        let (mut fm, _) = parse_soul(&content).unwrap();
        fm.setup_soft_epoch = 0;
        let custom_body = "# My customized soul\n\nLearned preferences here.\n";
        fs::write(&soul, serialize_soul(&fm, custom_body)).unwrap();

        run_setup(dir, claude_tmp.path());

        let updated = fs::read_to_string(&soul).unwrap();
        let (updated_fm, _) = parse_soul(&updated).unwrap();
        assert_eq!(updated_fm.setup_soft_epoch, SETUP_SOFT_EPOCH);
        assert_eq!(raw_body(&updated), custom_body);
    }

    #[test]
    fn rerun_with_unparseable_frontmatter_does_not_error() {
        let tmp = tempfile::tempdir().unwrap();
        let claude_tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        run_setup(dir, claude_tmp.path());

        let soul = paths::soul_path(dir);
        fs::write(&soul, "---\ngarbage: true\n---\nbody\n").unwrap();

        run_setup(dir, claude_tmp.path());
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
        let err = run(bad_dir, claude_tmp.path()).unwrap_err();
        assert!(err.to_string().contains("failed to create"));
    }

    #[test]
    fn claude_home_missing_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        let err = run(tmp.path(), Path::new("/nonexistent/claude")).unwrap_err();
        assert!(err.to_string().contains("does not exist"));
    }

    #[test]
    fn rerun_overwrites_skill_files() {
        let tmp = tempfile::tempdir().unwrap();
        let claude_tmp = tempfile::tempdir().unwrap();
        run_setup(tmp.path(), claude_tmp.path());

        let skill_md = paths::skill_dir(claude_tmp.path(), "leiter-setup").join("SKILL.md");
        fs::write(&skill_md, "old content").unwrap();

        run_setup(tmp.path(), claude_tmp.path());

        let content = fs::read_to_string(skill_md).unwrap();
        assert_ne!(content, "old content");
    }

    #[test]
    fn agent_setup_instructions_outputs_hook_commands() {
        let tmp = tempfile::tempdir().unwrap();
        let mut out = Vec::new();
        agent_setup_instructions(tmp.path(), &mut out).unwrap();
        let output = String::from_utf8(out).unwrap();
        assert!(output.contains("leiter hook context"));
        assert!(output.contains("leiter hook session-end"));
    }
}
