//! Path construction for the `~/.leiter/` state directory.
//!
//! All leiter state lives under a single directory in the user's home. These
//! functions build the canonical paths. They take `home` as a parameter so
//! callers (and tests) can substitute a different root.

use std::path::{Path, PathBuf};

use crate::errors::LeiterError;

/// Resolve the user's home directory via the `dirs` crate.
///
/// This is the only function that touches the real filesystem — everything else
/// is pure path construction.
pub fn home_dir() -> Result<PathBuf, LeiterError> {
    dirs::home_dir().ok_or(LeiterError::HomeNotFound)
}

/// Root of leiter's state directory (`<home>/.leiter/`).
pub fn leiter_dir(home: &Path) -> PathBuf {
    home.join(".leiter")
}

/// Path to the soul file (`<home>/.leiter/soul.md`).
pub fn soul_path(home: &Path) -> PathBuf {
    leiter_dir(home).join("soul.md")
}

/// Path to the session logs directory (`<home>/.leiter/logs/`).
pub fn logs_dir(home: &Path) -> PathBuf {
    leiter_dir(home).join("logs")
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::Path;

    fn fake_home() -> &'static Path {
        Path::new("/fake/home")
    }

    #[test]
    fn leiter_dir_ends_with_dot_leiter() {
        assert!(leiter_dir(fake_home()).ends_with(".leiter"));
    }

    #[test]
    fn soul_path_ends_with_soul_md() {
        assert!(soul_path(fake_home()).ends_with(".leiter/soul.md"));
    }

    #[test]
    fn logs_dir_ends_with_logs() {
        assert!(logs_dir(fake_home()).ends_with(".leiter/logs"));
    }

    #[test]
    fn paths_are_under_home() {
        let home = fake_home();
        assert!(leiter_dir(home).starts_with(home));
        assert!(soul_path(home).starts_with(home));
        assert!(logs_dir(home).starts_with(home));
    }
}
