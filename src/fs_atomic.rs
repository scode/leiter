//! Small atomic filesystem writes shared by state and managed block writers.
//!
//! The guarantee is intentionally narrow: callers provide the complete bytes
//! for one file, and this module writes them through a temporary file in the
//! target directory before renaming it into place. It does not fsync directory
//! entries or implement a cross-filesystem fallback; leiter only needs to
//! avoid torn files after ordinary interrupted writes.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Atomically replace `path` with `content`.
///
/// The parent directory is created if needed, and the temporary file is placed
/// beside the final path so `persist` can use a same-directory rename. When
/// `path` is itself a symlink (dotfiles-managed `CLAUDE.md`/`AGENTS.md` setups
/// routinely make it one), the rename targets the resolved file instead so the
/// symlink survives the write — see `resolve_write_target`.
///
/// On unix, an existing target's permission bits are carried onto the temp file
/// before the rename, so replacing an executable or group-readable file does
/// not silently reset it to tempfile's restrictive default. Brand-new files
/// keep that default. Note the caveat: a dotfiles setup that shares a file via
/// a *hard link* rather than a symlink is not preserved — the rename swaps in a
/// fresh inode and breaks the link pairing. Symlinks are the supported form of
/// indirection (see `resolve_write_target`).
pub fn write_atomic(path: &Path, content: &[u8]) -> Result<()> {
    let target = resolve_write_target(path)?;
    let parent = target
        .parent()
        .with_context(|| format!("path must have a parent: {}", target.display()))?;
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("failed to create temp file in {}", parent.display()))?;
    tmp.write_all(content)
        .with_context(|| format!("failed to write temp file for {}", target.display()))?;
    #[cfg(unix)]
    preserve_existing_permissions(&target, tmp.path())?;
    tmp.persist(&target)
        .with_context(|| format!("failed to persist {}", target.display()))?;
    Ok(())
}

/// Copy an existing target's mode onto the temp file before the rename.
///
/// A missing target is the create case: leave tempfile's default alone. Any
/// other stat error surfaces rather than silently writing with the wrong mode.
#[cfg(unix)]
fn preserve_existing_permissions(target: &Path, tmp: &Path) -> Result<()> {
    match fs::metadata(target) {
        Ok(metadata) => fs::set_permissions(tmp, metadata.permissions())
            .with_context(|| format!("failed to preserve permissions for {}", target.display())),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => {
            Err(err).with_context(|| format!("failed to read permissions of {}", target.display()))
        }
    }
}

/// Resolve the real file a write to `path` should land on.
///
/// A bare `rename()` over a symlink replaces the link itself with a regular
/// file, which would silently disconnect a dotfiles-managed `CLAUDE.md` or
/// `AGENTS.md` from wherever it actually points. So when `path`'s final
/// component is a symlink, this follows it to the real file and writes go
/// there instead — the link is never touched. A dangling symlink (the link
/// exists but its target does not) is refused outright rather than silently
/// materializing a new file at the broken target, since that could not match
/// what the user's dotfiles setup intended. A path that is not a symlink at
/// all — including one that does not exist yet — passes through unchanged, so
/// plain creates keep working exactly as before.
fn resolve_write_target(path: &Path) -> Result<PathBuf> {
    let is_symlink = fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_symlink());
    if !is_symlink {
        return Ok(path.to_path_buf());
    }

    fs::canonicalize(path).with_context(|| {
        let link_target = fs::read_link(path)
            .map(|target| target.display().to_string())
            .unwrap_or_else(|_| "<unreadable>".to_string());
        format!(
            "{} is a symlink to {} which does not exist; refusing to write through a dangling link",
            path.display(),
            link_target
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_missing_path_is_created() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("file.txt");

        write_atomic(&path, b"hello").unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"hello");
    }

    #[test]
    #[cfg(unix)]
    fn existing_target_permission_bits_are_preserved() {
        use std::os::unix::fs::PermissionsExt as _;

        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("file.txt");
        fs::write(&path, b"original").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();

        write_atomic(&path, b"updated").unwrap();

        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o644);
        assert_eq!(fs::read(&path).unwrap(), b"updated");
    }

    #[test]
    #[cfg(unix)]
    fn symlinked_target_is_followed_and_link_survives() {
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().join("real.txt");
        let link = tmp.path().join("link.txt");
        fs::write(&real, b"original").unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();

        write_atomic(&link, b"updated").unwrap();

        assert!(
            fs::symlink_metadata(&link).unwrap().is_symlink(),
            "write_atomic must not replace the symlink with a regular file"
        );
        assert_eq!(fs::read(&real).unwrap(), b"updated");
    }

    #[test]
    #[cfg(unix)]
    fn dangling_symlink_target_errors_and_names_the_link() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("nowhere.txt");
        let link = tmp.path().join("link.txt");
        std::os::unix::fs::symlink(&missing, &link).unwrap();

        let err = write_atomic(&link, b"updated").unwrap_err();

        let message = err.to_string();
        assert!(message.contains("dangling"), "message was: {message}");
        assert!(
            message.contains("nowhere.txt"),
            "message must name the link target: {message}"
        );
        assert!(
            !missing.exists(),
            "a dangling link must not be healed by materializing its target"
        );
        assert!(
            fs::symlink_metadata(&link).unwrap().is_symlink(),
            "the link itself must be left untouched"
        );
    }
}
