//! Persistent best-effort distillation metadata for Codex session logs.
//!
//! Claude distillation state still lives in the soul frontmatter. Codex state
//! is tracked separately so we can decide whether a rollout file changed since
//! the last successful `mark-distilled` cycle without touching `~/.codex/`.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

const CODEX_META_VERSION: u32 = 1;

/// On-disk Codex metadata persisted under `~/.leiter/codex-meta.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodexMeta {
    /// Schema version for the `codex-meta.toml` file format.
    ///
    /// This versions the serialized metadata shape, not the Codex transcript
    /// format itself. It lets leiter reject or migrate old/new metadata files
    /// explicitly instead of guessing.
    #[serde(default = "default_version")]
    pub version: u32,
    /// Watermarks that were successfully committed by the most recent
    /// `leiter soul mark-distilled` run.
    ///
    /// This is the authoritative dedupe state: if a discovered Codex session
    /// still matches the watermark stored here, leiter treats that session as
    /// already distilled and skips sending it to the LLM again.
    #[serde(default)]
    pub committed: BTreeMap<String, CodexSessionMeta>,
    /// Watermarks staged by the most recent successful `leiter soul distill`
    /// run, before they have been committed by `mark-distilled`.
    ///
    /// `pending` exists so `mark-distilled` can commit the exact file state
    /// that `distill` showed to the LLM, even if the underlying Codex session
    /// changes in between those two commands.
    #[serde(default)]
    pub pending: BTreeMap<String, CodexSessionMeta>,
}

/// Snapshot of one Codex session's rollout file at a specific point in the
/// distill/mark-distilled cycle.
///
/// Leiter needs this record because Codex dedupe is session-level rather than
/// global-timestamp-based: we want to skip re-sending a Codex session if the
/// underlying rollout file has not changed, but re-send the full canonicalized
/// session if it has changed.
///
/// `CodexMeta` stores these per-session snapshots in its `pending` and
/// `committed` maps, keyed by Codex session id:
///
/// - `pending` is the exact snapshot that the last `leiter soul distill` run
///   observed and showed to the LLM
/// - `committed` is the snapshot that the last successful
///   `leiter soul mark-distilled` accepted as distilled
///
/// On the next distill run, leiter compares the currently discovered session
/// file to the `committed` `CodexSessionMeta`. If the file snapshot still
/// matches, the session is skipped. If it differs, the session is treated as
/// changed and is emitted again in full.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodexSessionMeta {
    /// Absolute path of the rollout file that produced this watermark.
    ///
    /// Path is part of the dedupe key because a session can move from the live
    /// Codex tree to the archived tree without changing session identity.
    pub path: String,
    /// File size in bytes when this watermark was recorded.
    ///
    /// Together with `path` and `mtime_utc`, this is part of the session-level
    /// "has this file changed since last commit?" check.
    pub size_bytes: u64,
    /// File modification time when this watermark was recorded.
    ///
    /// This is stored in UTC so it can be compared deterministically across
    /// runs and across machines/time zones.
    pub mtime_utc: DateTime<Utc>,
    /// Session-level timestamp from the rollout's leading
    /// `session_meta.payload.timestamp`, when present.
    ///
    /// This is observational metadata used for ordering and debugging, not the
    /// primary dedupe watermark.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_timestamp_utc: Option<DateTime<Utc>>,
    /// Latest top-level event timestamp seen anywhere in the parsed rollout
    /// file, when one was present on the events.
    ///
    /// This helps explain what portion of the Codex session the watermark
    /// covers, but it does not control dedupe by itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_event_timestamp_utc: Option<DateTime<Utc>>,
}

fn default_version() -> u32 {
    CODEX_META_VERSION
}

impl Default for CodexMeta {
    fn default() -> Self {
        Self {
            version: default_version(),
            committed: BTreeMap::new(),
            pending: BTreeMap::new(),
        }
    }
}

impl CodexMeta {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }

        let raw = fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let meta: Self =
            toml::from_str(&raw).with_context(|| format!("failed to parse {}", path.display()))?;

        if meta.version != CODEX_META_VERSION {
            bail!(
                "unsupported codex metadata version {} in {}",
                meta.version,
                path.display()
            );
        }

        Ok(meta)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let parent = path.parent().with_context(|| {
            format!(
                "codex metadata path must have a parent directory: {}",
                path.display()
            )
        })?;
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;

        let serialized =
            toml::to_string_pretty(self).context("failed to serialize codex metadata")?;

        let mut tmp = tempfile::NamedTempFile::new_in(parent)
            .with_context(|| format!("failed to create temp file in {}", parent.display()))?;
        use std::io::Write as _;
        tmp.write_all(serialized.as_bytes())
            .context("failed to write codex metadata temp file")?;
        tmp.persist(path)
            .with_context(|| format!("failed to persist {}", path.display()))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn missing_file_returns_default() {
        let tmp = tempfile::tempdir().unwrap();
        let meta = CodexMeta::load(&tmp.path().join("codex-meta.toml")).unwrap();
        assert_eq!(meta, CodexMeta::default());
    }

    #[test]
    fn save_round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("codex-meta.toml");
        let ts = Utc.with_ymd_and_hms(2026, 3, 7, 18, 0, 0).unwrap();

        let mut meta = CodexMeta::default();
        meta.committed.insert(
            "sess-1".to_string(),
            CodexSessionMeta {
                path: "/tmp/session.jsonl".to_string(),
                size_bytes: 42,
                mtime_utc: ts,
                session_timestamp_utc: Some(ts),
                latest_event_timestamp_utc: Some(ts),
            },
        );
        meta.pending = meta.committed.clone();

        meta.save(&path).unwrap();
        let loaded = CodexMeta::load(&path).unwrap();
        assert_eq!(loaded, meta);
    }

    #[test]
    fn invalid_version_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("codex-meta.toml");
        fs::write(&path, "version = 99\n").unwrap();

        let err = CodexMeta::load(&path).unwrap_err();
        assert!(
            err.to_string()
                .contains("unsupported codex metadata version")
        );
    }
}
