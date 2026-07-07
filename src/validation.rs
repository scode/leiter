//! Shared state and soul validation for every command except `session-end`.
//!
//! Runtime compatibility now depends on `state.toml`, not metadata embedded in
//! `soul.md`. This module is the single place that loads state, reads the raw
//! soul body, checks setup epochs, and formats blocking messages for both hook
//! output and user-invoked commands.

use std::fs;
use std::path::{Path, PathBuf};

use crate::frontmatter::parse_soul;
use crate::paths;
use crate::state::{LeiterState, StateLoadError};
use crate::templates::SETUP_HARD_EPOCH;

/// Result of validating `state.toml` and `soul.md` against the current binary.
///
/// This enum is returned at command boundaries, not stored in collections or
/// moved through hot paths. Keeping the successful state inline avoids making
/// every caller unpack a `Box<LeiterState>` for a size optimization that does
/// not matter in leiter's CLI workload.
#[allow(clippy::large_enum_variant)]
pub enum ValidationStatus {
    /// State and soul are compatible with this binary.
    Compatible {
        /// Loaded leiter-managed state.
        state: LeiterState,
        /// Full markdown contents of `soul.md`.
        soul: String,
    },
    /// State or soul cannot be used by this binary.
    Incompatible(ValidationIncompatibility),
}

/// Why validation failed before the command could safely run.
pub enum ValidationIncompatibility {
    /// A pre-0.9.0 soul still carries frontmatter but no `state.toml` exists.
    LegacyLayout { soul_path: PathBuf },
    /// `state.toml` is missing, so leiter has not been initialized.
    StateNotFound,
    /// `soul.md` is missing, so leiter has not been initialized.
    SoulNotFound,
    /// `state.toml` exists but could not be parsed or has an unsupported
    /// schema version.
    StateCorrupt { state_path: PathBuf, error: String },
    /// `state.toml` exists but could not be read.
    StateUnreadable { state_path: PathBuf, error: String },
    /// `soul.md` exists but could not be read.
    SoulUnreadable { soul_path: PathBuf, error: String },
    /// State hard epoch is lower than the binary's, so setup must be re-run.
    SetupOutdated { state_epoch: u32, binary_epoch: u32 },
    /// State hard epoch is higher than the binary's, so the binary is old.
    BinaryOutdated { state_epoch: u32, binary_epoch: u32 },
}

impl ValidationIncompatibility {
    /// Agent-facing blocking message for hook and skill command output.
    ///
    /// The hard-failure cases use explicit "word for word" instructions
    /// because this text is delivered to the agent first, not directly to the
    /// user.
    pub fn agent_message(&self) -> String {
        match self {
            Self::LegacyLayout { soul_path } => format!(
                "ACTION REQUIRED: Leiter found the legacy frontmatter layout at {}. \
                 Tell the user EXACTLY this (word for word): \
                 \"Leiter has moved to hookless operation: it no longer uses hooks, \
                 and its six skills are replaced by a single consolidated skill. \
                 Please run `leiter claude install` in your terminal \u{2014} or let me \
                 run it now \u{2014} and follow its instructions, then start a new session.\" \
                 You MAY run `leiter claude install` yourself if the user approves, \
                 then follow that command's output. Do not attempt any other leiter \
                 command for the remainder of this session.",
                soul_path.display()
            ),
            Self::SetupOutdated {
                state_epoch,
                binary_epoch,
            } => format!(
                "ACTION REQUIRED: Leiter setup is incompatible \
                 (setup_hard_epoch: state={state_epoch}, binary={binary_epoch}). \
                 The binary was upgraded but setup was not re-run. \
                 Tell the user EXACTLY this (word for word): \
                 \"Leiter setup needs to be re-run \u{2014} please run \
                 `leiter claude install` in your terminal and follow the \
                 instructions, then start a new session.\" \
                 Do not attempt to use leiter commands in this session."
            ),
            Self::BinaryOutdated {
                state_epoch,
                binary_epoch,
            } => format!(
                "ACTION REQUIRED: Leiter setup is incompatible \
                 (setup_hard_epoch: state={state_epoch}, binary={binary_epoch}). \
                 The state file was created by a newer leiter binary than the \
                 one currently installed. Tell the user EXACTLY this (word for word): \
                 \"Your leiter binary is older than your soul file expects \
                 \u{2014} please upgrade leiter, then start a new session.\" \
                 Do not attempt to use leiter commands in this session."
            ),
            Self::StateCorrupt { state_path, error } => {
                let user_text = corrupt_state_user_text(state_path);
                format!(
                    "ACTION REQUIRED: The leiter state file ({}) is corrupt \
                     ({error}) and leiter cannot verify compatibility. \
                     Tell the user EXACTLY this (word for word): \
                     \"{user_text}\" \
                     Do not attempt to use leiter commands in this session.",
                    state_path.display()
                )
            }
            // Deliberately NOT the corrupt-state text: an unreadable file is
            // usually a permissions or I/O problem, and telling the user to
            // delete it would discard recoverable watermarks for no reason.
            Self::StateUnreadable { state_path, error } => format!(
                "ACTION REQUIRED: The leiter state file ({}) could not be read \
                 ({error}) and leiter cannot verify compatibility. \
                 Tell the user EXACTLY this (word for word): \
                 \"The leiter state file could not be read. \
                 Please check file permissions on {}, then start a new \
                 session.\" \
                 Do not attempt to use leiter commands in this session.",
                state_path.display(),
                state_path.display()
            ),
            Self::SoulUnreadable { soul_path, error } => format!(
                "ACTION REQUIRED: The leiter soul ({}) could not be read \
                 ({error}). Tell the user EXACTLY this (word for word): \
                 \"The leiter soul file could not be read. \
                 Please check file permissions on {}, then start a new \
                 session.\" \
                 Do not attempt to use leiter commands in this session.",
                soul_path.display(),
                soul_path.display()
            ),
            Self::StateNotFound | Self::SoulNotFound => {
                "Leiter is not initialized. Run `leiter claude install` to set up.".to_string()
            }
        }
    }

    /// Human-facing error message for direct CLI failures.
    pub fn user_message(&self) -> String {
        match self {
            Self::LegacyLayout { .. } => {
                "leiter is using the legacy hook-based layout. Run `leiter claude install` to migrate, then start a new session.".to_string()
            }
            Self::SetupOutdated {
                state_epoch,
                binary_epoch,
            } => format!(
                "leiter setup is incompatible \
                 (setup_hard_epoch: state={state_epoch}, binary={binary_epoch}). \
                 Run `leiter claude install` to update, then start a new session."
            ),
            Self::BinaryOutdated {
                state_epoch,
                binary_epoch,
            } => format!(
                "leiter binary is outdated \
                 (setup_hard_epoch: state={state_epoch}, binary={binary_epoch}). \
                 Please upgrade leiter, then start a new session."
            ),
            Self::StateCorrupt { state_path, error } => format!(
                "leiter state file ({}) is corrupt ({error}). Delete it and run \
                 `leiter claude install` to re-initialize, then start a new session.",
                state_path.display()
            ),
            Self::StateUnreadable { state_path, error } => format!(
                "leiter state file ({}) could not be read ({error}). Check file \
                 permissions, then start a new session.",
                state_path.display()
            ),
            Self::SoulUnreadable { soul_path, error } => format!(
                "leiter soul ({}) could not be read ({error}). \
                 Check file permissions and try again.",
                soul_path.display()
            ),
            Self::StateNotFound | Self::SoulNotFound => {
                "leiter is not initialized. Run `leiter claude install` to set up.".to_string()
            }
        }
    }
}

impl std::fmt::Display for ValidationIncompatibility {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.user_message())
    }
}

/// Load `state.toml` and run the hard (blocking) checks, without touching the
/// soul.
///
/// This is the state-only layer under [`validate_state`]. It exists as a
/// separate entry point because `leiter claude install` manages the soul file
/// itself: install must be able to check state compatibility in layouts where
/// the soul legitimately does not exist yet, which the full validation would
/// misreport as "not initialized".
pub fn load_and_check_state(state_dir: &Path) -> Result<LeiterState, ValidationIncompatibility> {
    let state_path = paths::state_path(state_dir);
    let state = match LeiterState::load(&state_path) {
        Ok(state) => state,
        Err(StateLoadError::NotFound { .. }) => {
            return Err(ValidationIncompatibility::StateNotFound);
        }
        Err(StateLoadError::Unreadable { path, error }) => {
            return Err(ValidationIncompatibility::StateUnreadable {
                state_path: path,
                error,
            });
        }
        Err(StateLoadError::Invalid { path, error }) => {
            return Err(ValidationIncompatibility::StateCorrupt {
                state_path: path,
                error,
            });
        }
    };

    if state.setup_hard_epoch < SETUP_HARD_EPOCH {
        return Err(ValidationIncompatibility::SetupOutdated {
            state_epoch: state.setup_hard_epoch,
            binary_epoch: SETUP_HARD_EPOCH,
        });
    }
    if state.setup_hard_epoch > SETUP_HARD_EPOCH {
        return Err(ValidationIncompatibility::BinaryOutdated {
            state_epoch: state.setup_hard_epoch,
            binary_epoch: SETUP_HARD_EPOCH,
        });
    }

    Ok(state)
}

/// Validate state and soul against this binary's epoch expectations.
///
/// This function deliberately reads `soul.md` as raw text. There is no runtime
/// frontmatter parse anymore; a malformed soul is merely malformed markdown,
/// while corrupt `state.toml` blocks commands because epochs and timestamps
/// cannot be trusted.
pub fn validate_state(state_dir: &Path) -> ValidationStatus {
    let state = match load_and_check_state(state_dir) {
        Ok(state) => state,
        Err(ValidationIncompatibility::StateNotFound) => {
            let soul_path = paths::soul_path(state_dir);
            match fs::read_to_string(&soul_path) {
                Ok(raw) if parse_soul(&raw).is_ok() => {
                    return ValidationStatus::Incompatible(
                        ValidationIncompatibility::LegacyLayout { soul_path },
                    );
                }
                Ok(_) => {
                    return ValidationStatus::Incompatible(
                        ValidationIncompatibility::StateNotFound,
                    );
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    return ValidationStatus::Incompatible(
                        ValidationIncompatibility::StateNotFound,
                    );
                }
                Err(e) => {
                    return ValidationStatus::Incompatible(
                        ValidationIncompatibility::SoulUnreadable {
                            soul_path,
                            error: e.to_string(),
                        },
                    );
                }
            }
        }
        Err(reason) => return ValidationStatus::Incompatible(reason),
    };

    let soul_path = paths::soul_path(state_dir);
    let soul = match fs::read_to_string(&soul_path) {
        Ok(content) => content,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return ValidationStatus::Incompatible(ValidationIncompatibility::SoulNotFound);
        }
        Err(e) => {
            return ValidationStatus::Incompatible(ValidationIncompatibility::SoulUnreadable {
                soul_path,
                error: e.to_string(),
            });
        }
    };

    ValidationStatus::Compatible { state, soul }
}

fn corrupt_state_user_text(state_path: &Path) -> String {
    format!(
        "The leiter state file is corrupt. Please delete {} and run `leiter claude install` to re-initialize, then start a new session.",
        state_path.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::test_support::{setup_state_dir, write_state_with_epochs};
    use crate::templates::{SETUP_HARD_EPOCH, SETUP_SOFT_EPOCH};

    #[test]
    fn epoch_constants_match_spec() {
        assert_eq!(SETUP_SOFT_EPOCH, 2);
        assert_eq!(
            SETUP_HARD_EPOCH, 2,
            "hard epoch 2 is the hookless migration trigger; keep this guard in sync with install's legacy migration path"
        );
    }

    #[test]
    fn missing_state_returns_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        match validate_state(tmp.path()) {
            ValidationStatus::Incompatible(ValidationIncompatibility::StateNotFound) => {}
            other => panic!("expected StateNotFound, got {}", status_label(&other)),
        }
    }

    #[test]
    fn missing_state_with_frontmatter_soul_returns_legacy_layout() {
        let tmp = tempfile::tempdir().unwrap();
        let soul_path = paths::soul_path(tmp.path());
        std::fs::write(
            &soul_path,
            "---\nlast_distilled: 2026-01-01T00:00:00Z\nsoul_version: 1\nsetup_soft_epoch: 1\nsetup_hard_epoch: 1\n---\nbody\n",
        )
        .unwrap();

        match validate_state(tmp.path()) {
            ValidationStatus::Incompatible(ValidationIncompatibility::LegacyLayout {
                soul_path: path,
            }) => assert_eq!(path, soul_path),
            other => panic!("expected LegacyLayout, got {}", status_label(&other)),
        }
    }

    #[test]
    fn missing_state_with_frontmatter_free_soul_returns_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(paths::soul_path(tmp.path()), "# Plain Soul\n").unwrap();

        match validate_state(tmp.path()) {
            ValidationStatus::Incompatible(ValidationIncompatibility::StateNotFound) => {}
            other => panic!("expected StateNotFound, got {}", status_label(&other)),
        }
    }

    #[test]
    fn missing_state_with_soul_directory_returns_soul_unreadable() {
        let tmp = tempfile::tempdir().unwrap();
        let soul_path = paths::soul_path(tmp.path());
        std::fs::create_dir(&soul_path).unwrap();

        match validate_state(tmp.path()) {
            ValidationStatus::Incompatible(ValidationIncompatibility::SoulUnreadable {
                soul_path: path,
                ..
            }) => assert_eq!(path, soul_path),
            other => panic!("expected SoulUnreadable, got {}", status_label(&other)),
        }
    }

    #[test]
    fn legacy_layout_agent_message_contains_verbatim_migration_text() {
        let msg = ValidationIncompatibility::LegacyLayout {
            soul_path: PathBuf::from("/tmp/leiter/soul.md"),
        }
        .agent_message();
        assert!(msg.contains("ACTION REQUIRED"));
        assert!(msg.contains("Tell the user EXACTLY this (word for word)"));
        assert!(msg.contains("Leiter has moved to hookless operation: it no longer uses hooks, and its six skills are replaced by a single consolidated skill. Please run `leiter claude install` in your terminal \u{2014} or let me run it now \u{2014} and follow its instructions, then start a new session."));
        assert!(msg.contains("MAY run `leiter claude install`"));
        assert!(msg.contains("Do not attempt any other leiter command"));
        assert!(!msg.contains("Do not attempt to use leiter commands"));
    }

    #[test]
    fn missing_soul_returns_not_found() {
        let tmp = setup_state_dir();
        std::fs::remove_file(paths::soul_path(tmp.path())).unwrap();
        match validate_state(tmp.path()) {
            ValidationStatus::Incompatible(ValidationIncompatibility::SoulNotFound) => {}
            other => panic!("expected SoulNotFound, got {}", status_label(&other)),
        }
    }

    #[test]
    fn corrupt_state_returns_corrupt() {
        let tmp = setup_state_dir();
        std::fs::write(paths::state_path(tmp.path()), "not = valid = toml").unwrap();
        match validate_state(tmp.path()) {
            ValidationStatus::Incompatible(ValidationIncompatibility::StateCorrupt { .. }) => {}
            other => panic!("expected StateCorrupt, got {}", status_label(&other)),
        }
    }

    #[test]
    fn directory_state_path_returns_unreadable_message() {
        let tmp = setup_state_dir();
        let state_path = paths::state_path(tmp.path());
        std::fs::remove_file(&state_path).unwrap();
        std::fs::create_dir(&state_path).unwrap();

        match validate_state(tmp.path()) {
            ValidationStatus::Incompatible(ValidationIncompatibility::StateUnreadable {
                state_path: path,
                error,
            }) => {
                assert_eq!(path, state_path);
                let msg = ValidationIncompatibility::StateUnreadable { state_path, error }
                    .agent_message();
                assert!(msg.contains("could not be read"));
                assert!(msg.contains("file permissions"));
                assert!(!msg.contains("delete"));
            }
            other => panic!("expected StateUnreadable, got {}", status_label(&other)),
        }
    }

    #[test]
    fn unsupported_state_version_returns_corrupt() {
        let tmp = setup_state_dir();
        std::fs::write(
            paths::state_path(tmp.path()),
            "version = 99\nsoul_version = 2\nlast_distilled = 1970-01-01T00:00:00Z\n",
        )
        .unwrap();
        match validate_state(tmp.path()) {
            ValidationStatus::Incompatible(ValidationIncompatibility::StateCorrupt { .. }) => {}
            other => panic!("expected StateCorrupt, got {}", status_label(&other)),
        }
    }

    #[test]
    fn hard_epoch_less_returns_setup_outdated() {
        let tmp = tempfile::tempdir().unwrap();
        write_state_with_epochs(
            tmp.path(),
            SETUP_SOFT_EPOCH,
            SETUP_HARD_EPOCH.saturating_sub(1),
        );
        match validate_state(tmp.path()) {
            ValidationStatus::Incompatible(ValidationIncompatibility::SetupOutdated { .. }) => {}
            other => panic!("expected SetupOutdated, got {}", status_label(&other)),
        }
    }

    #[test]
    fn hard_epoch_greater_returns_binary_outdated() {
        let tmp = tempfile::tempdir().unwrap();
        write_state_with_epochs(tmp.path(), SETUP_SOFT_EPOCH, SETUP_HARD_EPOCH + 1);
        match validate_state(tmp.path()) {
            ValidationStatus::Incompatible(ValidationIncompatibility::BinaryOutdated {
                ..
            }) => {}
            other => panic!("expected BinaryOutdated, got {}", status_label(&other)),
        }
    }

    #[test]
    fn matching_epochs_returns_compatible() {
        let tmp = setup_state_dir();
        match validate_state(tmp.path()) {
            ValidationStatus::Compatible { .. } => {}
            other => panic!("expected Compatible, got {}", status_label(&other)),
        }
    }

    #[test]
    fn compatible_returns_raw_soul_content() {
        let tmp = setup_state_dir();
        let content = std::fs::read_to_string(paths::soul_path(tmp.path())).unwrap();
        match validate_state(tmp.path()) {
            ValidationStatus::Compatible { soul, .. } => assert_eq!(soul, content),
            other => panic!("expected Compatible, got {}", status_label(&other)),
        }
    }

    #[test]
    fn corrupt_state_agent_message_contains_verbatim_recovery_text() {
        let path = PathBuf::from("/tmp/leiter/state.toml");
        let msg = ValidationIncompatibility::StateCorrupt {
            state_path: path.clone(),
            error: "bad toml".to_string(),
        }
        .agent_message();
        assert!(msg.contains("ACTION REQUIRED"));
        assert!(msg.contains("Tell the user EXACTLY this (word for word)"));
        assert!(msg.contains(&format!("Please delete {}", path.display())));
        assert!(msg.contains("Do not attempt to use leiter commands"));
    }

    #[test]
    fn state_unreadable_message_mentions_read_error() {
        let path = PathBuf::from("/tmp/leiter/state.toml");
        let msg = ValidationIncompatibility::StateUnreadable {
            state_path: path,
            error: "permission denied".to_string(),
        }
        .agent_message();
        assert!(msg.contains("could not be read"));
        assert!(msg.contains("permission denied"));
    }

    fn status_label(s: &ValidationStatus) -> &'static str {
        match s {
            ValidationStatus::Compatible { .. } => "Compatible",
            ValidationStatus::Incompatible(ValidationIncompatibility::LegacyLayout { .. }) => {
                "LegacyLayout"
            }
            ValidationStatus::Incompatible(ValidationIncompatibility::StateNotFound) => {
                "StateNotFound"
            }
            ValidationStatus::Incompatible(ValidationIncompatibility::SoulNotFound) => {
                "SoulNotFound"
            }
            ValidationStatus::Incompatible(ValidationIncompatibility::StateCorrupt { .. }) => {
                "StateCorrupt"
            }
            ValidationStatus::Incompatible(ValidationIncompatibility::StateUnreadable {
                ..
            }) => "StateUnreadable",
            ValidationStatus::Incompatible(ValidationIncompatibility::SoulUnreadable {
                ..
            }) => "SoulUnreadable",
            ValidationStatus::Incompatible(ValidationIncompatibility::SetupOutdated { .. }) => {
                "SetupOutdated"
            }
            ValidationStatus::Incompatible(ValidationIncompatibility::BinaryOutdated {
                ..
            }) => "BinaryOutdated",
        }
    }
}
