//! Unified leiter state stored in `<state_dir>/state.toml`.
//!
//! `state.toml` is the only file the CLI uses for bookkeeping: schema
//! version, setup epochs, soul template version, distillation timestamp, and
//! per-harness transcript watermarks all live here. The soul file is plain
//! markdown owned by the agent.
//!
//! Saves are atomic. Leiter writes a temporary file in the same directory and
//! persists it over `state.toml`, so readers never observe a partially written
//! state file after a crash or interrupted command.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};

use crate::frontmatter::parse_soul;
use crate::paths;
use crate::templates::{SETUP_HARD_EPOCH, SETUP_SOFT_EPOCH, SOUL_TEMPLATE_VERSION};

/// Current on-disk schema version for `state.toml`.
pub const STATE_VERSION: u32 = 1;

/// Leiter-managed metadata persisted in `<state_dir>/state.toml`.
///
/// This struct intentionally excludes the soul body. The agent owns
/// `soul.md`; leiter owns this file. Keeping that boundary explicit prevents
/// ordinary soul edits from corrupting epoch checks or distillation
/// watermarks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeiterState {
    /// Schema version for `state.toml`.
    ///
    /// Missing versions default to the current version for early state files,
    /// but explicit unsupported versions are rejected during load instead of
    /// guessed at.
    #[serde(default = "default_version")]
    pub version: u32,
    /// Soul template version that the current `soul.md` has been migrated to.
    pub soul_version: u32,
    /// Setup epoch for soft (nudge-only) compatibility checks.
    ///
    /// Defaults to 1 so state files written before epochs were split can still
    /// be validated by newer binaries.
    #[serde(default = "default_setup_epoch")]
    pub setup_soft_epoch: u32,
    /// Setup epoch for hard (blocking) compatibility checks.
    ///
    /// Defaults to 1 so state files written before epochs were split can still
    /// be validated by newer binaries.
    #[serde(default = "default_setup_epoch")]
    pub setup_hard_epoch: u32,
    /// Timestamp used by `leiter soul distill` to select undistilled Claude
    /// session logs.
    #[serde(with = "toml_datetime_utc")]
    pub last_distilled: DateTime<Utc>,
    /// Codex rollout watermarks used only when experimental Codex distillation
    /// is enabled.
    ///
    /// The reusable [`WatermarkSet`] shape is intentionally not Codex-specific;
    /// a later schema can add a parallel `claude` field with the same type.
    #[serde(default)]
    pub codex: WatermarkSet,
}

/// A pair of committed and pending transcript watermarks for one harness.
///
/// `committed` is the dedupe baseline from the last successful
/// `mark-distilled` cycle. `pending` is the exact file state emitted by the
/// most recent `distill` run and is promoted only after the agent finishes
/// applying the output.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatermarkSet {
    /// Watermarks accepted as successfully distilled.
    #[serde(default)]
    pub committed: BTreeMap<String, SessionWatermark>,
    /// Watermarks staged by `distill` but not yet accepted by
    /// `mark-distilled`.
    #[serde(default)]
    pub pending: BTreeMap<String, SessionWatermark>,
}

/// Snapshot of one session's transcript file at a specific point in the
/// distill/mark-distilled cycle.
///
/// Leiter needs this record because transcript dedupe is session-level rather
/// than global-timestamp-based: we want to skip re-sending a session if the
/// underlying file has not changed, but re-send the full canonicalized session
/// if it has changed.
///
/// `WatermarkSet` stores these per-session snapshots in its `pending` and
/// `committed` maps, keyed by stable session id:
///
/// - `pending` is the exact snapshot that the last `leiter soul distill` run
///   observed and showed to the LLM
/// - `committed` is the snapshot that the last successful
///   `leiter soul mark-distilled` accepted as distilled
///
/// On the next distill run, leiter compares the currently discovered session
/// file to the `committed` `SessionWatermark`. If the file snapshot still
/// matches, the session is skipped. If it differs, the session is treated as
/// changed and is emitted again in full.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionWatermark {
    /// Absolute path of the transcript file that produced this watermark.
    ///
    /// Path is part of the dedupe key because a session can move between live
    /// and archived transcript trees without changing session identity.
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
    #[serde(with = "toml_datetime_utc")]
    pub mtime_utc: DateTime<Utc>,
    /// Session-level timestamp from the transcript header, when present.
    ///
    /// This is observational metadata used for ordering and debugging, not the
    /// primary dedupe watermark.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "toml_datetime_utc::option"
    )]
    pub session_timestamp_utc: Option<DateTime<Utc>>,
    /// Latest top-level event timestamp seen anywhere in the parsed transcript
    /// file, when one was present on the events.
    ///
    /// This helps explain what portion of the session the watermark covers,
    /// but it does not control dedupe by itself.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "toml_datetime_utc::option"
    )]
    pub latest_event_timestamp_utc: Option<DateTime<Utc>>,
}

/// Why `state.toml` could not be loaded as usable leiter state.
#[derive(Debug)]
pub enum StateLoadError {
    /// No state file exists at the requested path.
    NotFound { path: PathBuf },
    /// The file exists but could not be read.
    Unreadable { path: PathBuf, error: String },
    /// The file was read but is not valid current leiter state.
    Invalid { path: PathBuf, error: String },
}

impl std::fmt::Display for StateLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound { path } => write!(f, "state file not found: {}", path.display()),
            Self::Unreadable { path, error } => {
                write!(
                    f,
                    "state file {} could not be read: {error}",
                    path.display()
                )
            }
            Self::Invalid { path, error } => {
                write!(f, "state file {} is invalid: {error}", path.display())
            }
        }
    }
}

impl std::error::Error for StateLoadError {}

impl LeiterState {
    /// Construct the state written by a fresh `leiter claude install`.
    ///
    /// The timestamp starts at the Unix epoch so the first distillation sees
    /// all existing session logs. Epoch and template fields are stamped from
    /// the current binary because setup has just been performed.
    pub fn fresh() -> Self {
        Self {
            version: STATE_VERSION,
            soul_version: SOUL_TEMPLATE_VERSION,
            setup_soft_epoch: SETUP_SOFT_EPOCH,
            setup_hard_epoch: SETUP_HARD_EPOCH,
            last_distilled: epoch(),
            codex: WatermarkSet::default(),
        }
    }

    /// Load and validate `state.toml`.
    ///
    /// Missing files, unreadable files, and parse/version errors are separate
    /// outcomes because callers present them differently: missing means leiter
    /// is uninitialized, while invalid state blocks commands as corrupt state.
    pub fn load(path: &Path) -> std::result::Result<Self, StateLoadError> {
        let raw = match fs::read_to_string(path) {
            Ok(raw) => raw,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Err(StateLoadError::NotFound {
                    path: path.to_path_buf(),
                });
            }
            Err(err) => {
                return Err(StateLoadError::Unreadable {
                    path: path.to_path_buf(),
                    error: err.to_string(),
                });
            }
        };

        let state: Self = toml::from_str(&raw).map_err(|err| StateLoadError::Invalid {
            path: path.to_path_buf(),
            error: err.to_string(),
        })?;

        if state.version != STATE_VERSION {
            return Err(StateLoadError::Invalid {
                path: path.to_path_buf(),
                error: format!("unsupported state version {}", state.version),
            });
        }

        Ok(state)
    }

    /// Atomically write `state.toml`.
    ///
    /// The parent directory is created if needed. The temporary file is placed
    /// next to the target so `persist` is an atomic rename on the same
    /// filesystem.
    pub fn save(&self, path: &Path) -> Result<()> {
        let parent = path
            .parent()
            .with_context(|| format!("state path must have a parent: {}", path.display()))?;
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;

        let serialized = toml::to_string_pretty(self).context("failed to serialize state")?;
        let mut tmp = tempfile::NamedTempFile::new_in(parent)
            .with_context(|| format!("failed to create temp file in {}", parent.display()))?;
        tmp.write_all(serialized.as_bytes())
            .context("failed to write state temp file")?;
        tmp.persist(path)
            .with_context(|| format!("failed to persist {}", path.display()))?;
        Ok(())
    }
}

/// Convert the legacy frontmatter-plus-codex-meta layout into `state.toml`.
///
/// This is intentionally not wired into any runtime command in this revision;
/// SPEC.md's "Legacy layout migration" section reserves it for a later install
/// flow. Keeping it unit-tested now makes that later wiring a small command
/// change instead of a data migration written under release pressure.
///
/// The routine is deliberately re-runnable from any crash point. Its steps —
/// write `state.toml`, strip the soul frontmatter, delete `codex-meta.toml` —
/// each check current on-disk reality rather than assuming a pristine legacy
/// layout, so a migration that died halfway is completed (not corrupted) by
/// running it again. An existing `state.toml` is kept, never overwritten: it
/// may already carry watermarks newer than anything reconstructable from the
/// legacy files. The soul rewrite is atomic (temp file + rename) because the
/// soul is the one file whose contents cannot be regenerated.
#[allow(dead_code)] // SPEC.md "Legacy layout migration" says this is unwired for now.
pub fn migrate_legacy_layout(state_dir: &Path) -> Result<()> {
    let soul_path = paths::soul_path(state_dir);
    let state_path = paths::state_path(state_dir);
    let codex_meta_path = paths::codex_meta_path(state_dir);

    let raw_soul = fs::read_to_string(&soul_path)
        .with_context(|| format!("failed to read {}", soul_path.display()))?;

    match parse_soul(&raw_soul) {
        Ok((frontmatter, body)) => {
            if !state_path.exists() {
                let codex = match LegacyCodexMeta::load_if_exists(&codex_meta_path)? {
                    Some(meta) => WatermarkSet {
                        committed: meta.committed,
                        pending: meta.pending,
                    },
                    None => WatermarkSet::default(),
                };

                let state = LeiterState {
                    version: STATE_VERSION,
                    soul_version: frontmatter.soul_version,
                    setup_soft_epoch: frontmatter.setup_soft_epoch,
                    setup_hard_epoch: frontmatter.setup_hard_epoch,
                    last_distilled: frontmatter.last_distilled,
                    codex,
                };
                state.save(&state_path)?;
            }

            write_atomic(&soul_path, body)?;
        }
        Err(_) => {
            // No parseable frontmatter: the soul is already stripped. That is
            // only a valid migration state when state.toml exists (a resumed
            // half-migration); otherwise there is nothing here to migrate.
            if !state_path.exists() {
                bail!(
                    "{} has no frontmatter and {} does not exist — not a legacy layout",
                    soul_path.display(),
                    state_path.display()
                );
            }
        }
    }

    if codex_meta_path.exists() {
        fs::remove_file(&codex_meta_path)
            .with_context(|| format!("failed to delete {}", codex_meta_path.display()))?;
    }

    Ok(())
}

/// Write a file via temp-file-plus-rename in its own directory.
///
/// Same guarantee as [`LeiterState::save`]: a crash mid-write never leaves a
/// torn file at the destination.
fn write_atomic(path: &Path, content: &str) -> Result<()> {
    let parent = path
        .parent()
        .with_context(|| format!("path must have a parent: {}", path.display()))?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("failed to create temp file in {}", parent.display()))?;
    tmp.write_all(content.as_bytes())
        .with_context(|| format!("failed to write temp file for {}", path.display()))?;
    tmp.persist(path)
        .with_context(|| format!("failed to persist {}", path.display()))?;
    Ok(())
}

#[derive(Debug, Deserialize)]
struct LegacyCodexMeta {
    #[serde(default = "default_version")]
    version: u32,
    #[serde(default)]
    committed: BTreeMap<String, SessionWatermark>,
    #[serde(default)]
    pending: BTreeMap<String, SessionWatermark>,
}

impl LegacyCodexMeta {
    fn load_if_exists(path: &Path) -> Result<Option<Self>> {
        let raw = match fs::read_to_string(path) {
            Ok(raw) => raw,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => {
                return Err(err).with_context(|| format!("failed to read {}", path.display()));
            }
        };
        let meta: Self =
            toml::from_str(&raw).with_context(|| format!("failed to parse {}", path.display()))?;
        if meta.version != LEGACY_CODEX_META_VERSION {
            bail!(
                "unsupported legacy codex metadata version {} in {}",
                meta.version,
                path.display()
            );
        }
        Ok(Some(meta))
    }
}

/// Schema version of `codex-meta.toml` files at the moment they were retired.
///
/// Frozen forever: legacy files can only ever be version 1, regardless of how
/// far `STATE_VERSION` advances. Comparing against the live constant instead
/// would silently break migration on the first `STATE_VERSION` bump.
const LEGACY_CODEX_META_VERSION: u32 = 1;

/// Files that predate an explicit `version` field are v1 by definition —
/// NOT the current version, or a future binary would misread a version-less
/// v1-era file as current-schema.
fn default_version() -> u32 {
    1
}

fn default_setup_epoch() -> u32 {
    1
}

fn epoch() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(1970, 1, 1, 0, 0, 0).unwrap()
}

fn format_datetime(ts: DateTime<Utc>) -> String {
    ts.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true)
}

mod toml_datetime_utc {
    use super::*;
    use serde::{Deserializer, Serializer, de::Error as _};

    pub fn serialize<S>(value: &DateTime<Utc>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let datetime = toml::value::Datetime::from_str(&format_datetime(*value))
            .map_err(serde::ser::Error::custom)?;
        datetime.serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<DateTime<Utc>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = toml::Value::deserialize(deserializer)?;
        match value {
            toml::Value::Datetime(dt) => parse_datetime(&dt.to_string()).map_err(D::Error::custom),
            toml::Value::String(s) => parse_datetime(&s).map_err(D::Error::custom),
            other => Err(D::Error::custom(format!(
                "expected RFC 3339 datetime, got {other:?}"
            ))),
        }
    }

    fn parse_datetime(value: &str) -> Result<DateTime<Utc>, chrono::ParseError> {
        Ok(DateTime::parse_from_rfc3339(value)?.with_timezone(&Utc))
    }

    pub mod option {
        use super::*;

        pub fn serialize<S>(value: &Option<DateTime<Utc>>, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            match value {
                Some(value) => super::serialize(value, serializer),
                None => serializer.serialize_none(),
            }
        }

        pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<DateTime<Utc>>, D::Error>
        where
            D: Deserializer<'de>,
        {
            let value = Option::<toml::Value>::deserialize(deserializer)?;
            match value {
                Some(toml::Value::Datetime(dt)) => Ok(Some(
                    super::parse_datetime(&dt.to_string()).map_err(D::Error::custom)?,
                )),
                Some(toml::Value::String(s)) => {
                    Ok(Some(super::parse_datetime(&s).map_err(D::Error::custom)?))
                }
                Some(other) => Err(D::Error::custom(format!(
                    "expected RFC 3339 datetime, got {other:?}"
                ))),
                None => Ok(None),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontmatter::{SoulFrontmatter, serialize_soul};

    fn sample_watermark() -> SessionWatermark {
        let ts = Utc.with_ymd_and_hms(2026, 3, 7, 18, 0, 0).unwrap();
        SessionWatermark {
            path: "/tmp/session.jsonl".to_string(),
            size_bytes: 42,
            mtime_utc: ts,
            session_timestamp_utc: Some(ts),
            latest_event_timestamp_utc: Some(ts),
        }
    }

    fn sample_frontmatter() -> SoulFrontmatter {
        SoulFrontmatter {
            last_distilled: Utc.with_ymd_and_hms(2026, 7, 1, 12, 0, 0).unwrap(),
            soul_version: 1,
            setup_soft_epoch: 1,
            setup_hard_epoch: 1,
        }
    }

    #[test]
    fn missing_state_returns_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let err = LeiterState::load(&paths::state_path(tmp.path())).unwrap_err();
        assert!(matches!(err, StateLoadError::NotFound { .. }));
    }

    #[test]
    fn save_round_trips_watermark_shape() {
        let tmp = tempfile::tempdir().unwrap();
        let path = paths::state_path(tmp.path());
        let mut state = LeiterState::fresh();
        state
            .codex
            .committed
            .insert("sess-1".to_string(), sample_watermark());
        state
            .codex
            .pending
            .insert("sess-2".to_string(), sample_watermark());

        state.save(&path).unwrap();
        let raw = fs::read_to_string(&path).unwrap();
        assert!(raw.contains("[codex.committed.sess-1]"), "raw: {raw}");
        assert!(raw.contains("[codex.pending.sess-2]"), "raw: {raw}");
        assert!(
            raw.contains("mtime_utc = 2026-03-07T18:00:00Z"),
            "timestamps must serialize as bare TOML datetimes, not strings: {raw}"
        );

        let loaded = LeiterState::load(&path).unwrap();
        assert_eq!(loaded, state);
    }

    #[test]
    fn invalid_version_is_invalid_not_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let path = paths::state_path(tmp.path());
        fs::write(
            &path,
            "version = 99\nsoul_version = 2\nlast_distilled = 1970-01-01T00:00:00Z\n",
        )
        .unwrap();

        let err = LeiterState::load(&path).unwrap_err();
        assert!(matches!(err, StateLoadError::Invalid { .. }));
        assert!(err.to_string().contains("unsupported state version"));
    }

    #[test]
    fn missing_epoch_fields_default_to_one() {
        let tmp = tempfile::tempdir().unwrap();
        let path = paths::state_path(tmp.path());
        fs::write(
            &path,
            "version = 1\nsoul_version = 2\nlast_distilled = 1970-01-01T00:00:00Z\n",
        )
        .unwrap();

        let state = LeiterState::load(&path).unwrap();
        assert_eq!(state.setup_soft_epoch, 1);
        assert_eq!(state.setup_hard_epoch, 1);
    }

    #[test]
    fn legacy_migration_writes_state_strips_soul_and_deletes_codex_meta() {
        let tmp = tempfile::tempdir().unwrap();
        let state_dir = tmp.path();
        fs::create_dir_all(state_dir).unwrap();
        let fm = sample_frontmatter();
        fs::write(paths::soul_path(state_dir), serialize_soul(&fm, "body\n")).unwrap();
        fs::write(
            paths::codex_meta_path(state_dir),
            r#"version = 1

[committed."committed-sess"]
path = "/tmp/committed.jsonl"
size_bytes = 42
mtime_utc = "2026-03-07T18:00:00Z"
session_timestamp_utc = "2026-03-07T17:59:00Z"
latest_event_timestamp_utc = "2026-03-07T18:00:00Z"

[pending."pending-sess"]
path = "/tmp/pending.jsonl"
size_bytes = 99
mtime_utc = "2026-03-07T19:00:00Z"
session_timestamp_utc = "2026-03-07T18:59:00Z"
latest_event_timestamp_utc = "2026-03-07T19:00:00Z"
"#,
        )
        .unwrap();

        migrate_legacy_layout(state_dir).unwrap();

        let state = LeiterState::load(&paths::state_path(state_dir)).unwrap();
        assert_eq!(state.soul_version, 1);
        assert_eq!(state.last_distilled, fm.last_distilled);
        assert!(state.codex.committed.contains_key("committed-sess"));
        assert!(state.codex.pending.contains_key("pending-sess"));
        assert_eq!(
            fs::read_to_string(paths::soul_path(state_dir)).unwrap(),
            "body\n"
        );
        assert!(!paths::codex_meta_path(state_dir).exists());
    }

    #[test]
    fn legacy_migration_without_codex_meta_writes_empty_codex_maps() {
        let tmp = tempfile::tempdir().unwrap();
        let state_dir = tmp.path();
        let fm = sample_frontmatter();
        fs::write(paths::soul_path(state_dir), serialize_soul(&fm, "body\n")).unwrap();

        migrate_legacy_layout(state_dir).unwrap();

        let state = LeiterState::load(&paths::state_path(state_dir)).unwrap();
        assert_eq!(state.soul_version, fm.soul_version);
        assert_eq!(state.setup_soft_epoch, fm.setup_soft_epoch);
        assert_eq!(state.setup_hard_epoch, fm.setup_hard_epoch);
        assert_eq!(state.last_distilled, fm.last_distilled);
        assert!(state.codex.committed.is_empty());
        assert!(state.codex.pending.is_empty());
    }

    #[test]
    fn legacy_migration_keeps_existing_state_when_stripping_frontmatter() {
        let tmp = tempfile::tempdir().unwrap();
        let state_dir = tmp.path();
        let fm = sample_frontmatter();
        let mut existing = LeiterState::fresh();
        existing.last_distilled = Utc.with_ymd_and_hms(2026, 8, 2, 10, 0, 0).unwrap();
        existing.save(&paths::state_path(state_dir)).unwrap();
        fs::write(paths::soul_path(state_dir), serialize_soul(&fm, "body\n")).unwrap();

        migrate_legacy_layout(state_dir).unwrap();

        let state = LeiterState::load(&paths::state_path(state_dir)).unwrap();
        assert_eq!(state.last_distilled, existing.last_distilled);
        assert_ne!(state.last_distilled, fm.last_distilled);
        assert_eq!(
            fs::read_to_string(paths::soul_path(state_dir)).unwrap(),
            "body\n"
        );
    }

    #[test]
    fn legacy_migration_already_stripped_soul_keeps_files_and_deletes_codex_meta() {
        let tmp = tempfile::tempdir().unwrap();
        let state_dir = tmp.path();
        let existing = LeiterState::fresh();
        existing.save(&paths::state_path(state_dir)).unwrap();
        fs::write(paths::soul_path(state_dir), "body\n").unwrap();
        fs::write(
            paths::codex_meta_path(state_dir),
            "this file is obsolete once state.toml exists",
        )
        .unwrap();

        migrate_legacy_layout(state_dir).unwrap();

        let state = LeiterState::load(&paths::state_path(state_dir)).unwrap();
        assert_eq!(state, existing);
        assert_eq!(
            fs::read_to_string(paths::soul_path(state_dir)).unwrap(),
            "body\n"
        );
        assert!(!paths::codex_meta_path(state_dir).exists());
    }

    #[test]
    fn legacy_migration_frontmatter_free_soul_without_state_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let state_dir = tmp.path();
        fs::write(paths::soul_path(state_dir), "body\n").unwrap();

        let err = migrate_legacy_layout(state_dir).unwrap_err();

        assert!(err.to_string().contains("not a legacy layout"));
    }

    #[test]
    fn legacy_migration_unsupported_codex_meta_version_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let state_dir = tmp.path();
        let fm = sample_frontmatter();
        fs::write(paths::soul_path(state_dir), serialize_soul(&fm, "body\n")).unwrap();
        fs::write(paths::codex_meta_path(state_dir), "version = 99\n").unwrap();

        let err = migrate_legacy_layout(state_dir).unwrap_err();

        assert!(
            err.to_string()
                .contains("unsupported legacy codex metadata version 99")
        );
    }

    #[test]
    fn legacy_migration_corrupt_codex_meta_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let state_dir = tmp.path();
        let fm = sample_frontmatter();
        fs::write(paths::soul_path(state_dir), serialize_soul(&fm, "body\n")).unwrap();
        fs::write(paths::codex_meta_path(state_dir), "not = valid = toml").unwrap();

        let err = migrate_legacy_layout(state_dir).unwrap_err();

        assert!(err.to_string().contains("failed to parse"));
    }
}
