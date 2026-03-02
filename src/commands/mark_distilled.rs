//! `leiter soul mark-distilled` — set `last_distilled` to the current time.
//!
//! Deterministically updates the soul frontmatter timestamp so the agent
//! never has to edit `last_distilled` by hand. This avoids imprecise
//! timestamps caused by agent rounding.

use std::io::Write;
use std::path::Path;

use anyhow::{Result, bail};
use chrono::{SubsecRound, Utc};

use crate::frontmatter::serialize_soul;
use crate::paths;
use crate::soul_validation::{SoulStatus, validate_soul};

pub fn run(state_dir: &Path, out: &mut impl Write) -> Result<()> {
    let soul_path = paths::soul_path(state_dir);

    let (mut fm, body) = match validate_soul(state_dir) {
        SoulStatus::Incompatible(reason) => bail!("{}", reason.agent_message()),
        SoulStatus::Compatible {
            frontmatter, body, ..
        } => (frontmatter, body),
    };

    fm.last_distilled = Utc::now().trunc_subsecs(0);
    std::fs::write(&soul_path, serialize_soul(&fm, &body))?;

    writeln!(
        out,
        "last_distilled set to {}",
        fm.last_distilled
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::test_support::{bytes_to_string, setup_state_dir};
    use crate::frontmatter::{SoulFrontmatter, parse_soul};
    use crate::templates::{SETUP_HARD_EPOCH, SETUP_SOFT_EPOCH};
    use chrono::{SubsecRound, TimeZone, Utc};
    use std::fs;

    fn run_mark_distilled(state_dir: &Path) -> String {
        let mut out = Vec::new();
        run(state_dir, &mut out).unwrap();
        bytes_to_string(out)
    }

    #[test]
    fn sets_last_distilled_to_approximately_now() {
        let tmp = setup_state_dir();
        let before = Utc::now().trunc_subsecs(0);
        run_mark_distilled(tmp.path());
        let after = Utc::now();

        let content = fs::read_to_string(paths::soul_path(tmp.path())).unwrap();
        let (fm, _) = parse_soul(&content).unwrap();
        assert!(fm.last_distilled >= before);
        assert!(fm.last_distilled <= after);
    }

    #[test]
    fn preserves_soul_body_bytes() {
        let tmp = setup_state_dir();
        let soul_path = paths::soul_path(tmp.path());

        let original = fs::read_to_string(&soul_path).unwrap();
        let (_, original_body) = parse_soul(&original).unwrap();

        run_mark_distilled(tmp.path());

        let updated = fs::read_to_string(&soul_path).unwrap();
        let (_, updated_body) = parse_soul(&updated).unwrap();
        assert_eq!(updated_body, original_body);
    }

    #[test]
    fn preserves_other_frontmatter_fields() {
        let tmp = setup_state_dir();
        let soul_path = paths::soul_path(tmp.path());

        let original = fs::read_to_string(&soul_path).unwrap();
        let (original_fm, _) = parse_soul(&original).unwrap();

        run_mark_distilled(tmp.path());

        let updated = fs::read_to_string(&soul_path).unwrap();
        let (updated_fm, _) = parse_soul(&updated).unwrap();
        assert_eq!(updated_fm.soul_version, original_fm.soul_version);
        assert_eq!(updated_fm.setup_soft_epoch, original_fm.setup_soft_epoch);
        assert_eq!(updated_fm.setup_hard_epoch, original_fm.setup_hard_epoch);
    }

    #[test]
    fn missing_soul_file_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let mut out = Vec::new();
        let result = run(tmp.path(), &mut out);
        assert!(result.is_err());
    }

    #[test]
    fn outputs_confirmation_with_timestamp() {
        let tmp = setup_state_dir();
        let output = run_mark_distilled(tmp.path());
        assert!(output.starts_with("last_distilled set to "));
    }

    #[test]
    fn confirmation_matches_stored_value() {
        let tmp = setup_state_dir();
        let output = run_mark_distilled(tmp.path());
        let displayed_ts = output
            .trim()
            .strip_prefix("last_distilled set to ")
            .unwrap();

        let content = fs::read_to_string(paths::soul_path(tmp.path())).unwrap();
        let (fm, _) = parse_soul(&content).unwrap();
        let stored_ts = fm
            .last_distilled
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        assert_eq!(displayed_ts, stored_ts);
    }

    #[test]
    fn malformed_frontmatter_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let soul_path = paths::soul_path(tmp.path());
        fs::write(&soul_path, "---\nnot: valid: frontmatter\n---\nbody\n").unwrap();

        let mut out = Vec::new();
        let err = run(tmp.path(), &mut out).unwrap_err();
        assert!(err.to_string().contains("invalid YAML"));
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
    fn hard_epoch_mismatch_new_soul_errors() {
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
    fn hard_epoch_mismatch_old_soul_errors() {
        let tmp = tempfile::tempdir().unwrap();
        write_soul_with_epochs(
            tmp.path(),
            SETUP_SOFT_EPOCH,
            SETUP_HARD_EPOCH.saturating_sub(1),
        );

        let mut out = Vec::new();
        let err = run(tmp.path(), &mut out).unwrap_err();
        assert!(err.to_string().contains("leiter claude install"));
    }
}
