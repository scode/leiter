//! `leiter soul-upgrade` — detect soul template drift and output migration instructions.
//!
//! Compares the `soul_version` in the user's soul file against the current
//! template version built into the binary. When outdated, outputs a changelog,
//! the current template, and instructions so the agent can restructure the soul
//! while preserving learned preferences.

use std::fs;
use std::io::Write;
use std::path::Path;

use anyhow::Result;

use crate::errors::LeiterError;
use crate::frontmatter::parse_soul;
use crate::paths;
use crate::templates::{
    SOUL_TEMPLATE, SOUL_TEMPLATE_CHANGELOG, SOUL_TEMPLATE_VERSION, SOUL_UPGRADE_INSTRUCTIONS,
};

/// Run the soul-upgrade command.
///
/// Reads the soul file's `soul_version` and compares it to the binary's
/// built-in template version. If up to date, says so. If outdated, outputs
/// the changelog of intervening versions, the full current template, and
/// migration instructions for the agent to follow.
pub fn run(home: &Path, out: &mut impl Write) -> Result<()> {
    let soul_path = paths::soul_path(home);
    let content = fs::read_to_string(&soul_path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            LeiterError::SoulNotFound.into()
        } else {
            anyhow::anyhow!("failed to read {}: {e}", soul_path.display())
        }
    })?;
    let (fm, _) = parse_soul(&content)?;

    if fm.soul_version >= SOUL_TEMPLATE_VERSION {
        writeln!(out, "Soul is up to date (version {}).", fm.soul_version)?;
        return Ok(());
    }

    writeln!(
        out,
        "Soul version {} is outdated (current: {}).\n",
        fm.soul_version, SOUL_TEMPLATE_VERSION
    )?;

    writeln!(out, "## Changelog\n")?;
    for &(version, description) in SOUL_TEMPLATE_CHANGELOG {
        if version > fm.soul_version && version <= SOUL_TEMPLATE_VERSION {
            writeln!(out, "**Version {version}:** {description}\n")?;
        }
    }

    writeln!(
        out,
        "## Current template (version {SOUL_TEMPLATE_VERSION})\n"
    )?;
    write!(out, "{SOUL_TEMPLATE}")?;
    if !SOUL_TEMPLATE.ends_with('\n') {
        writeln!(out)?;
    }
    writeln!(out)?;

    write!(out, "{SOUL_UPGRADE_INSTRUCTIONS}")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::agent_setup;
    use crate::frontmatter::serialize_soul;

    fn setup_home() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        agent_setup::run(tmp.path(), &mut Vec::new()).unwrap();
        tmp
    }

    fn run_upgrade(home: &Path) -> String {
        let mut out = Vec::new();
        run(home, &mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    fn set_soul_version(home: &Path, version: u32) {
        let soul_path = paths::soul_path(home);
        let content = fs::read_to_string(&soul_path).unwrap();
        let (mut fm, body) = parse_soul(&content).unwrap();
        fm.soul_version = version;
        fs::write(&soul_path, serialize_soul(&fm, body)).unwrap();
    }

    #[test]
    fn up_to_date_reports_current() {
        let tmp = setup_home();
        let output = run_upgrade(tmp.path());
        assert!(output.contains("up to date"));
    }

    #[test]
    fn outdated_includes_changelog() {
        let tmp = setup_home();
        set_soul_version(tmp.path(), 0);

        let output = run_upgrade(tmp.path());
        assert!(output.contains("Changelog"));
        assert!(output.contains("Version 1"));
        assert!(output.contains("Version 2"));
    }

    #[test]
    fn outdated_includes_template() {
        let tmp = setup_home();
        set_soul_version(tmp.path(), 0);

        let output = run_upgrade(tmp.path());
        assert!(output.contains("Current template"));
        assert!(output.contains("# Technology & Environment"));
    }

    #[test]
    fn outdated_includes_migration_instructions() {
        let tmp = setup_home();
        set_soul_version(tmp.path(), 0);

        let output = run_upgrade(tmp.path());
        assert!(output.contains("soul_version"));
        assert!(output.contains("Migration instructions"));
    }

    #[test]
    fn missing_soul_errors() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(paths::logs_dir(tmp.path())).unwrap();

        let mut out = Vec::new();
        let result = run(tmp.path(), &mut out);
        assert!(result.is_err());
    }
}
