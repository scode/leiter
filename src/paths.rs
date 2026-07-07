//! Path construction for the leiter state directory.
//!
//! All leiter state lives under a single directory. These functions build the
//! canonical paths. Commands receive the state directory as a parameter so
//! callers (and tests) can substitute a different root.

use std::path::{Path, PathBuf};

use tracing::warn;

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

/// Path to leiter-managed metadata (`<state_dir>/state.toml`).
pub fn state_path(state_dir: &Path) -> PathBuf {
    state_dir.join("state.toml")
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

/// Resolve the Claude home a command should use, favoring testability.
///
/// This is the fallback used by commands whose Claude-home access is
/// best-effort — external session scans and opportunistic resync — where a
/// missing home should warn-and-skip rather than abort. Those commands take an
/// `Option<&Path>` override and call this instead of touching
/// `default_claude_home` directly, so unit tests can inject a temp directory
/// and never risk touching a developer's real `~/.claude`. It is not the only
/// resolver: `main.rs`'s `leiter claude`/`leiter codex` dispatch resolves the
/// hard-required home directly via [`default_claude_home`], because for those
/// commands (e.g. install) a missing home must fail, not warn-skip.
///
/// An explicit override always wins and is not validated. Without one, a
/// missing `$HOME` degrades to `None` with a logged warning rather than
/// failing the caller outright: callers that only use the Claude home for
/// best-effort, opportunistic work (external session scans, opportunistic
/// resync) should keep working for everything that does not depend on it.
/// Callers for which the Claude home is mandatory (e.g. `leiter sync`) must
/// turn a `None` back into a hard error themselves.
pub fn resolve_claude_home(override_path: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = override_path {
        return Some(path.to_path_buf());
    }

    match default_claude_home() {
        Ok(home) => Some(home),
        Err(err) => {
            warn!("Claude home unavailable: {err}");
            None
        }
    }
}

/// Resolve the Codex home a command should use, but only when Codex support
/// is enabled.
///
/// Mirrors [`resolve_claude_home`]'s override-wins, warn-and-skip contract,
/// with one addition: when `codex_enabled` is false this returns `None`
/// without even attempting default discovery, so callers whose Codex gate is
/// off never emit a spurious "home unavailable" warning for a feature they
/// have not turned on.
pub fn resolve_codex_home(override_path: Option<&Path>, codex_enabled: bool) -> Option<PathBuf> {
    if !codex_enabled {
        return None;
    }

    if let Some(path) = override_path {
        return Some(path.to_path_buf());
    }

    match default_codex_home() {
        Ok(home) => Some(home),
        Err(err) => {
            warn!("Codex home unavailable: {err}");
            None
        }
    }
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

/// Path to Claude Code's managed prompt file.
pub fn claude_md_path(claude_home: &Path) -> PathBuf {
    claude_home.join("CLAUDE.md")
}

/// Path to Codex's managed prompt file.
pub fn agents_md_path(codex_home: &Path) -> PathBuf {
    codex_home.join("AGENTS.md")
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
        assert!(state_path(dir).starts_with(dir));
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

    // These resolver tests deliberately never touch the `$HOME` environment
    // variable: the override branches are fully deterministic, and asserting
    // on the fallback-to-`default_*_home` branch would mean either mutating
    // the test process's environment (disallowed — see CLAUDE.md) or being
    // sensitive to whatever `$HOME` happens to be in CI. Coverage for that
    // branch instead comes from the `tests/integration.rs` end-to-end tests,
    // which redirect `$HOME` for a spawned child process.

    #[test]
    fn resolve_claude_home_prefers_override() {
        let override_path = Path::new("/fake/override/claude");
        assert_eq!(
            resolve_claude_home(Some(override_path)),
            Some(override_path.to_path_buf())
        );
    }

    #[test]
    fn resolve_codex_home_prefers_override_when_enabled() {
        let override_path = Path::new("/fake/override/codex");
        assert_eq!(
            resolve_codex_home(Some(override_path), true),
            Some(override_path.to_path_buf())
        );
    }

    #[test]
    fn resolve_codex_home_ignores_override_when_disabled() {
        let override_path = Path::new("/fake/override/codex");
        assert_eq!(resolve_codex_home(Some(override_path), false), None);
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
