//! Shared soul file validation — epoch and frontmatter checks.
//!
//! Every command except `session-end` calls [`validate_soul`] before doing
//! work. This single function is the sole implementation of epoch and
//! frontmatter checks, preventing drift between commands.

use std::fs;
use std::path::{Path, PathBuf};

use crate::frontmatter::{SoulFrontmatter, parse_soul};
use crate::paths;
use crate::templates::{SETUP_HARD_EPOCH, SETUP_SOFT_EPOCH};

/// Result of validating the soul file against the current binary.
pub enum SoulStatus {
    /// Soul is compatible. May include a soft epoch nudge.
    Compatible {
        frontmatter: SoulFrontmatter,
        body: String,
        /// Full file content, needed by `context` to output verbatim.
        raw_content: String,
        /// Present when soft epochs don't match (nudge, not blocking).
        soft_nudge: Option<String>,
    },
    /// Soul is missing, corrupt, or has a hard epoch mismatch.
    Incompatible(SoulIncompatibility),
}

/// Why the soul is incompatible with this binary.
pub enum SoulIncompatibility {
    /// Soul's hard epoch is lower than the binary's — setup was not re-run
    /// after a binary upgrade.
    SetupOutdated { soul_epoch: u32, binary_epoch: u32 },
    /// Soul's hard epoch is higher than the binary's — the binary is older
    /// than the soul.
    BinaryOutdated { soul_epoch: u32, binary_epoch: u32 },
    /// Frontmatter could not be parsed.
    FrontmatterCorrupt { soul_path: PathBuf, error: String },
    /// Soul file exists but could not be read (permissions, I/O error, etc.).
    SoulUnreadable { soul_path: PathBuf, error: String },
    /// Soul file does not exist.
    SoulNotFound,
}

impl SoulIncompatibility {
    /// Agent-facing error message for this incompatibility.
    pub fn agent_message(&self) -> String {
        match self {
            Self::SetupOutdated {
                soul_epoch,
                binary_epoch,
            } => format!(
                "ACTION REQUIRED: Leiter setup is incompatible \
                 (setup_hard_epoch: soul={soul_epoch}, binary={binary_epoch}). \
                 The binary was upgraded but setup was not re-run. \
                 Before responding to the user's first message, tell them \
                 EXACTLY this (word for word): \
                 \"Leiter setup needs to be re-run \u{2014} please run \
                 `leiter claude install` in your terminal and follow the \
                 instructions, then start a new session.\" \
                 Do not attempt to use leiter commands in this session."
            ),
            Self::BinaryOutdated {
                soul_epoch,
                binary_epoch,
            } => format!(
                "ACTION REQUIRED: Leiter setup is incompatible \
                 (setup_hard_epoch: soul={soul_epoch}, binary={binary_epoch}). \
                 The soul was created by a newer leiter binary than the one \
                 currently installed. Before responding to the user's first \
                 message, tell them EXACTLY this (word for word): \
                 \"Your leiter binary is older than your soul file expects \
                 \u{2014} please upgrade leiter, then start a new session.\" \
                 Do not attempt to use leiter commands in this session."
            ),
            Self::FrontmatterCorrupt { soul_path, error } => format!(
                "ACTION REQUIRED: The leiter soul ({}) has invalid YAML \
                 front matter ({error}) and leiter cannot verify compatibility. \
                 Before responding to the user's first message, tell them \
                 EXACTLY this (word for word): \
                 \"The leiter soul has corrupt frontmatter. Please fix \
                 the YAML front matter manually, or delete the soul file and \
                 run `leiter claude install` to start fresh, then start a new \
                 session.\" \
                 Do not attempt to use leiter commands in this session.",
                soul_path.display()
            ),
            Self::SoulUnreadable { soul_path, error } => format!(
                "ACTION REQUIRED: The leiter soul ({}) could not be read \
                 ({error}). Before responding to the user's first message, \
                 tell them EXACTLY this (word for word): \
                 \"The leiter soul file could not be read. \
                 Please check file permissions on {}, then start a new \
                 session.\" \
                 Do not attempt to use leiter commands in this session.",
                soul_path.display(),
                soul_path.display()
            ),
            Self::SoulNotFound => {
                "Leiter is not initialized. Run `leiter claude install` to set up.".to_string()
            }
        }
    }

    /// Human-facing error message for user-invoked CLI commands.
    pub fn user_message(&self) -> String {
        match self {
            Self::SetupOutdated {
                soul_epoch,
                binary_epoch,
            } => format!(
                "leiter setup is incompatible \
                 (setup_hard_epoch: soul={soul_epoch}, binary={binary_epoch}). \
                 Run `leiter claude install` to update, then start a new session."
            ),
            Self::BinaryOutdated {
                soul_epoch,
                binary_epoch,
            } => format!(
                "leiter binary is outdated \
                 (setup_hard_epoch: soul={soul_epoch}, binary={binary_epoch}). \
                 Please upgrade leiter, then start a new session."
            ),
            Self::FrontmatterCorrupt { soul_path, error } => format!(
                "leiter soul ({}) has invalid YAML front matter ({error}). \
                 Fix the front matter manually, or delete the soul file and \
                 run `leiter claude install` to start fresh.",
                soul_path.display()
            ),
            Self::SoulUnreadable { soul_path, error } => format!(
                "leiter soul ({}) could not be read ({error}). \
                 Check file permissions and try again.",
                soul_path.display()
            ),
            Self::SoulNotFound => {
                "leiter is not initialized. Run `leiter claude install` to set up.".to_string()
            }
        }
    }
}

impl std::fmt::Display for SoulIncompatibility {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.user_message())
    }
}

/// Validate the soul file against this binary's epoch expectations.
///
/// Returns [`SoulStatus::Compatible`] when the soul exists, has valid
/// frontmatter, and hard epochs match. A soft epoch mismatch is reported
/// via `soft_nudge` but is not blocking.
///
/// Returns [`SoulStatus::Incompatible`] when the soul is missing, has
/// corrupt frontmatter, or has a hard epoch mismatch.
pub fn validate_soul(state_dir: &Path) -> SoulStatus {
    let soul_path = paths::soul_path(state_dir);

    let raw_content = match fs::read_to_string(&soul_path) {
        Ok(content) => content,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return SoulStatus::Incompatible(SoulIncompatibility::SoulNotFound);
        }
        Err(e) => {
            return SoulStatus::Incompatible(SoulIncompatibility::SoulUnreadable {
                soul_path,
                error: e.to_string(),
            });
        }
    };

    let (fm, body) = match parse_soul(&raw_content) {
        Ok((fm, body)) => (fm, body.to_string()),
        Err(e) => {
            return SoulStatus::Incompatible(SoulIncompatibility::FrontmatterCorrupt {
                soul_path,
                error: e.to_string(),
            });
        }
    };

    if fm.setup_hard_epoch < SETUP_HARD_EPOCH {
        return SoulStatus::Incompatible(SoulIncompatibility::SetupOutdated {
            soul_epoch: fm.setup_hard_epoch,
            binary_epoch: SETUP_HARD_EPOCH,
        });
    }
    if fm.setup_hard_epoch > SETUP_HARD_EPOCH {
        return SoulStatus::Incompatible(SoulIncompatibility::BinaryOutdated {
            soul_epoch: fm.setup_hard_epoch,
            binary_epoch: SETUP_HARD_EPOCH,
        });
    }

    let soft_nudge = if fm.setup_soft_epoch < SETUP_SOFT_EPOCH {
        Some(format!(
            "Before responding to the user's first message, briefly mention \
             that leiter setup is slightly behind the binary \
             (setup_soft_epoch: soul={}, binary={SETUP_SOFT_EPOCH}). \
             Suggest they run `leiter claude install` when convenient. \
             Keep it to one short sentence.",
            fm.setup_soft_epoch,
        ))
    } else if fm.setup_soft_epoch > SETUP_SOFT_EPOCH {
        Some(format!(
            "Before responding to the user's first message, briefly mention \
             that the leiter binary is slightly behind the setup \
             (setup_soft_epoch: soul={}, binary={SETUP_SOFT_EPOCH}). \
             Suggest they upgrade leiter when convenient. \
             Keep it to one short sentence.",
            fm.setup_soft_epoch,
        ))
    } else {
        None
    };

    SoulStatus::Compatible {
        frontmatter: fm,
        body,
        raw_content,
        soft_nudge,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::test_support::setup_state_dir;
    use crate::frontmatter::{SoulFrontmatter, serialize_soul};
    use chrono::{TimeZone, Utc};

    fn write_soul_with_epochs(state_dir: &Path, soft: u32, hard: u32) {
        let fm = SoulFrontmatter {
            last_distilled: Utc.with_ymd_and_hms(1970, 1, 1, 0, 0, 0).unwrap(),
            soul_version: 2,
            setup_soft_epoch: soft,
            setup_hard_epoch: hard,
        };
        let soul = serialize_soul(&fm, "body\n");
        std::fs::create_dir_all(state_dir).unwrap();
        std::fs::write(paths::soul_path(state_dir), soul).unwrap();
    }

    #[test]
    fn missing_soul_returns_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        match validate_soul(tmp.path()) {
            SoulStatus::Incompatible(SoulIncompatibility::SoulNotFound) => {}
            other => panic!("expected SoulNotFound, got {}", status_label(&other)),
        }
    }

    #[test]
    fn corrupt_frontmatter_returns_corrupt() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path()).unwrap();
        std::fs::write(paths::soul_path(tmp.path()), "not frontmatter\n").unwrap();
        match validate_soul(tmp.path()) {
            SoulStatus::Incompatible(SoulIncompatibility::FrontmatterCorrupt { .. }) => {}
            other => panic!("expected FrontmatterCorrupt, got {}", status_label(&other)),
        }
    }

    #[test]
    fn hard_epoch_less_returns_setup_outdated() {
        let tmp = tempfile::tempdir().unwrap();
        write_soul_with_epochs(
            tmp.path(),
            SETUP_SOFT_EPOCH,
            SETUP_HARD_EPOCH.saturating_sub(1),
        );
        match validate_soul(tmp.path()) {
            SoulStatus::Incompatible(SoulIncompatibility::SetupOutdated { .. }) => {}
            other => panic!("expected SetupOutdated, got {}", status_label(&other)),
        }
    }

    #[test]
    fn hard_epoch_greater_returns_binary_outdated() {
        let tmp = tempfile::tempdir().unwrap();
        write_soul_with_epochs(tmp.path(), SETUP_SOFT_EPOCH, SETUP_HARD_EPOCH + 1);
        match validate_soul(tmp.path()) {
            SoulStatus::Incompatible(SoulIncompatibility::BinaryOutdated { .. }) => {}
            other => panic!("expected BinaryOutdated, got {}", status_label(&other)),
        }
    }

    #[test]
    fn matching_epochs_returns_compatible() {
        let tmp = setup_state_dir();
        match validate_soul(tmp.path()) {
            SoulStatus::Compatible { soft_nudge, .. } => {
                assert!(soft_nudge.is_none());
            }
            other => panic!("expected Compatible, got {}", status_label(&other)),
        }
    }

    #[test]
    fn soft_epoch_less_returns_nudge() {
        let tmp = tempfile::tempdir().unwrap();
        write_soul_with_epochs(
            tmp.path(),
            SETUP_SOFT_EPOCH.saturating_sub(1),
            SETUP_HARD_EPOCH,
        );
        match validate_soul(tmp.path()) {
            SoulStatus::Compatible { soft_nudge, .. } => {
                let nudge = soft_nudge.expect("expected soft nudge");
                assert!(nudge.contains("setup is slightly behind"));
            }
            other => panic!(
                "expected Compatible with nudge, got {}",
                status_label(&other)
            ),
        }
    }

    #[test]
    fn soft_epoch_greater_returns_nudge() {
        let tmp = tempfile::tempdir().unwrap();
        write_soul_with_epochs(tmp.path(), SETUP_SOFT_EPOCH + 1, SETUP_HARD_EPOCH);
        match validate_soul(tmp.path()) {
            SoulStatus::Compatible { soft_nudge, .. } => {
                let nudge = soft_nudge.expect("expected soft nudge");
                assert!(nudge.contains("binary is slightly behind"));
            }
            other => panic!(
                "expected Compatible with nudge, got {}",
                status_label(&other)
            ),
        }
    }

    #[test]
    fn compatible_includes_raw_content() {
        let tmp = setup_state_dir();
        let expected = std::fs::read_to_string(paths::soul_path(tmp.path())).unwrap();
        match validate_soul(tmp.path()) {
            SoulStatus::Compatible { raw_content, .. } => {
                assert_eq!(raw_content, expected);
            }
            other => panic!("expected Compatible, got {}", status_label(&other)),
        }
    }

    #[test]
    fn agent_message_setup_outdated_contains_install() {
        let msg = SoulIncompatibility::SetupOutdated {
            soul_epoch: 0,
            binary_epoch: 1,
        }
        .agent_message();
        assert!(msg.contains("leiter claude install"));
        assert!(msg.contains("ACTION REQUIRED"));
    }

    #[test]
    fn agent_message_binary_outdated_contains_upgrade() {
        let msg = SoulIncompatibility::BinaryOutdated {
            soul_epoch: 2,
            binary_epoch: 1,
        }
        .agent_message();
        assert!(msg.contains("binary is older than your soul file"));
        assert!(msg.contains("ACTION REQUIRED"));
    }

    #[test]
    fn agent_message_corrupt_contains_warning() {
        let msg = SoulIncompatibility::FrontmatterCorrupt {
            soul_path: PathBuf::from("/test/soul.md"),
            error: "bad yaml".to_string(),
        }
        .agent_message();
        assert!(msg.contains("invalid YAML"));
        assert!(msg.contains("ACTION REQUIRED"));
        assert!(msg.contains("/test/soul.md"));
    }

    #[test]
    fn agent_message_unreadable_contains_permissions() {
        let msg = SoulIncompatibility::SoulUnreadable {
            soul_path: PathBuf::from("/test/soul.md"),
            error: "Permission denied".to_string(),
        }
        .agent_message();
        assert!(msg.contains("could not be read"));
        assert!(msg.contains("Permission denied"));
        assert!(msg.contains("/test/soul.md"));
    }

    #[test]
    fn agent_message_not_found() {
        let msg = SoulIncompatibility::SoulNotFound.agent_message();
        assert!(msg.contains("not initialized"));
        assert!(msg.contains("leiter claude install"));
    }

    /// Guard against bumping epoch constants without adding migration logic.
    ///
    /// When `SETUP_HARD_EPOCH` or `SETUP_SOFT_EPOCH` is bumped, `verify_epochs`
    /// in `agent_setup.rs` must be updated to handle upgrading souls from the
    /// previous epoch. Without migration logic, `leiter claude install` will
    /// refuse to proceed on existing installations, leaving users stuck.
    ///
    /// See DESIGN.md "Epoch system guards all commands against
    /// binary/configuration drift" for full context.
    #[test]
    fn epoch_constants_require_migration_logic_when_bumped() {
        assert_eq!(
            SETUP_HARD_EPOCH, 1,
            "SETUP_HARD_EPOCH was bumped from 1. Update verify_epochs() in \
             agent_setup.rs with migration logic for souls at the previous \
             epoch before updating this assertion. See DESIGN.md."
        );
        assert_eq!(
            SETUP_SOFT_EPOCH, 1,
            "SETUP_SOFT_EPOCH was bumped from 1. Update verify_epochs() in \
             agent_setup.rs with migration logic for souls at the previous \
             epoch before updating this assertion. See DESIGN.md."
        );
    }

    fn status_label(s: &SoulStatus) -> &'static str {
        match s {
            SoulStatus::Compatible { .. } => "Compatible",
            SoulStatus::Incompatible(SoulIncompatibility::SoulNotFound) => "SoulNotFound",
            SoulStatus::Incompatible(SoulIncompatibility::SetupOutdated { .. }) => "SetupOutdated",
            SoulStatus::Incompatible(SoulIncompatibility::BinaryOutdated { .. }) => {
                "BinaryOutdated"
            }
            SoulStatus::Incompatible(SoulIncompatibility::FrontmatterCorrupt { .. }) => {
                "FrontmatterCorrupt"
            }
            SoulStatus::Incompatible(SoulIncompatibility::SoulUnreadable { .. }) => {
                "SoulUnreadable"
            }
        }
    }
}
