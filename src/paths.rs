//! Path construction for the `~/.leiter/` state directory.
//!
//! All leiter state lives under a single directory. These functions build the
//! canonical paths. Commands receive the state directory as a parameter so
//! callers (and tests) can substitute a different root.

use std::path::{Path, PathBuf};

use crate::errors::LeiterError;

/// Resolve the leiter state directory.
///
/// Checks `LEITER_HOME` first — when set, it points directly to the state
/// directory (no `.leiter/` suffix appended). Falls back to
/// `~/.leiter/`. This is the only function that consults runtime environment
/// state — everything else is pure path construction.
pub fn state_dir() -> Result<PathBuf, LeiterError> {
    if let Ok(dir) = std::env::var("LEITER_HOME") {
        return Ok(PathBuf::from(dir));
    }
    Ok(dirs::home_dir()
        .ok_or(LeiterError::HomeNotFound)?
        .join(".leiter"))
}

/// Path to the soul file (`<state_dir>/soul.md`).
pub fn soul_path(state_dir: &Path) -> PathBuf {
    state_dir.join("soul.md")
}

/// Path to the session logs directory (`<state_dir>/logs/`).
pub fn logs_dir(state_dir: &Path) -> PathBuf {
    state_dir.join("logs")
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::Path;

    fn fake_state_dir() -> &'static Path {
        Path::new("/fake/state")
    }

    #[test]
    fn soul_path_ends_with_soul_md() {
        assert!(soul_path(fake_state_dir()).ends_with("soul.md"));
    }

    #[test]
    fn logs_dir_ends_with_logs() {
        assert!(logs_dir(fake_state_dir()).ends_with("logs"));
    }

    #[test]
    fn paths_are_under_state_dir() {
        let dir = fake_state_dir();
        assert!(soul_path(dir).starts_with(dir));
        assert!(logs_dir(dir).starts_with(dir));
    }
}
