//! `leiter soul show` — output soul body wrapped in XML boundary tags.
//!
//! Validates state and wraps the full soul in `<leiter-soul-content>` tags so the agent can
//! display it verbatim without interpreting the content as directives.

use std::io::Write;
use std::path::Path;

use anyhow::{Result, bail};

use crate::validation::{ValidationStatus, validate_state};

/// Run the soul show command.
///
/// Validates state, then outputs the whole soul
/// wrapped in XML boundary tags for safe verbatim display.
pub fn run(state_dir: &Path, out: &mut impl Write) -> Result<()> {
    let soul = match validate_state(state_dir) {
        ValidationStatus::Incompatible(reason) => bail!("{}", reason.agent_message()),
        ValidationStatus::Compatible { soul, .. } => soul,
    };

    writeln!(out, "<leiter-soul-content>")?;
    write!(out, "{soul}")?;
    writeln!(out, "</leiter-soul-content>")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::test_support::{setup_state_dir, write_state_with_epochs};
    use crate::paths;
    use crate::templates::{SETUP_HARD_EPOCH, SETUP_SOFT_EPOCH};
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

    #[test]
    fn hard_epoch_mismatch_errors() {
        let tmp = tempfile::tempdir().unwrap();
        write_state_with_epochs(tmp.path(), SETUP_SOFT_EPOCH, SETUP_HARD_EPOCH + 1);

        let mut out = Vec::new();
        let err = run(tmp.path(), &mut out).unwrap_err();
        assert!(
            err.to_string()
                .contains("binary is older than your soul file")
        );
    }

    #[test]
    fn corrupt_state_errors() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path()).unwrap();
        fs::write(paths::soul_path(tmp.path()), "body\n").unwrap();
        fs::write(paths::state_path(tmp.path()), "not = valid = toml").unwrap();

        let mut out = Vec::new();
        let err = run(tmp.path(), &mut out).unwrap_err();
        assert!(err.to_string().contains("state file"));
    }
}
