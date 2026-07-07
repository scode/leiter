//! Byte-preserving writer for leiter-managed soul-delivery blocks.
//!
//! The managed span is located only by the public sentinel comments. Content
//! outside that span is carried forward as raw bytes so user-authored
//! `CLAUDE.md` or `AGENTS.md` content survives append, replace, and removal
//! without unrelated normalization.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use sha2::{Digest as _, Sha256};

use crate::fs_atomic::write_atomic;

/// Opening sentinel for the leiter-managed block.
pub const BEGIN_SENTINEL: &str = "<!-- SCODE_LEITER_BEGIN -->";
/// Closing sentinel for the leiter-managed block.
pub const END_SENTINEL: &str = "<!-- SCODE_LEITER_END -->";

/// Result of writing a managed block into a target file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteOutcome {
    /// The file did not exist and now contains only the managed block.
    Created,
    /// The file existed without a managed block and the block was appended.
    Appended,
    /// An existing managed span was replaced in place.
    Replaced,
}

/// Result of removing a managed block from a target file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoveOutcome {
    /// A managed span was found and removed.
    Removed,
    /// The file was missing or did not contain a complete managed span.
    NotPresent,
}

/// Compose the exact managed block that leiter writes to harness files.
///
/// The soul body is fenced with a run of backticks that is strictly longer
/// than any backtick run inside the soul itself. This keeps markdown fences,
/// decorators, and import-like text in the soul as inert data inside the
/// materialized copy.
///
/// Sentinel strings inside the soul are neutralized before embedding (see
/// [`neutralize_sentinels`]): the composed block must never contain sentinel
/// bytes between the real delimiters, or every later span location — replace,
/// removal, clobber detection — silently operates on a truncated span.
pub fn compose_block(soul_path: &Path, soul: &str) -> String {
    let soul = neutralize_sentinels(soul);
    let fence = backtick_fence(&soul);
    format!(
        "{BEGIN_SENTINEL}\n\
         <!-- This block is machine-managed by leiter. Edit {soul_path} and run `leiter sync`; do not edit between these sentinels. `leiter sync` reports hand edits and refuses to overwrite them unless you pass `--force`. -->\n\
         {}\n\
         {fence}\n\
         {soul}\
         {soul_trailing_newline}{fence}\n\
         {END_SENTINEL}\n",
        crate::templates::managed_block_preamble(soul_path),
        soul_path = soul_path.display(),
        soul_trailing_newline = if soul.ends_with('\n') { "" } else { "\n" },
    )
}

/// Break any literal sentinel occurrence in soul text so it cannot delimit.
///
/// A soul that quotes leiter's own sentinels (a session about leiter itself is
/// enough to get them distilled in) would otherwise embed a real END sentinel
/// inside the block: span location would truncate there, an untouched block
/// would hash as "hand-edited" (freezing sync behind false refusals), and a
/// `--force` rewrite would orphan the old block's tail into unfenced
/// instruction position. The transform breaks the opening `<!--` of the
/// matched sentinel text only (`<!- -`), keeping the mention legible while
/// making the byte pattern unmatchable. Only the materialized copy is
/// transformed — `soul.md` itself is never altered.
fn neutralize_sentinels(soul: &str) -> String {
    soul.replace(BEGIN_SENTINEL, &BEGIN_SENTINEL.replacen("<!--", "<!- -", 1))
        .replace(END_SENTINEL, &END_SENTINEL.replacen("<!--", "<!- -", 1))
}

/// Return a backtick fence long enough to contain `soul` verbatim.
pub fn backtick_fence(soul: &str) -> String {
    "`".repeat((longest_backtick_run(soul) + 1).max(3))
}

/// Hex SHA-256 of arbitrary text.
pub fn sha256_hex(content: &str) -> String {
    let digest = Sha256::digest(content.as_bytes());
    format!("{digest:x}")
}

/// Write `block` into `path` according to leiter's sentinel semantics.
///
/// A missing file is created. A file with no sentinels is preserved
/// byte-for-byte and gets the block appended. A file with both sentinels has
/// only the inclusive sentinel span replaced. A `BEGIN` without a matching
/// `END` is an error because the managed span is ambiguous.
///
/// A lone `END` without a preceding `BEGIN` is treated as "no sentinels" and
/// appended to. SPEC.md is silent on that malformed case; append is the least
/// destructive choice because leiter has not found proof that it owns any span
/// in the file.
pub fn write_managed_block(path: &Path, block: &str) -> Result<WriteOutcome> {
    let existing = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            write_atomic(path, block.as_bytes())?;
            return Ok(WriteOutcome::Created);
        }
        Err(err) => return Err(err).with_context(|| format!("failed to read {}", path.display())),
    };

    let rewritten = match locate_span(&existing) {
        SpanLocation::Complete { start, after_end } => {
            let mut next = Vec::with_capacity(existing.len() - (after_end - start) + block.len());
            next.extend_from_slice(&existing[..start]);
            next.extend_from_slice(block.as_bytes());
            next.extend_from_slice(&existing[after_end..]);
            write_atomic(path, &next)?;
            return Ok(WriteOutcome::Replaced);
        }
        SpanLocation::DanglingBegin => {
            bail!(
                "{} contains {} without a matching later {}; refusing to edit it",
                path.display(),
                BEGIN_SENTINEL,
                END_SENTINEL
            );
        }
        SpanLocation::None => {
            let mut next = existing;
            if !next.is_empty() && !next.ends_with(b"\n") {
                next.push(b'\n');
            }
            next.extend_from_slice(block.as_bytes());
            next
        }
    };

    write_atomic(path, &rewritten)?;
    Ok(WriteOutcome::Appended)
}

/// Remove the managed block from `path`, preserving all other bytes.
///
/// Missing files and files without a complete sentinel span are no-ops. A
/// dangling `BEGIN` still errors, matching write semantics: it indicates a
/// malformed managed region whose extent cannot be determined safely.
pub fn remove_managed_block(path: &Path) -> Result<RemoveOutcome> {
    let existing = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(RemoveOutcome::NotPresent);
        }
        Err(err) => return Err(err).with_context(|| format!("failed to read {}", path.display())),
    };

    match locate_span(&existing) {
        SpanLocation::Complete { start, after_end } => {
            let mut next = Vec::with_capacity(existing.len() - (after_end - start));
            next.extend_from_slice(&existing[..start]);
            next.extend_from_slice(&existing[after_end..]);
            write_atomic(path, &next)?;
            Ok(RemoveOutcome::Removed)
        }
        SpanLocation::DanglingBegin => bail!(
            "{} contains {} without a matching later {}; refusing to edit it",
            path.display(),
            BEGIN_SENTINEL,
            END_SENTINEL
        ),
        SpanLocation::None => Ok(RemoveOutcome::NotPresent),
    }
}

/// Return the complete managed span currently present in `path`.
///
/// `Ok(None)` means the file is missing, has no begin sentinel, or has only a
/// lone end sentinel. A dangling begin sentinel is an error because callers
/// must not guess where the managed region ends.
pub fn read_current_block(path: &Path) -> Result<Option<String>> {
    let existing = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).with_context(|| format!("failed to read {}", path.display())),
    };

    match locate_span(&existing) {
        SpanLocation::Complete { start, after_end } => Ok(Some(
            String::from_utf8_lossy(&existing[start..after_end]).into_owned(),
        )),
        SpanLocation::DanglingBegin => bail!(
            "{} contains {} without a matching later {}; refusing to read it",
            path.display(),
            BEGIN_SENTINEL,
            END_SENTINEL
        ),
        SpanLocation::None => Ok(None),
    }
}

fn longest_backtick_run(s: &str) -> usize {
    let mut longest = 0;
    let mut current = 0;
    for ch in s.chars() {
        if ch == '`' {
            current += 1;
            longest = longest.max(current);
        } else {
            current = 0;
        }
    }
    longest
}

/// Where the managed span sits in a file, if anywhere.
enum SpanLocation {
    /// A complete span: byte offset of the BEGIN line's start, and the offset
    /// one past the END line (including its trailing newline when present).
    Complete { start: usize, after_end: usize },
    /// A BEGIN sentinel line with no END sentinel line after it. The span's
    /// extent cannot be determined safely; callers must refuse to edit.
    DanglingBegin,
    /// No BEGIN sentinel line. Any END line without a BEGIN is not leiter's.
    None,
}

/// Locate the managed span with line-anchored sentinel matching.
///
/// Only a line consisting of exactly a sentinel (modulo surrounding ASCII
/// whitespace) delimits the span, and END is searched only after BEGIN. Both
/// properties are load-bearing for the sentinel-injection defense: leiter
/// always writes sentinels on their own lines, so prose mentioning a sentinel
/// mid-line must never truncate the located span (a truncated span makes an
/// untouched block hash as hand-edited, and makes a forced rewrite orphan the
/// old block's tail into unfenced instruction position).
fn locate_span(existing: &[u8]) -> SpanLocation {
    let mut begin: Option<usize> = None;
    let mut offset = 0;
    while offset <= existing.len() {
        let line_end = existing[offset..]
            .iter()
            .position(|&b| b == b'\n')
            .map(|pos| offset + pos)
            .unwrap_or(existing.len());
        let line = &existing[offset..line_end];
        let trimmed = trim_ascii(line);

        match begin {
            Option::None => {
                if trimmed == BEGIN_SENTINEL.as_bytes() {
                    begin = Some(offset);
                }
            }
            Some(start) => {
                if trimmed == END_SENTINEL.as_bytes() {
                    let after_end = if line_end < existing.len() {
                        line_end + 1
                    } else {
                        line_end
                    };
                    return SpanLocation::Complete { start, after_end };
                }
            }
        }

        if line_end >= existing.len() {
            break;
        }
        offset = line_end + 1;
    }

    match begin {
        Some(_) => SpanLocation::DanglingBegin,
        Option::None => SpanLocation::None,
    }
}

fn trim_ascii(line: &[u8]) -> &[u8] {
    let start = line
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(line.len());
    let end = line
        .iter()
        .rposition(|b| !b.is_ascii_whitespace())
        .map(|pos| pos + 1)
        .unwrap_or(start);
    &line[start..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fence_is_longer_than_backtick_runs() {
        assert_eq!(backtick_fence("plain"), "```");
        assert_eq!(backtick_fence("has ``` triple"), "````");
        assert_eq!(backtick_fence("has ```` quad"), "`````");
    }

    #[test]
    fn writer_creates_appends_replaces_and_preserves_surrounding_bytes() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("CLAUDE.md");
        let block = format!("{BEGIN_SENTINEL}\nnew\n{END_SENTINEL}\n");

        assert_eq!(
            write_managed_block(&path, &block).unwrap(),
            WriteOutcome::Created
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), block);

        fs::write(&path, b"alpha").unwrap();
        assert_eq!(
            write_managed_block(&path, &block).unwrap(),
            WriteOutcome::Appended
        );
        assert_eq!(
            fs::read(&path).unwrap(),
            b"alpha\n<!-- SCODE_LEITER_BEGIN -->\nnew\n<!-- SCODE_LEITER_END -->\n"
        );

        fs::write(
            &path,
            b"pre\n<!-- SCODE_LEITER_BEGIN -->\nold\n<!-- SCODE_LEITER_END -->\npost",
        )
        .unwrap();
        assert_eq!(
            write_managed_block(&path, &block).unwrap(),
            WriteOutcome::Replaced
        );
        assert_eq!(
            fs::read(&path).unwrap(),
            b"pre\n<!-- SCODE_LEITER_BEGIN -->\nnew\n<!-- SCODE_LEITER_END -->\npost"
        );
    }

    #[test]
    fn begin_without_end_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("CLAUDE.md");
        fs::write(&path, BEGIN_SENTINEL).unwrap();

        let err = write_managed_block(&path, "block").unwrap_err();
        assert!(err.to_string().contains("without a matching"));
    }

    #[test]
    fn lone_end_is_treated_as_no_sentinels() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("CLAUDE.md");
        fs::write(&path, format!("user\n{END_SENTINEL}\n")).unwrap();

        write_managed_block(&path, "block\n").unwrap();

        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            format!("user\n{END_SENTINEL}\nblock\n")
        );
    }

    #[test]
    fn remove_preserves_unrelated_content() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("CLAUDE.md");
        fs::write(
            &path,
            b"pre\n<!-- SCODE_LEITER_BEGIN -->\nold\n<!-- SCODE_LEITER_END -->\npost",
        )
        .unwrap();

        assert_eq!(remove_managed_block(&path).unwrap(), RemoveOutcome::Removed);
        assert_eq!(fs::read(&path).unwrap(), b"pre\npost");
    }

    /// Dotfiles-managed setups routinely make `CLAUDE.md` a symlink into a
    /// separate config repo. Writing through it must update the real file
    /// while leaving the symlink itself in place — see the "Symlinked targets
    /// are followed, not replaced" paragraph of SPEC.md.
    #[test]
    #[cfg(unix)]
    fn write_through_symlink_preserves_link_and_surrounding_content() {
        let tmp = tempfile::tempdir().unwrap();
        let dotfiles = tmp.path().join("dotfiles");
        fs::create_dir_all(&dotfiles).unwrap();
        let real = dotfiles.join("real.md");
        fs::write(&real, b"pre\npost").unwrap();
        let claude_md = tmp.path().join("CLAUDE.md");
        std::os::unix::fs::symlink(&real, &claude_md).unwrap();

        let block = format!("{BEGIN_SENTINEL}\nnew\n{END_SENTINEL}\n");
        let outcome = write_managed_block(&claude_md, &block).unwrap();

        assert_eq!(outcome, WriteOutcome::Appended);
        assert!(
            fs::symlink_metadata(&claude_md).unwrap().is_symlink(),
            "CLAUDE.md must remain a symlink after the write"
        );
        assert_eq!(
            fs::read(&real).unwrap(),
            format!("pre\npost\n{block}").as_bytes()
        );
    }

    /// An END sentinel appearing before any BEGIN is a malformed, out-of-order
    /// managed region. Because the only BEGIN then has no END after it, span
    /// location reports a dangling begin, and every operation must refuse rather
    /// than guess where the span really is.
    #[test]
    fn end_before_begin_is_out_of_order_and_bails() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("CLAUDE.md");
        fs::write(&path, format!("{END_SENTINEL}\nuser\n{BEGIN_SENTINEL}\n")).unwrap();

        assert!(write_managed_block(&path, "block").is_err());
        assert!(remove_managed_block(&path).is_err());
        assert!(read_current_block(&path).is_err());
    }

    #[test]
    fn remove_missing_file_is_not_present() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("CLAUDE.md");
        assert_eq!(
            remove_managed_block(&path).unwrap(),
            RemoveOutcome::NotPresent
        );
    }

    #[test]
    fn remove_lone_end_is_not_present() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("CLAUDE.md");
        fs::write(&path, format!("user\n{END_SENTINEL}\n")).unwrap();
        assert_eq!(
            remove_managed_block(&path).unwrap(),
            RemoveOutcome::NotPresent
        );
    }

    #[test]
    fn remove_dangling_begin_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("CLAUDE.md");
        fs::write(&path, format!("{BEGIN_SENTINEL}\nno end here\n")).unwrap();
        assert!(remove_managed_block(&path).is_err());
    }

    /// A soul with no trailing newline must still fence cleanly: the closing
    /// backtick fence lands on its own line and the whole block round-trips
    /// through write→read unchanged.
    #[test]
    fn compose_block_without_trailing_newline_round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("CLAUDE.md");
        let block = compose_block(Path::new("/state/soul.md"), "soul body without newline");

        assert!(block.ends_with(&format!("{END_SENTINEL}\n")));
        write_managed_block(&path, &block).unwrap();
        assert_eq!(read_current_block(&path).unwrap().unwrap(), block);
    }

    /// Sentinel-injection defense: a soul that quotes leiter's own sentinels on
    /// their own lines must not plant real delimiter lines inside the composed
    /// block, or span location would truncate. Only the real opening/closing
    /// delimiters may match, and the block must round-trip with a stable hash
    /// across repeated writes.
    #[test]
    fn soul_containing_sentinels_neutralized_and_span_is_stable() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("CLAUDE.md");
        let soul = format!("intro\n{BEGIN_SENTINEL}\nmiddle\n{END_SENTINEL}\ntail\n");
        let block = compose_block(Path::new("/state/soul.md"), &soul);

        let begin_lines = block
            .lines()
            .filter(|line| line.trim() == BEGIN_SENTINEL)
            .count();
        let end_lines = block
            .lines()
            .filter(|line| line.trim() == END_SENTINEL)
            .count();
        assert_eq!(begin_lines, 1, "only the real opening delimiter may match");
        assert_eq!(end_lines, 1, "only the real closing delimiter may match");

        write_managed_block(&path, &block).unwrap();
        let first = read_current_block(&path).unwrap().unwrap();
        write_managed_block(&path, &block).unwrap();
        let second = read_current_block(&path).unwrap().unwrap();
        assert_eq!(first, block);
        assert_eq!(sha256_hex(&first), sha256_hex(&second));
    }

    /// A mid-line mention of a sentinel in user-authored content is prose, not a
    /// delimiter: line-anchored matching leaves it alone, so the block is
    /// appended and the user's line survives byte-for-byte.
    #[test]
    fn mid_line_sentinel_mention_in_user_content_does_not_delimit() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("CLAUDE.md");
        let user = format!("Note: the {END_SENTINEL} marker is machine-managed.\n");
        fs::write(&path, &user).unwrap();

        let block = format!("{BEGIN_SENTINEL}\nnew\n{END_SENTINEL}\n");
        assert_eq!(
            write_managed_block(&path, &block).unwrap(),
            WriteOutcome::Appended
        );

        let content = fs::read_to_string(&path).unwrap();
        assert!(
            content.starts_with(&user),
            "user content preserved verbatim"
        );
        assert!(content.ends_with(&block));
    }

    #[test]
    #[cfg(unix)]
    fn write_through_dangling_symlink_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let claude_md = tmp.path().join("CLAUDE.md");
        std::os::unix::fs::symlink(tmp.path().join("nonexistent.md"), &claude_md).unwrap();

        let block = format!("{BEGIN_SENTINEL}\nnew\n{END_SENTINEL}\n");
        let err = write_managed_block(&claude_md, &block).unwrap_err();

        assert!(
            err.to_string().contains("dangling"),
            "error should mention the dangling link: {err}"
        );
        assert!(
            !claude_md.exists(),
            "a dangling link must not be silently created into a file"
        );
    }
}
