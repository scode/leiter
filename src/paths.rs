//! Path construction for the leiter state directory.
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
/// `$HOME/.leiter/`. This is the only function that consults runtime environment
/// state — everything else is pure path construction.
pub fn state_dir() -> Result<PathBuf, LeiterError> {
    if let Ok(dir) = std::env::var("LEITER_HOME") {
        // Absolutize so permission_path always sees an absolute path.
        return std::path::absolute(&dir).map_err(|e| LeiterError::StateDir(dir, e));
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

/// Path to the Codex distillation metadata file (`<state_dir>/codex-meta.toml`).
pub fn codex_meta_path(state_dir: &Path) -> PathBuf {
    state_dir.join("codex-meta.toml")
}

/// Path to the main leiter config file (`<state_dir>/leiter.toml`).
pub fn leiter_config_path(state_dir: &Path) -> PathBuf {
    state_dir.join("leiter.toml")
}

/// Default Claude Code home directory (`$HOME/.claude/`).
pub fn default_claude_home() -> Result<PathBuf, LeiterError> {
    Ok(dirs::home_dir()
        .ok_or(LeiterError::HomeNotFound)?
        .join(".claude"))
}

/// Default Codex home directory (`$HOME/.codex/`).
pub fn default_codex_home() -> Result<PathBuf, LeiterError> {
    Ok(dirs::home_dir()
        .ok_or(LeiterError::HomeNotFound)?
        .join(".codex"))
}

/// Format a path for use in Claude Code `permissions.allow` entries.
///
/// Claude Code uses gitignore-style path matching in permission rules:
/// `~/path` for home-relative, `//path` for absolute filesystem paths,
/// and `/path` for project-relative. Paths under `$HOME` become `~/...`;
/// all others become `//...`.
pub fn permission_path(path: &Path) -> String {
    permission_path_with_home(path, dirs::home_dir().as_deref())
}

fn permission_path_with_home(path: &Path, home: Option<&Path>) -> String {
    if let Some(home) = home
        && let Ok(relative) = path.strip_prefix(home)
    {
        return format!("~/{}", relative.display());
    }
    // Absolute paths: `/ + /opt/...` = `//opt/...` (gitignore absolute).
    // state_dir() guarantees absolute paths, so this branch always gets one.
    debug_assert!(path.is_absolute(), "permission_path expects absolute input");
    format!("/{}", path.display())
}

/// Path to a specific skill directory (`<claude_home>/skills/<name>/`).
pub fn skill_dir(claude_home: &Path, name: &str) -> PathBuf {
    claude_home.join("skills").join(name)
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
        assert!(codex_meta_path(dir).starts_with(dir));
        assert!(leiter_config_path(dir).starts_with(dir));
    }

    fn fake_claude_home() -> &'static Path {
        Path::new("/fake/claude")
    }

    #[test]
    fn skill_dir_contains_name() {
        let dir = skill_dir(fake_claude_home(), "leiter-setup");
        assert!(dir.ends_with("leiter-setup"));
        assert!(dir.starts_with(fake_claude_home()));
    }

    #[test]
    fn claude_paths_are_under_claude_home() {
        let ch = fake_claude_home();
        assert!(skill_dir(ch, "test").starts_with(ch));
    }

    #[test]
    fn permission_path_under_home_uses_tilde() {
        let home = Path::new("/Users/alice");
        let path = Path::new("/Users/alice/.leiter/soul.md");
        assert_eq!(
            permission_path_with_home(path, Some(home)),
            "~/.leiter/soul.md"
        );
    }

    #[test]
    fn permission_path_outside_home_uses_double_slash() {
        let home = Path::new("/Users/alice");
        let path = Path::new("/opt/leiter/soul.md");
        assert_eq!(
            permission_path_with_home(path, Some(home)),
            "//opt/leiter/soul.md"
        );
    }

    #[test]
    fn permission_path_no_home_uses_double_slash() {
        let path = Path::new("/opt/leiter/soul.md");
        assert_eq!(
            permission_path_with_home(path, None),
            "//opt/leiter/soul.md"
        );
    }
}
