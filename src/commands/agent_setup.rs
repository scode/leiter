//! `leiter agent-setup` — first-time initialization and hook configuration instructions.
//!
//! Creates the `~/.leiter/` directory structure and initial soul file, then
//! prints instructions for the agent to wire up Claude Code hooks. If any
//! filesystem step fails, outputs a message for the agent to relay the error
//! to the user instead of exiting non-zero.

use std::fs;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};
use chrono::{DateTime, TimeZone, Utc};
use tracing::info;

use crate::frontmatter::{serialize_soul, SoulFrontmatter};
use crate::paths;
use crate::templates::{AGENT_SETUP_INSTRUCTIONS, SOUL_TEMPLATE, SOUL_TEMPLATE_VERSION};

/// Run the agent-setup command.
///
/// Creates directories and the initial soul file under `home`, then writes
/// setup instructions to `out`. If the filesystem steps fail, writes an error
/// relay message to `out` instead of returning an error — this way the agent
/// always gets actionable output.
pub fn run(home: &Path, out: &mut impl Write) -> Result<()> {
    if let Err(e) = init_filesystem(home) {
        write!(out, "leiter agent-setup failed during initialization:\n\n  {e:#}\n\nPlease relay this error to the user.\n")?;
        return Ok(());
    }

    write!(out, "{AGENT_SETUP_INSTRUCTIONS}")?;
    Ok(())
}

/// Deterministic filesystem initialization: create dirs and seed soul file.
fn init_filesystem(home: &Path) -> Result<()> {
    let leiter_dir = paths::leiter_dir(home);
    let logs_dir = paths::logs_dir(home);
    let soul_path = paths::soul_path(home);

    fs::create_dir_all(&leiter_dir)
        .with_context(|| format!("failed to create {}", leiter_dir.display()))?;
    fs::create_dir_all(&logs_dir)
        .with_context(|| format!("failed to create {}", logs_dir.display()))?;

    if !soul_path.exists() {
        let frontmatter = SoulFrontmatter {
            last_distilled: epoch(),
            soul_version: SOUL_TEMPLATE_VERSION,
        };
        let content = serialize_soul(&frontmatter, SOUL_TEMPLATE);
        fs::write(&soul_path, &content)
            .with_context(|| format!("failed to write {}", soul_path.display()))?;
        info!("created {}", soul_path.display());
    } else {
        info!("soul file already exists, skipping: {}", soul_path.display());
    }

    Ok(())
}

fn epoch() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(1970, 1, 1, 0, 0, 0).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontmatter::parse_soul;

    fn run_setup(home: &Path) -> String {
        let mut out = Vec::new();
        run(home, &mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn fresh_setup_creates_directories_and_soul() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        run_setup(home);

        assert!(paths::leiter_dir(home).is_dir());
        assert!(paths::logs_dir(home).is_dir());
        assert!(paths::soul_path(home).is_file());
    }

    #[test]
    fn soul_has_expected_frontmatter() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        run_setup(home);

        let content = fs::read_to_string(paths::soul_path(home)).unwrap();
        let (fm, _) = parse_soul(&content).unwrap();
        assert_eq!(fm.last_distilled, epoch());
        assert_eq!(fm.soul_version, SOUL_TEMPLATE_VERSION);
    }

    #[test]
    fn soul_body_matches_template() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        run_setup(home);

        let content = fs::read_to_string(paths::soul_path(home)).unwrap();
        let (_, body) = parse_soul(&content).unwrap();
        assert_eq!(body, SOUL_TEMPLATE);
    }

    #[test]
    fn running_twice_does_not_overwrite_soul() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        run_setup(home);

        // Modify the soul file to detect overwrites.
        let soul = paths::soul_path(home);
        fs::write(&soul, "modified").unwrap();

        run_setup(home);

        let content = fs::read_to_string(&soul).unwrap();
        assert_eq!(content, "modified");
    }

    #[test]
    fn running_twice_still_creates_missing_directories() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        run_setup(home);

        // Remove logs dir to simulate partial state.
        fs::remove_dir(paths::logs_dir(home)).unwrap();

        run_setup(home);

        assert!(paths::logs_dir(home).is_dir());
    }

    #[test]
    fn output_contains_setup_instructions() {
        let tmp = tempfile::tempdir().unwrap();
        let output = run_setup(tmp.path());
        assert!(output.contains("leiter context"));
        assert!(output.contains("leiter stop-hook"));
    }

    #[test]
    fn init_failure_outputs_error_relay_message() {
        // Use a non-existent path that can't be created to trigger a failure.
        let bad_home = Path::new("/dev/null/impossible");
        let output = run_setup(bad_home);
        assert!(output.contains("failed during initialization"));
        assert!(output.contains("relay this error"));
    }
}
