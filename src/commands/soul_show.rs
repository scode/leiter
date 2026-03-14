//! `leiter soul show` — output soul body wrapped in XML boundary tags.
//!
//! Validates the soul file (epoch checks), strips frontmatter, and
//! wraps the body in `<leiter-soul-content>` tags so the agent can
//! display it verbatim without interpreting the content as directives.

use std::io::Write;
use std::path::Path;

use anyhow::{Result, bail};

use crate::soul_validation::{SoulStatus, validate_soul};

/// Run the soul show command.
///
/// Validates the soul, then outputs the body (without frontmatter)
/// wrapped in XML boundary tags for safe verbatim display.
pub fn run(state_dir: &Path, out: &mut impl Write) -> Result<()> {
    let body = match validate_soul(state_dir) {
        SoulStatus::Incompatible(reason) => bail!("{}", reason.agent_message()),
        SoulStatus::Compatible { body, .. } => body,
    };

    writeln!(out, "<leiter-soul-content>")?;
    write!(out, "{body}")?;
    writeln!(out, "</leiter-soul-content>")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::test_support::setup_state_dir;
    use crate::frontmatter::{SoulFrontmatter, serialize_soul};
    use crate::paths;
    use crate::templates::{SETUP_HARD_EPOCH, SETUP_SOFT_EPOCH};
    use chrono::{TimeZone, Utc};
    use std::fs;

    fn run_show(state_dir: &Path) -> String {
        let mut out = Vec::new();
        run(state_dir, &mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn output_contains_xml_wrapper_tags() {
        let tmp = setup_state_dir();
        let output = run_show(tmp.path());
        assert!(output.contains("<leiter-soul-content>"));
        assert!(output.contains("</leiter-soul-content>"));
    }

    #[test]
    fn output_contains_soul_body() {
        let tmp = setup_state_dir();
        let output = run_show(tmp.path());
        assert!(output.contains("# Communication Style"));
        assert!(output.contains("# Coding Preferences"));
    }

    #[test]
    fn output_does_not_contain_frontmatter() {
        let tmp = setup_state_dir();
        let output = run_show(tmp.path());
        assert!(!output.contains("last_distilled"));
        assert!(!output.contains("setup_soft_epoch"));
        assert!(!output.contains("setup_hard_epoch"));
    }

    #[test]
    fn missing_soul_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let mut out = Vec::new();
        let result = run(tmp.path(), &mut out);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not initialized"));
    }

    fn write_soul_with_epochs(state_dir: &Path, soft: u32, hard: u32) {
        let fm = SoulFrontmatter {
            last_distilled: Utc.with_ymd_and_hms(1970, 1, 1, 0, 0, 0).unwrap(),
            soul_version: 2,
            setup_soft_epoch: soft,
            setup_hard_epoch: hard,
        };
        let soul = serialize_soul(&fm, "body\n");
        fs::create_dir_all(state_dir).unwrap();
        fs::write(paths::soul_path(state_dir), soul).unwrap();
    }

    #[test]
    fn hard_epoch_mismatch_errors() {
        let tmp = tempfile::tempdir().unwrap();
        write_soul_with_epochs(tmp.path(), SETUP_SOFT_EPOCH, SETUP_HARD_EPOCH + 1);

        let mut out = Vec::new();
        let err = run(tmp.path(), &mut out).unwrap_err();
        assert!(
            err.to_string()
                .contains("binary is older than your soul file")
        );
    }

    #[test]
    fn corrupt_frontmatter_errors() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path()).unwrap();
        fs::write(paths::soul_path(tmp.path()), "not frontmatter").unwrap();

        let mut out = Vec::new();
        let err = run(tmp.path(), &mut out).unwrap_err();
        assert!(err.to_string().contains("invalid YAML"));
    }
}
