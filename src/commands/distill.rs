//! `leiter soul distill` — output new session logs for the agent to distill.
//!
//! Reads the `last_distilled` timestamp from `state.toml`, scans the
//! logs directory for files with timestamps >= that value, and outputs them
//! chronologically. The inclusive comparison ensures a log written in the same
//! second as the distillation timestamp is not lost.
//!
//! After output, performs best-effort deletion of log files with timestamps
//! strictly before `last_distilled` (already processed by a prior
//! distillation). With `--dry-run`, reports what would be deleted instead.
//!
//! ## JSONL pre-processing
//!
//! Claude Code session transcripts are JSONL files where each line is a JSON
//! object with a `type` field. The vast majority of content is tool machinery
//! invisible to the user (tool results, tool invocations, progress events,
//! thinking blocks, file history snapshots). In observed sessions, user text +
//! assistant text combined are typically 2–15% of the file.
//!
//! We filter each log down to approximately what the user saw:
//!
//! Kept:
//!   - `type: "user"` without `toolUseResult` key (user messages)
//!   - `type: "assistant"` with text blocks (assistant responses)
//!   - Unknown types (fail-useful)
//!   - Non-JSON lines (fail-useful)
//!
//! Dropped:
//!   - `type: "user"` with `toolUseResult` (tool output)
//!   - `type: "assistant"` with only tool_use/thinking blocks
//!   - `type: "progress"`, `"file-history-snapshot"`, `"system"`
//!
//! Uses `serde_json::Value` (not typed structs) to stay resilient to schema
//! changes. If parsing or field access fails, we include the raw line rather
//! than silently dropping it.

use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use chrono::SubsecRound;
use tracing::{debug, warn};

use crate::claude_sessions::{
    ClaudeScanOutcome, DistilledClaudeSession,
    collect_changed_sessions as collect_claude_changed_sessions,
};
use crate::claude_transcript::filter_session_log;
use crate::codex::{DistilledCodexSession, collect_changed_sessions};
use crate::config::LeiterConfig;
use crate::log_filename::{ParsedLogEntry, collect_log_entries};
use crate::paths;
use crate::state::LeiterState;
use crate::templates::{DISTILL_DATA_PREAMBLE, SOUL_WRITING_GUIDELINES};
use crate::validation::{ValidationStatus, validate_state};

/// Run the distill command.
///
/// Validates `state.toml` epochs and soul readability, then outputs all
/// session logs whose filename timestamps are >= `last_distilled` from state, sorted
/// chronologically. Then deletes obsolete logs (timestamps strictly before
/// `last_distilled`). With `dry_run`, reports what would be deleted instead.
pub fn run(
    state_dir: &Path,
    out: &mut impl Write,
    dry_run: bool,
    claude_home_override: Option<&Path>,
    codex_home_override: Option<&Path>,
) -> Result<()> {
    let config = load_config_best_effort(state_dir);
    let claude_home = resolve_claude_home(claude_home_override);
    let codex_home = resolve_codex_home(codex_home_override, config.enable_codex_experimental);
    distill_with_inputs(
        state_dir,
        claude_home.as_deref(),
        codex_home.as_deref(),
        config.enable_codex_experimental,
        out,
        dry_run,
    )
}

/// Shared distill implementation once runtime inputs have already been
/// resolved.
///
/// `run` uses this after reading config and resolving the default Codex home.
/// Unit tests call the same function directly so they exercise the production
/// distill algorithm while still injecting a temp Codex home path and gate
/// state.
fn distill_with_inputs(
    state_dir: &Path,
    claude_home: Option<&Path>,
    codex_home: Option<&Path>,
    codex_enabled: bool,
    out: &mut impl Write,
    dry_run: bool,
) -> Result<()> {
    let logs_dir = paths::logs_dir(state_dir);
    // Captured before any scanning so mark-distilled can adopt it as the next
    // last_distilled: everything that happens after this instant is the next
    // run's responsibility, including sessions born mid-distill.
    let scan_started = chrono::Utc::now().trunc_subsecs(0);

    let mut state = match validate_state(state_dir) {
        ValidationStatus::Incompatible(reason) => bail!("{}", reason.agent_message()),
        ValidationStatus::Compatible { state, .. } => state,
    };

    let entries = collect_log_entries(&logs_dir)
        .with_context(|| format!("failed to read logs directory: {}", logs_dir.display()))?;

    let mut legacy_logs = Vec::new();
    let mut obsolete = Vec::new();

    for entry in entries {
        if entry.timestamp >= state.last_distilled {
            legacy_logs.push(entry);
        } else {
            obsolete.push(entry);
        }
    }

    let claude_scan =
        collect_claude_sessions(state_dir, &mut state, claude_home, scan_started, dry_run);
    // Suppress legacy copies of every session the external store accounts
    // for — both sessions emitted this run and sessions whose committed
    // watermark says they were already distilled (the idle-exit case: the
    // SessionEnd hook copies a log after mark-distilled committed the
    // session). Floor-skipped ids are deliberately not in this set.
    legacy_logs.retain(|entry| !claude_scan.accounted_ids.contains(&entry.session_id));
    let external_claude_sessions = claude_scan.changed;

    let codex_sessions =
        collect_codex_sessions(state_dir, &mut state, codex_home, codex_enabled, dry_run);

    let claude_emissions = merge_claude_emissions(legacy_logs, external_claude_sessions);
    let has_claude_logs = !claude_emissions.is_empty();
    let has_codex_logs = !codex_sessions.is_empty();

    if !has_claude_logs && !has_codex_logs {
        writeln!(out, "No new session logs to process.")?;
    } else {
        write!(out, "{SOUL_WRITING_GUIDELINES}")?;
        writeln!(out, "{DISTILL_DATA_PREAMBLE}")?;
        writeln!(out, "<session-transcripts>")?;

        for emission in &claude_emissions {
            match emission {
                ClaudeEmission::Legacy(entry) => {
                    let content = fs::read_to_string(&entry.path).with_context(|| {
                        format!("failed to read log file: {}", entry.path.display())
                    })?;
                    writeln!(
                        out,
                        "<session source=\"claude\" file=\"{}\">",
                        entry.filename
                    )?;
                    filter_session_log(&content, out)?;
                    writeln!(out, "</session>")?;
                }
                ClaudeEmission::External(session) => {
                    writeln!(
                        out,
                        "<session source=\"claude\" file=\"{}\">",
                        session.file_label
                    )?;
                    write!(out, "{}", session.rendered)?;
                    writeln!(out, "</session>")?;
                }
            }
        }

        for session in &codex_sessions {
            writeln!(
                out,
                "<session source=\"codex\" file=\"{}\">",
                session.file_label
            )?;
            write!(out, "{}", session.rendered)?;
            writeln!(out, "</session>")?;
        }

        writeln!(out, "</session-transcripts>")?;
    }

    if !obsolete.is_empty() {
        obsolete.sort_by(|a, b| a.filename.cmp(&b.filename));

        if dry_run {
            writeln!(out, "Obsolete logs that would be deleted:")?;
            for entry in &obsolete {
                writeln!(out, "  {}", entry.filename)?;
            }
        } else {
            for entry in &obsolete {
                match fs::remove_file(&entry.path) {
                    Ok(()) => {
                        debug!("deleted obsolete log: {}", entry.filename);
                    }
                    Err(e) => {
                        warn!("failed to delete obsolete log {}: {e}", entry.filename);
                    }
                }
            }
        }
    }

    Ok(())
}

/// Resolve the Claude home used by the external session scan.
///
/// A caller-provided path wins and is not validated here; discovery itself is
/// best-effort and treats missing directories as an empty scan. Without an
/// override, failure to locate the user's home directory disables only the
/// external Claude scan and leaves legacy logs usable.
fn resolve_claude_home(override_path: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = override_path {
        return Some(path.to_path_buf());
    }

    match paths::default_claude_home() {
        Ok(claude_home) => Some(claude_home),
        Err(err) => {
            warn!("Claude home unavailable, skipping external Claude session scan: {err}");
            None
        }
    }
}

/// Resolve the Codex home only when the Codex gate is enabled.
///
/// The disabled gate must not even look at the default Codex directory. When
/// enabled, explicit test injection wins; otherwise a missing home directory
/// is logged and treated as "no Codex sessions" rather than a distill error.
fn resolve_codex_home(override_path: Option<&Path>, codex_enabled: bool) -> Option<PathBuf> {
    if !codex_enabled {
        return None;
    }

    if let Some(path) = override_path {
        return Some(path.to_path_buf());
    }

    match paths::default_codex_home() {
        Ok(codex_home) => Some(codex_home),
        Err(err) => {
            warn!("Codex home unavailable, skipping Codex distillation: {err}");
            None
        }
    }
}

/// Best-effort external Claude collection for one `leiter soul distill` run.
///
/// The scanner is always active. When it finds changed sessions, every changed
/// watermark is staged on non-dry-run, including sessions that render to no
/// visible content. Emission filters happen later so staging stays tied to the
/// exact file states this distill run observed.
///
/// The pending map is replaced even when the Claude home cannot be resolved
/// and the scan is skipped: pending means "exactly what this run showed the
/// LLM", and a run that scanned nothing showed nothing — leaving a stale
/// pending map behind would let the next mark-distilled commit watermarks for
/// sessions this cycle never emitted. The staged scan-start time is what
/// mark-distilled later adopts as `last_distilled` (see that command's docs
/// for the loss-window rationale).
fn collect_claude_sessions(
    state_dir: &Path,
    state: &mut LeiterState,
    claude_home: Option<&Path>,
    scan_started: chrono::DateTime<chrono::Utc>,
    dry_run: bool,
) -> ClaudeScanOutcome {
    let outcome = match claude_home {
        Some(claude_home) => collect_claude_changed_sessions(
            claude_home,
            &state.claude.committed,
            state.last_distilled,
        ),
        None => ClaudeScanOutcome {
            changed: Vec::new(),
            accounted_ids: BTreeSet::new(),
        },
    };

    if !dry_run {
        state.claude.pending = outcome
            .changed
            .iter()
            .map(|session| (session.session_id.clone(), session.watermark.clone()))
            .collect();
        state.pending_scan_started_utc = Some(scan_started);
        if let Err(err) = state.save(&paths::state_path(state_dir)) {
            warn!("failed to update leiter state with Claude pending watermarks: {err}");
        }
    }

    outcome
}

/// Best-effort Codex collection for one `leiter soul distill` run.
///
/// This encapsulates the Codex-specific metadata load, changed-session
/// discovery, and pending-watermark staging. It returns only the changed
/// sessions that produced visible rendered transcript content; changed sessions
/// that canonicalize to nothing still matter for watermark staging, but they do
/// not need to escape this helper because they are never emitted to the LLM.
fn collect_codex_sessions(
    state_dir: &Path,
    state: &mut LeiterState,
    codex_home: Option<&Path>,
    codex_enabled: bool,
    dry_run: bool,
) -> Vec<DistilledCodexSession> {
    if !codex_enabled {
        return Vec::new();
    }

    let Some(codex_home) = codex_home else {
        return Vec::new();
    };

    let codex_sessions = collect_changed_sessions(codex_home, &state.codex.committed);
    if !dry_run {
        state.codex.pending = codex_sessions
            .iter()
            .map(|session| (session.session_id.clone(), session.watermark.clone()))
            .collect();
        if let Err(err) = state.save(&paths::state_path(state_dir)) {
            warn!("failed to update leiter state with Codex pending watermarks: {err}");
        }
    }

    codex_sessions
        .into_iter()
        .filter(|session| !session.rendered.is_empty())
        .collect()
}

/// Claude transcript item ready to be emitted in chronological order.
///
/// Legacy hook logs and external Claude sessions have different label and
/// rendering paths, but they share one ordering stream. Keeping them in this
/// enum makes the merge explicit without flattening away the source-specific
/// read behavior.
enum ClaudeEmission {
    /// Hook-copied log from `<state_dir>/logs/`.
    Legacy(ParsedLogEntry),
    /// Directly scanned Claude Code project transcript.
    External(DistilledClaudeSession),
}

/// Merge legacy and external Claude sessions into one chronological stream.
///
/// Empty external sessions are excluded from emission here, after their
/// watermarks have already been staged. Legacy logs use filename timestamps;
/// external sessions use the scanner's bounded header timestamp with mtime
/// fallback.
fn merge_claude_emissions(
    legacy_logs: Vec<ParsedLogEntry>,
    external_sessions: Vec<DistilledClaudeSession>,
) -> Vec<ClaudeEmission> {
    let mut emissions = Vec::new();
    for entry in legacy_logs {
        emissions.push(ClaudeEmission::Legacy(entry));
    }
    for session in external_sessions {
        if !session.rendered.is_empty() {
            emissions.push(ClaudeEmission::External(session));
        }
    }

    emissions.sort_by(|a, b| {
        emission_sort_timestamp(a)
            .cmp(&emission_sort_timestamp(b))
            .then_with(|| emission_sort_label(a).cmp(emission_sort_label(b)))
    });
    emissions
}

/// Return the timestamp key for one Claude emission.
///
/// This keeps the sort contract in one place: filename time for legacy logs,
/// scanner-derived session time for external transcripts.
fn emission_sort_timestamp(emission: &ClaudeEmission) -> chrono::DateTime<chrono::Utc> {
    match emission {
        ClaudeEmission::Legacy(entry) => entry.timestamp,
        ClaudeEmission::External(session) => session.sort_timestamp,
    }
}

/// Return a deterministic tie-break label for one Claude emission.
///
/// The exact label is not semantically meaningful; it only keeps output stable
/// when two sessions share the same sort timestamp.
fn emission_sort_label(emission: &ClaudeEmission) -> &str {
    match emission {
        ClaudeEmission::Legacy(entry) => &entry.filename,
        ClaudeEmission::External(session) => &session.session_id,
    }
}

fn load_config_best_effort(state_dir: &Path) -> LeiterConfig {
    let config_path = paths::leiter_config_path(state_dir);
    match LeiterConfig::load(&config_path) {
        Ok(config) => config,
        Err(err) => {
            warn!("failed to load leiter config, using defaults: {err}");
            LeiterConfig::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::test_support::{
        bytes_to_string, setup_state_dir, update_state, write_state_with_epochs,
    };
    use crate::config::LeiterConfig;
    use crate::log_filename::generate_log_filename;
    use crate::state::{LeiterState, SessionWatermark};
    use crate::templates::{SETUP_HARD_EPOCH, SETUP_SOFT_EPOCH};
    use chrono::{DateTime, TimeZone, Utc};
    use serde_json::Value;
    use std::path::PathBuf;

    fn run_distill(state_dir: &Path) -> String {
        let claude_home = tempfile::tempdir().unwrap();
        let mut out = Vec::new();
        distill_with_inputs(
            state_dir,
            Some(claude_home.path()),
            None,
            false,
            &mut out,
            false,
        )
        .unwrap();
        bytes_to_string(out)
    }

    fn run_distill_dry(state_dir: &Path) -> String {
        let claude_home = tempfile::tempdir().unwrap();
        let mut out = Vec::new();
        distill_with_inputs(
            state_dir,
            Some(claude_home.path()),
            None,
            false,
            &mut out,
            true,
        )
        .unwrap();
        bytes_to_string(out)
    }

    fn run_distill_with_codex_home(state_dir: &Path, codex_home: &Path, dry_run: bool) -> String {
        let claude_home = tempfile::tempdir().unwrap();
        let mut out = Vec::new();
        distill_with_inputs(
            state_dir,
            Some(claude_home.path()),
            Some(codex_home),
            true,
            &mut out,
            dry_run,
        )
        .unwrap();
        bytes_to_string(out)
    }

    fn run_distill_with_claude_home(state_dir: &Path, claude_home: &Path, dry_run: bool) -> String {
        let mut out = Vec::new();
        distill_with_inputs(state_dir, Some(claude_home), None, false, &mut out, dry_run).unwrap();
        bytes_to_string(out)
    }

    fn set_codex_enabled(state_dir: &Path, enabled: bool) {
        let config = LeiterConfig {
            enable_codex_experimental: enabled,
        };
        config.save(&paths::leiter_config_path(state_dir)).unwrap();
    }

    fn write_log(
        state_dir: &Path,
        year: i32,
        month: u32,
        day: u32,
        hour: u32,
        session_id: &str,
        content: &str,
    ) {
        let ts = Utc.with_ymd_and_hms(year, month, day, hour, 0, 0).unwrap();
        let filename = generate_log_filename(ts, session_id);
        let path = paths::logs_dir(state_dir).join(filename);
        fs::write(path, content).unwrap();
    }

    fn write_claude_session(
        claude_home: &Path,
        slug: &str,
        session_id: &str,
        lines: &[Value],
    ) -> PathBuf {
        let path = claude_home
            .join("projects")
            .join(slug)
            .join(format!("{session_id}.jsonl"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut rendered = String::new();
        for line in lines {
            rendered.push_str(&serde_json::to_string(line).unwrap());
            rendered.push('\n');
        }
        fs::write(&path, rendered).unwrap();
        path
    }

    fn claude_user_line(text: &str, ts: &str) -> Value {
        serde_json::json!({
            "timestamp": ts,
            "type": "user",
            "message": {"content": text}
        })
    }

    fn claude_progress_line() -> Value {
        serde_json::json!({"type": "progress", "data": {"type": "agent_progress"}})
    }

    fn set_last_distilled(state_dir: &Path, year: i32, month: u32, day: u32, hour: u32) {
        let ts = Utc.with_ymd_and_hms(year, month, day, hour, 0, 0).unwrap();
        update_state(state_dir, |state| state.last_distilled = ts);
    }

    fn ts(year: i32, month: u32, day: u32, hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, hour, 0, 0).unwrap()
    }

    fn watermark_for_path(
        path: &Path,
        session_timestamp_utc: Option<DateTime<Utc>>,
    ) -> SessionWatermark {
        let metadata = fs::metadata(path).unwrap();
        SessionWatermark {
            path: path.display().to_string(),
            size_bytes: metadata.len(),
            mtime_utc: DateTime::<Utc>::from(metadata.modified().unwrap()),
            session_timestamp_utc,
            latest_event_timestamp_utc: None,
        }
    }

    #[test]
    fn no_logs_at_all() {
        let tmp = setup_state_dir();
        let output = run_distill(tmp.path());
        assert!(output.contains("No new session logs to process"));
    }

    #[test]
    fn all_logs_older_than_last_distilled() {
        let tmp = setup_state_dir();
        write_log(tmp.path(), 2026, 1, 1, 10, "old", "old content");
        set_last_distilled(tmp.path(), 2026, 6, 1, 0);

        let output = run_distill(tmp.path());
        assert!(output.contains("No new session logs to process"));
    }

    #[test]
    fn log_with_timestamp_equal_to_last_distilled_is_included() {
        let tmp = setup_state_dir();
        write_log(tmp.path(), 2026, 6, 1, 0, "exact", "exact content");
        set_last_distilled(tmp.path(), 2026, 6, 1, 0);

        let output = run_distill(tmp.path());
        assert!(output.contains("exact content"));
    }

    #[test]
    fn log_with_timestamp_after_last_distilled_is_included() {
        let tmp = setup_state_dir();
        write_log(tmp.path(), 2026, 7, 1, 0, "new", "new content");
        set_last_distilled(tmp.path(), 2026, 6, 1, 0);

        let output = run_distill(tmp.path());
        assert!(output.contains("new content"));
    }

    #[test]
    fn multiple_logs_in_chronological_order() {
        let tmp = setup_state_dir();
        write_log(tmp.path(), 2026, 3, 1, 0, "second", "BBB");
        write_log(tmp.path(), 2026, 1, 1, 0, "first", "AAA");
        write_log(tmp.path(), 2026, 5, 1, 0, "third", "CCC");

        let output = run_distill(tmp.path());
        let pos_a = output.find("AAA").unwrap();
        let pos_b = output.find("BBB").unwrap();
        let pos_c = output.find("CCC").unwrap();
        assert!(pos_a < pos_b);
        assert!(pos_b < pos_c);
    }

    #[test]
    fn each_log_has_filename_header() {
        let tmp = setup_state_dir();
        write_log(tmp.path(), 2026, 1, 1, 0, "sess1", "content1");

        let output = run_distill(tmp.path());
        assert!(
            output.contains("<session source=\"claude\" file=\"20260101T000000Z-sess1.jsonl\">")
        );
    }

    #[test]
    fn non_json_content_preserved_verbatim() {
        let tmp = setup_state_dir();
        let original = "line one\n  indented\n\nlast line\n";
        write_log(tmp.path(), 2026, 1, 1, 0, "sess1", original);

        let output = run_distill(tmp.path());
        assert!(output.contains(original));
    }

    #[test]
    fn unparseable_filenames_silently_skipped() {
        let tmp = setup_state_dir();
        write_log(tmp.path(), 2026, 1, 1, 0, "good", "good content");

        let bad_path = paths::logs_dir(tmp.path()).join("not-a-log.txt");
        fs::write(bad_path, "bad").unwrap();

        let output = run_distill(tmp.path());
        assert!(output.contains("good content"));
        assert!(!output.contains("bad"));
    }

    #[test]
    fn output_includes_writing_guidelines() {
        let tmp = setup_state_dir();
        write_log(tmp.path(), 2026, 1, 1, 0, "sess1", "content");

        let output = run_distill(tmp.path());
        assert!(output.contains("Soul-writing guidelines"));
    }

    #[test]
    fn guidelines_appear_before_logs() {
        let tmp = setup_state_dir();
        write_log(tmp.path(), 2026, 1, 1, 0, "sess1", "content");

        let output = run_distill(tmp.path());
        let guidelines_pos = output.find("Soul-writing guidelines").unwrap();
        let log_pos = output
            .find("<session source=\"claude\" file=\"20260101T000000Z-sess1.jsonl\">")
            .unwrap();
        assert!(guidelines_pos < log_pos);
    }

    #[test]
    fn no_guidelines_when_no_logs() {
        let tmp = setup_state_dir();
        let output = run_distill(tmp.path());
        assert!(!output.contains("Soul-writing guidelines"));
    }

    #[test]
    fn output_includes_data_preamble() {
        let tmp = setup_state_dir();
        write_log(tmp.path(), 2026, 1, 1, 0, "sess1", "content");

        let output = run_distill(tmp.path());
        assert!(output.contains("HISTORICAL DATA"));
        assert!(output.contains("<session-transcripts>"));
    }

    #[test]
    fn data_preamble_between_guidelines_and_sessions() {
        let tmp = setup_state_dir();
        write_log(tmp.path(), 2026, 1, 1, 0, "sess1", "content");

        let output = run_distill(tmp.path());
        let guidelines_pos = output.find("Soul-writing guidelines").unwrap();
        let preamble_pos = output.find("HISTORICAL DATA").unwrap();
        // Find the standalone tag, not the reference inside the preamble text.
        let session_pos = output.find("\n<session-transcripts>\n").unwrap();
        assert!(guidelines_pos < preamble_pos);
        assert!(preamble_pos < session_pos);
    }

    #[test]
    fn sessions_wrapped_in_xml_tags() {
        let tmp = setup_state_dir();
        write_log(tmp.path(), 2026, 1, 1, 0, "s1", "content1");
        write_log(tmp.path(), 2026, 2, 1, 0, "s2", "content2");

        let output = run_distill(tmp.path());
        assert!(output.contains("<session source=\"claude\" file=\"20260101T000000Z-s1.jsonl\">"));
        assert!(output.contains("<session source=\"claude\" file=\"20260201T000000Z-s2.jsonl\">"));
        assert!(output.contains("</session>"));
        assert!(output.contains("</session-transcripts>"));
    }

    #[test]
    fn no_xml_tags_when_no_logs() {
        let tmp = setup_state_dir();
        let output = run_distill(tmp.path());
        assert!(!output.contains("<session-transcripts>"));
        assert!(!output.contains("HISTORICAL DATA"));
    }

    #[test]
    fn external_claude_session_is_emitted_and_staged() {
        let tmp = setup_state_dir();
        let claude_home = tempfile::tempdir().unwrap();
        let session_id = "0198fb08-e6e4-7a41-8b3f-2fc8a9ee215d";
        write_claude_session(
            claude_home.path(),
            "proj",
            session_id,
            &[claude_user_line("external hello", "2026-07-01T12:00:00Z")],
        );
        set_last_distilled(tmp.path(), 2026, 1, 1, 0);

        let output = run_distill_with_claude_home(tmp.path(), claude_home.path(), false);

        assert!(output.contains("external hello"));
        assert!(output.contains(&format!(
            "<session source=\"claude\" file=\"projects/proj/{session_id}.jsonl\">"
        )));
        let state = LeiterState::load(&paths::state_path(tmp.path())).unwrap();
        assert!(state.claude.pending.contains_key(session_id));
    }

    #[test]
    fn external_claude_dry_run_does_not_stage_pending() {
        let tmp = setup_state_dir();
        let claude_home = tempfile::tempdir().unwrap();
        let session_id = "0198fb08-e6e4-7a41-8b3f-2fc8a9ee215d";
        write_claude_session(
            claude_home.path(),
            "proj",
            session_id,
            &[claude_user_line("dry hello", "2026-07-01T12:00:00Z")],
        );
        set_last_distilled(tmp.path(), 2026, 1, 1, 0);

        let output = run_distill_with_claude_home(tmp.path(), claude_home.path(), true);

        assert!(output.contains("dry hello"));
        let state = LeiterState::load(&paths::state_path(tmp.path())).unwrap();
        assert!(state.claude.pending.is_empty());
    }

    #[test]
    fn external_claude_changed_session_reemits_after_mark_and_growth() {
        let tmp = setup_state_dir();
        let claude_home = tempfile::tempdir().unwrap();
        let session_id = "0198fb08-e6e4-7a41-8b3f-2fc8a9ee215d";
        write_claude_session(
            claude_home.path(),
            "proj",
            session_id,
            &[claude_user_line("first external", "2026-07-01T12:00:00Z")],
        );
        set_last_distilled(tmp.path(), 2026, 1, 1, 0);

        let first = run_distill_with_claude_home(tmp.path(), claude_home.path(), false);
        assert!(first.contains("first external"));
        crate::commands::mark_distilled::run(tmp.path(), &mut Vec::new()).unwrap();

        write_claude_session(
            claude_home.path(),
            "proj",
            session_id,
            &[
                claude_user_line("first external", "2026-07-01T12:00:00Z"),
                claude_user_line("second external", "2026-07-01T12:00:01Z"),
            ],
        );

        let second = run_distill_with_claude_home(tmp.path(), claude_home.path(), false);
        assert!(second.contains("first external"));
        assert!(second.contains("second external"));
    }

    #[test]
    fn external_claude_resumed_files_emit_as_independent_sessions() {
        let tmp = setup_state_dir();
        let claude_home = tempfile::tempdir().unwrap();
        let first_id = "0198fb08-e6e4-7a41-8b3f-2fc8a9ee215d";
        let second_id = "1198fb08-e6e4-7a41-8b3f-2fc8a9ee215d";
        write_claude_session(
            claude_home.path(),
            "proj",
            first_id,
            &[claude_user_line("shared history", "2026-07-01T12:00:00Z")],
        );
        write_claude_session(
            claude_home.path(),
            "proj",
            second_id,
            &[
                claude_user_line("shared history", "2026-07-01T12:00:00Z"),
                claude_user_line("resumed tail", "2026-07-01T13:00:00Z"),
            ],
        );
        set_last_distilled(tmp.path(), 2026, 1, 1, 0);

        let output = run_distill_with_claude_home(tmp.path(), claude_home.path(), false);

        assert_eq!(output.matches("<session source=\"claude\"").count(), 2);
        assert!(output.contains(&format!("projects/proj/{first_id}.jsonl")));
        assert!(output.contains(&format!("projects/proj/{second_id}.jsonl")));
    }

    #[test]
    fn external_claude_dedupes_matching_legacy_log_only() {
        let tmp = setup_state_dir();
        let claude_home = tempfile::tempdir().unwrap();
        let external_id = "0198fb08-e6e4-7a41-8b3f-2fc8a9ee215d";
        write_claude_session(
            claude_home.path(),
            "proj",
            external_id,
            &[claude_user_line(
                "external copy wins",
                "2026-07-01T12:00:00Z",
            )],
        );
        write_log(
            tmp.path(),
            2026,
            7,
            1,
            12,
            external_id,
            "legacy duplicate loses",
        );
        write_log(tmp.path(), 2026, 7, 1, 13, "legacy-only", "legacy survives");
        set_last_distilled(tmp.path(), 2026, 1, 1, 0);

        let output = run_distill_with_claude_home(tmp.path(), claude_home.path(), false);

        assert!(output.contains("external copy wins"));
        assert!(output.contains("projects/proj/0198fb08-e6e4-7a41-8b3f-2fc8a9ee215d.jsonl"));
        assert!(!output.contains("legacy duplicate loses"));
        assert!(output.contains("legacy survives"));
        assert!(output.contains("20260701T130000Z-legacy-only.jsonl"));
    }

    #[test]
    fn external_claude_empty_content_is_staged_but_not_emitted() {
        let tmp = setup_state_dir();
        let claude_home = tempfile::tempdir().unwrap();
        let session_id = "0198fb08-e6e4-7a41-8b3f-2fc8a9ee215d";
        write_claude_session(
            claude_home.path(),
            "proj",
            session_id,
            &[claude_progress_line()],
        );
        set_last_distilled(tmp.path(), 2026, 1, 1, 0);

        let output = run_distill_with_claude_home(tmp.path(), claude_home.path(), false);

        assert!(output.contains("No new session logs to process"));
        assert!(!output.contains(&format!("projects/proj/{session_id}.jsonl")));
        let state = LeiterState::load(&paths::state_path(tmp.path())).unwrap();
        assert!(state.claude.pending.contains_key(session_id));
    }

    #[test]
    fn external_and_legacy_claude_sessions_interleave_chronologically() {
        let tmp = setup_state_dir();
        let claude_home = tempfile::tempdir().unwrap();
        let early_id = "0198fb08-e6e4-7a41-8b3f-2fc8a9ee215d";
        let late_id = "1198fb08-e6e4-7a41-8b3f-2fc8a9ee215d";
        write_claude_session(
            claude_home.path(),
            "proj",
            early_id,
            &[claude_user_line("external T1", "2026-07-01T11:00:00Z")],
        );
        write_log(tmp.path(), 2026, 7, 1, 12, "legacy-t2", "legacy T2");
        write_claude_session(
            claude_home.path(),
            "proj",
            late_id,
            &[claude_user_line("external T3", "2026-07-01T13:00:00Z")],
        );
        set_last_distilled(tmp.path(), 2026, 1, 1, 0);

        let output = run_distill_with_claude_home(tmp.path(), claude_home.path(), false);
        let early_pos = output.find("external T1").unwrap();
        let legacy_pos = output.find("legacy T2").unwrap();
        let late_pos = output.find("external T3").unwrap();

        assert!(early_pos < legacy_pos);
        assert!(legacy_pos < late_pos);
    }

    #[test]
    fn committed_unchanged_external_session_suppresses_matching_legacy_log() {
        let tmp = setup_state_dir();
        let claude_home = tempfile::tempdir().unwrap();
        let session_id = "0198fb08-e6e4-7a41-8b3f-2fc8a9ee215d";
        let session_path = write_claude_session(
            claude_home.path(),
            "proj",
            session_id,
            &[claude_user_line(
                "external already committed",
                "2026-07-01T12:00:00Z",
            )],
        );
        write_log(
            tmp.path(),
            2026,
            7,
            1,
            12,
            session_id,
            "legacy duplicate should stay hidden",
        );
        set_last_distilled(tmp.path(), 2026, 1, 1, 0);
        update_state(tmp.path(), |state| {
            state.claude.committed.insert(
                session_id.to_string(),
                watermark_for_path(&session_path, Some(ts(2026, 7, 1, 12))),
            );
        });

        let output = run_distill_with_claude_home(tmp.path(), claude_home.path(), false);

        assert!(output.contains("No new session logs to process"));
        assert!(!output.contains("external already committed"));
        assert!(!output.contains("legacy duplicate should stay hidden"));
    }

    #[test]
    fn empty_external_session_still_suppresses_matching_legacy_log() {
        let tmp = setup_state_dir();
        let claude_home = tempfile::tempdir().unwrap();
        let session_id = "0198fb08-e6e4-7a41-8b3f-2fc8a9ee215d";
        write_claude_session(
            claude_home.path(),
            "proj",
            session_id,
            &[claude_progress_line()],
        );
        write_log(
            tmp.path(),
            2026,
            7,
            1,
            12,
            session_id,
            "legacy duplicate should be suppressed",
        );
        set_last_distilled(tmp.path(), 2026, 1, 1, 0);

        let output = run_distill_with_claude_home(tmp.path(), claude_home.path(), false);

        assert!(output.contains("No new session logs to process"));
        assert!(!output.contains("legacy duplicate should be suppressed"));
        let state = LeiterState::load(&paths::state_path(tmp.path())).unwrap();
        assert!(state.claude.pending.contains_key(session_id));
    }

    #[test]
    fn legacy_only_session_is_still_emitted_when_external_store_lacks_id() {
        let tmp = setup_state_dir();
        let claude_home = tempfile::tempdir().unwrap();
        let external_id = "0198fb08-e6e4-7a41-8b3f-2fc8a9ee215d";
        write_claude_session(
            claude_home.path(),
            "proj",
            external_id,
            &[claude_user_line("external present", "2026-07-01T12:00:00Z")],
        );
        write_log(tmp.path(), 2026, 7, 1, 13, "legacy-only", "legacy only");
        set_last_distilled(tmp.path(), 2026, 1, 1, 0);

        let output = run_distill_with_claude_home(tmp.path(), claude_home.path(), false);

        assert!(output.contains("external present"));
        assert!(output.contains("legacy only"));
        assert!(output.contains("20260701T130000Z-legacy-only.jsonl"));
    }

    #[test]
    fn unresolved_claude_home_clears_pending_on_non_dry_run_only() {
        let tmp = setup_state_dir();
        let stale_ts = ts(2026, 3, 7, 18);
        update_state(tmp.path(), |state| {
            state.claude.pending.insert(
                "stale".to_string(),
                SessionWatermark {
                    path: "/tmp/stale.jsonl".to_string(),
                    size_bytes: 12,
                    mtime_utc: stale_ts,
                    session_timestamp_utc: Some(stale_ts),
                    latest_event_timestamp_utc: None,
                },
            );
        });
        let before = Utc::now() - chrono::Duration::seconds(1);
        let mut out = Vec::new();
        distill_with_inputs(tmp.path(), None, None, false, &mut out, false).unwrap();
        let after = Utc::now() + chrono::Duration::seconds(1);

        let state = LeiterState::load(&paths::state_path(tmp.path())).unwrap();
        assert!(state.claude.pending.is_empty());
        let scan_started = state.pending_scan_started_utc.unwrap();
        assert!(scan_started >= before);
        assert!(scan_started <= after);

        update_state(tmp.path(), |state| {
            state.claude.pending.insert(
                "stale".to_string(),
                SessionWatermark {
                    path: "/tmp/stale.jsonl".to_string(),
                    size_bytes: 12,
                    mtime_utc: stale_ts,
                    session_timestamp_utc: Some(stale_ts),
                    latest_event_timestamp_utc: None,
                },
            );
        });
        let mut dry_out = Vec::new();
        distill_with_inputs(tmp.path(), None, None, false, &mut dry_out, true).unwrap();

        let dry_state = LeiterState::load(&paths::state_path(tmp.path())).unwrap();
        assert!(dry_state.claude.pending.contains_key("stale"));
    }

    #[test]
    fn mark_distilled_adopts_scan_start_staged_by_distill() {
        let tmp = setup_state_dir();
        let claude_home = tempfile::tempdir().unwrap();
        let mut out = Vec::new();
        distill_with_inputs(
            tmp.path(),
            Some(claude_home.path()),
            None,
            false,
            &mut out,
            false,
        )
        .unwrap();
        let staged = LeiterState::load(&paths::state_path(tmp.path()))
            .unwrap()
            .pending_scan_started_utc
            .unwrap();

        crate::commands::mark_distilled::run(tmp.path(), &mut Vec::new()).unwrap();

        let updated = LeiterState::load(&paths::state_path(tmp.path())).unwrap();
        assert_eq!(updated.last_distilled, staged);
        assert!(updated.pending_scan_started_utc.is_none());
    }

    #[test]
    fn missing_soul_errors() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(paths::logs_dir(tmp.path())).unwrap();

        let mut out = Vec::new();
        let claude_home = tempfile::tempdir().unwrap();
        let result = run(tmp.path(), &mut out, false, Some(claude_home.path()), None);
        assert!(result.is_err());
    }

    // --- filter_session_log unit tests ---

    fn filter(input: &str) -> String {
        let mut out = Vec::new();
        filter_session_log(input, &mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    fn jsonl_user(text: &str) -> String {
        serde_json::json!({"type": "user", "message": {"content": text}}).to_string()
    }

    fn jsonl_assistant_text(text: &str) -> String {
        serde_json::json!({
            "type": "assistant",
            "message": {"content": [{"type": "text", "text": text}]}
        })
        .to_string()
    }

    fn jsonl_assistant_tool_use() -> String {
        serde_json::json!({
            "type": "assistant",
            "message": {"content": [{"type": "tool_use", "id": "t1", "name": "Read", "input": {}}]}
        })
        .to_string()
    }

    fn jsonl_tool_result() -> String {
        serde_json::json!({
            "type": "user",
            "toolUseResult": {"tool_use_id": "t1"},
            "message": {"content": "file contents here"}
        })
        .to_string()
    }

    fn jsonl_progress() -> String {
        serde_json::json!({"type": "progress", "data": {"type": "agent_progress"}}).to_string()
    }

    #[test]
    fn filter_extracts_user_text() {
        let output = filter(&jsonl_user("hello world"));
        assert_eq!(output, "[user]: hello world\n");
    }

    #[test]
    fn filter_extracts_assistant_text() {
        let output = filter(&jsonl_assistant_text("here is my response"));
        assert_eq!(output, "[assistant]: here is my response\n");
    }

    #[test]
    fn filter_concatenates_multiple_text_blocks() {
        let line = serde_json::json!({
            "type": "assistant",
            "message": {"content": [
                {"type": "text", "text": "first part"},
                {"type": "tool_use", "id": "t1", "name": "Read", "input": {}},
                {"type": "text", "text": "second part"}
            ]}
        })
        .to_string();
        let output = filter(&line);
        assert_eq!(
            output,
            "[assistant]: first part\n\nsecond part\n[assistant tool]: Read\n"
        );
    }

    #[test]
    fn filter_drops_tool_results() {
        let output = filter(&jsonl_tool_result());
        assert_eq!(output, "");
    }

    #[test]
    fn filter_emits_tool_summary_for_tool_use_only_assistant() {
        let output = filter(&jsonl_assistant_tool_use());
        assert_eq!(output, "[assistant tool]: Read\n");
    }

    #[test]
    fn filter_drops_progress() {
        let output = filter(&jsonl_progress());
        assert_eq!(output, "");
    }

    #[test]
    fn filter_drops_system() {
        let line = serde_json::json!({"type": "system", "event": "init"}).to_string();
        assert_eq!(filter(&line), "");
    }

    #[test]
    fn filter_drops_file_history_snapshot() {
        let line = serde_json::json!({"type": "file-history-snapshot", "files": []}).to_string();
        assert_eq!(filter(&line), "");
    }

    #[test]
    fn filter_includes_unknown_type_as_raw() {
        let line = serde_json::json!({"type": "new_future_type", "data": 42}).to_string();
        let output = filter(&line);
        assert_eq!(output, format!("{line}\n"));
    }

    #[test]
    fn filter_includes_non_json_as_raw() {
        let output = filter("this is not json at all");
        assert_eq!(output, "this is not json at all\n");
    }

    #[test]
    fn filter_includes_json_without_type_as_raw() {
        let line = serde_json::json!({"foo": "bar"}).to_string();
        let output = filter(&line);
        assert_eq!(output, format!("{line}\n"));
    }

    #[test]
    fn filter_preserves_blank_lines() {
        let input = format!("{}\n\n{}", jsonl_user("hi"), jsonl_user("bye"));
        let output = filter(&input);
        assert_eq!(output, "[user]: hi\n\n[user]: bye\n");
    }

    #[test]
    fn filter_mixed_session() {
        let lines = [
            jsonl_user("help me with rust"),
            jsonl_assistant_text("Sure, I can help."),
            jsonl_assistant_tool_use(),
            jsonl_tool_result(),
            jsonl_progress(),
            jsonl_assistant_text("Here is the result."),
            jsonl_user("thanks"),
        ];
        let input = lines.join("\n");
        let output = filter(&input);
        assert_eq!(
            output,
            "[user]: help me with rust\n\
             [assistant]: Sure, I can help.\n\
             [assistant tool]: Read\n\
             [assistant]: Here is the result.\n\
             [user]: thanks\n"
        );
    }

    #[test]
    fn filter_drops_thinking_only_assistant() {
        let line = serde_json::json!({
            "type": "assistant",
            "message": {"content": [{"type": "thinking", "thinking": "let me think..."}]}
        })
        .to_string();
        assert_eq!(filter(&line), "");
    }

    #[test]
    fn filter_tool_summary_with_file_path() {
        let line = serde_json::json!({
            "type": "assistant",
            "message": {"content": [
                {"type": "tool_use", "id": "t1", "name": "Edit", "input": {"file_path": "src/main.rs"}}
            ]}
        })
        .to_string();
        assert_eq!(filter(&line), "[assistant tool]: Edit(src/main.rs)\n");
    }

    #[test]
    fn filter_tool_summary_with_command() {
        let line = serde_json::json!({
            "type": "assistant",
            "message": {"content": [
                {"type": "tool_use", "id": "t1", "name": "Bash", "input": {"command": "cargo test"}}
            ]}
        })
        .to_string();
        assert_eq!(filter(&line), "[assistant tool]: Bash(cargo test)\n");
    }

    #[test]
    fn filter_tool_summary_with_pattern() {
        let line = serde_json::json!({
            "type": "assistant",
            "message": {"content": [
                {"type": "tool_use", "id": "t1", "name": "Grep", "input": {"pattern": "fn main"}}
            ]}
        })
        .to_string();
        assert_eq!(filter(&line), "[assistant tool]: Grep(fn main)\n");
    }

    #[test]
    fn filter_tool_summary_no_key_param() {
        let line = serde_json::json!({
            "type": "assistant",
            "message": {"content": [
                {"type": "tool_use", "id": "t1", "name": "Agent", "input": {"prompt": "explore"}}
            ]}
        })
        .to_string();
        assert_eq!(filter(&line), "[assistant tool]: Agent\n");
    }

    #[test]
    fn filter_tool_summary_command_truncated() {
        let long_cmd = "x".repeat(200);
        let line = serde_json::json!({
            "type": "assistant",
            "message": {"content": [
                {"type": "tool_use", "id": "t1", "name": "Bash", "input": {"command": long_cmd}}
            ]}
        })
        .to_string();
        let output = filter(&line);
        let expected_truncated: String = "x".repeat(120);
        assert_eq!(
            output,
            format!("[assistant tool]: Bash({expected_truncated}...)\n")
        );
    }

    #[test]
    fn filter_tool_summary_multiple_tools() {
        let line = serde_json::json!({
            "type": "assistant",
            "message": {"content": [
                {"type": "tool_use", "id": "t1", "name": "Read", "input": {"file_path": "a.rs"}},
                {"type": "tool_use", "id": "t2", "name": "Bash", "input": {"command": "ls"}}
            ]}
        })
        .to_string();
        assert_eq!(
            filter(&line),
            "[assistant tool]: Read(a.rs)\n[assistant tool]: Bash(ls)\n"
        );
    }

    #[test]
    fn filter_tool_summary_file_path_takes_priority_over_command() {
        let line = serde_json::json!({
            "type": "assistant",
            "message": {"content": [
                {"type": "tool_use", "id": "t1", "name": "Edit", "input": {
                    "file_path": "src/lib.rs",
                    "command": "ignored"
                }}
            ]}
        })
        .to_string();
        assert_eq!(filter(&line), "[assistant tool]: Edit(src/lib.rs)\n");
    }

    #[test]
    fn filter_extracts_user_text_from_array_content() {
        let line = serde_json::json!({
            "type": "user",
            "message": {"content": [{"type": "text", "text": "hello from array"}]}
        })
        .to_string();
        assert_eq!(filter(&line), "[user]: hello from array\n");
    }

    // --- obsolete log cleanup tests ---

    #[test]
    fn obsolete_logs_deleted() {
        let tmp = setup_state_dir();
        write_log(tmp.path(), 2026, 1, 1, 10, "old", "old content");
        write_log(tmp.path(), 2026, 7, 1, 0, "new", "new content");
        set_last_distilled(tmp.path(), 2026, 6, 1, 0);

        let output = run_distill(tmp.path());
        assert!(output.contains("new content"));
        assert!(!output.contains("old content"));

        let remaining: Vec<_> = fs::read_dir(paths::logs_dir(tmp.path()))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(remaining.len(), 1);
        assert!(remaining[0].contains("new"));
    }

    #[test]
    fn obsolete_logs_dry_run_does_not_delete() {
        let tmp = setup_state_dir();
        write_log(tmp.path(), 2026, 1, 1, 10, "old", "old content");
        write_log(tmp.path(), 2026, 7, 1, 0, "new", "new content");
        set_last_distilled(tmp.path(), 2026, 6, 1, 0);

        let output = run_distill_dry(tmp.path());
        assert!(output.contains("would be deleted"));
        assert!(output.contains("old"));

        let remaining: Vec<_> = fs::read_dir(paths::logs_dir(tmp.path())).unwrap().collect();
        assert_eq!(remaining.len(), 2);
    }

    #[test]
    fn no_obsolete_logs_no_cleanup_output() {
        let tmp = setup_state_dir();
        write_log(tmp.path(), 2026, 7, 1, 0, "new", "new content");
        set_last_distilled(tmp.path(), 2026, 6, 1, 0);

        let output = run_distill(tmp.path());
        assert!(!output.contains("deleted"));
        assert!(!output.contains("would be deleted"));
        assert!(!output.contains("Obsolete"));
    }

    #[test]
    fn all_obsolete_no_new_logs() {
        let tmp = setup_state_dir();
        write_log(tmp.path(), 2026, 1, 1, 10, "old", "old content");
        set_last_distilled(tmp.path(), 2026, 6, 1, 0);

        let output = run_distill(tmp.path());
        assert!(output.contains("No new session logs to process"));

        let remaining: Vec<_> = fs::read_dir(paths::logs_dir(tmp.path())).unwrap().collect();
        assert_eq!(remaining.len(), 0);
    }

    #[test]
    fn unparseable_filenames_not_deleted() {
        let tmp = setup_state_dir();
        let bad_path = paths::logs_dir(tmp.path()).join("not-a-log.txt");
        fs::write(&bad_path, "bad").unwrap();
        set_last_distilled(tmp.path(), 2026, 6, 1, 0);

        run_distill(tmp.path());
        assert!(bad_path.exists(), "unparseable file must not be deleted");
    }

    #[test]
    fn log_at_last_distilled_not_deleted() {
        let tmp = setup_state_dir();
        write_log(tmp.path(), 2026, 6, 1, 0, "exact", "exact content");
        set_last_distilled(tmp.path(), 2026, 6, 1, 0);

        let output = run_distill(tmp.path());
        assert!(output.contains("exact content"));

        let remaining: Vec<_> = fs::read_dir(paths::logs_dir(tmp.path()))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(remaining.len(), 1);
        assert!(remaining[0].contains("exact"));
    }

    #[test]
    fn soft_epoch_mismatch_succeeds() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(paths::logs_dir(tmp.path())).unwrap();
        write_state_with_epochs(tmp.path(), SETUP_SOFT_EPOCH + 1, SETUP_HARD_EPOCH);
        write_log(tmp.path(), 2026, 1, 1, 0, "sess1", "log content");

        let mut out = Vec::new();
        let claude_home = tempfile::tempdir().unwrap();
        run(tmp.path(), &mut out, false, Some(claude_home.path()), None).unwrap();
        let output = bytes_to_string(out);
        assert!(output.contains("log content"));
    }

    #[test]
    fn hard_epoch_mismatch_new_state_errors() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(paths::logs_dir(tmp.path())).unwrap();
        write_state_with_epochs(tmp.path(), SETUP_SOFT_EPOCH, SETUP_HARD_EPOCH + 1);

        let mut out = Vec::new();
        let claude_home = tempfile::tempdir().unwrap();
        let err = run(tmp.path(), &mut out, false, Some(claude_home.path()), None).unwrap_err();
        assert!(
            err.to_string()
                .contains("binary is older than your soul file")
        );
    }

    #[test]
    fn hard_epoch_mismatch_old_state_errors() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(paths::logs_dir(tmp.path())).unwrap();
        write_state_with_epochs(
            tmp.path(),
            SETUP_SOFT_EPOCH,
            SETUP_HARD_EPOCH.saturating_sub(1),
        );

        let mut out = Vec::new();
        let claude_home = tempfile::tempdir().unwrap();
        let err = run(tmp.path(), &mut out, false, Some(claude_home.path()), None).unwrap_err();
        assert!(err.to_string().contains("leiter claude install"));
    }

    #[test]
    fn corrupt_state_errors() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(paths::logs_dir(tmp.path())).unwrap();
        fs::write(paths::soul_path(tmp.path()), "body\n").unwrap();
        fs::write(paths::state_path(tmp.path()), "not = valid = toml").unwrap();

        let mut out = Vec::new();
        let codex_home = tempfile::tempdir().unwrap();
        let result = distill_with_inputs(
            tmp.path(),
            None,
            Some(codex_home.path()),
            false,
            &mut out,
            false,
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("state file"));
    }

    fn write_codex_rollout(codex_home: &Path, rel: &str, lines: &[Value]) -> PathBuf {
        let path = codex_home.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut rendered = String::new();
        for line in lines {
            rendered.push_str(&serde_json::to_string(line).unwrap());
            rendered.push('\n');
        }
        fs::write(&path, rendered).unwrap();
        path
    }

    fn codex_session_meta(id: &str, ts: &str) -> Value {
        serde_json::json!({
            "timestamp": ts,
            "type": "session_meta",
            "payload": {
                "id": id,
                "timestamp": ts
            }
        })
    }

    #[test]
    fn codex_missing_is_best_effort() {
        let tmp = setup_state_dir();
        set_codex_enabled(tmp.path(), true);
        let codex_home = tempfile::tempdir().unwrap();
        fs::remove_dir_all(codex_home.path()).unwrap();

        let output = run_distill_with_codex_home(tmp.path(), codex_home.path(), false);
        assert!(output.contains("No new session logs to process"));
    }

    #[test]
    fn codex_live_and_archived_sessions_are_included() {
        let tmp = setup_state_dir();
        set_codex_enabled(tmp.path(), true);
        let codex_home = tempfile::tempdir().unwrap();
        write_codex_rollout(
            codex_home.path(),
            "sessions/2026/03/07/live.jsonl",
            &[
                codex_session_meta("live-sess", "2026-03-07T18:00:00Z"),
                serde_json::json!({
                    "timestamp": "2026-03-07T18:00:01Z",
                    "type": "response_item",
                    "payload": {
                        "type": "message",
                        "role": "user",
                        "content": [{"type": "input_text", "text": "live hello"}]
                    }
                }),
            ],
        );
        write_codex_rollout(
            codex_home.path(),
            "archived_sessions/archived.jsonl",
            &[
                codex_session_meta("archived-sess", "2026-03-07T19:00:00Z"),
                serde_json::json!({
                    "timestamp": "2026-03-07T19:00:01Z",
                    "type": "response_item",
                    "payload": {
                        "type": "message",
                        "role": "assistant",
                        "content": [{"type": "output_text", "text": "archived hello"}]
                    }
                }),
            ],
        );

        let output = run_distill_with_codex_home(tmp.path(), codex_home.path(), false);
        assert!(output.contains("live hello"));
        assert!(output.contains("archived hello"));
        assert!(
            output.contains("<session source=\"codex\" file=\"sessions/2026/03/07/live.jsonl\">")
        );
        assert!(
            output.contains("<session source=\"codex\" file=\"archived_sessions/archived.jsonl\">")
        );
    }

    #[test]
    fn unchanged_codex_session_not_emitted_twice_after_mark() {
        let tmp = setup_state_dir();
        set_codex_enabled(tmp.path(), true);
        let codex_home = tempfile::tempdir().unwrap();
        write_codex_rollout(
            codex_home.path(),
            "sessions/session.jsonl",
            &[
                codex_session_meta("sess", "2026-03-07T18:00:00Z"),
                serde_json::json!({
                    "timestamp": "2026-03-07T18:00:01Z",
                    "type": "response_item",
                    "payload": {
                        "type": "message",
                        "role": "user",
                        "content": [{"type": "input_text", "text": "hello once"}]
                    }
                }),
            ],
        );

        let first = run_distill_with_codex_home(tmp.path(), codex_home.path(), false);
        assert!(first.contains("hello once"));

        update_state(tmp.path(), |state| {
            let pending = std::mem::take(&mut state.codex.pending);
            state.codex.committed.extend(pending);
        });

        let second = run_distill_with_codex_home(tmp.path(), codex_home.path(), false);
        assert!(!second.contains("hello once"));
        assert!(second.contains("No new session logs to process"));
    }

    #[test]
    fn changed_codex_session_is_re_emitted_in_full() {
        let tmp = setup_state_dir();
        set_codex_enabled(tmp.path(), true);
        let codex_home = tempfile::tempdir().unwrap();
        let path = write_codex_rollout(
            codex_home.path(),
            "sessions/session.jsonl",
            &[
                codex_session_meta("sess", "2026-03-07T18:00:00Z"),
                serde_json::json!({
                    "timestamp": "2026-03-07T18:00:01Z",
                    "type": "response_item",
                    "payload": {
                        "type": "message",
                        "role": "user",
                        "content": [{"type": "input_text", "text": "hello once"}]
                    }
                }),
            ],
        );

        run_distill_with_codex_home(tmp.path(), codex_home.path(), false);
        update_state(tmp.path(), |state| {
            let pending = std::mem::take(&mut state.codex.pending);
            state.codex.committed.extend(pending);
        });

        write_codex_rollout(
            codex_home.path(),
            "sessions/session.jsonl",
            &[
                codex_session_meta("sess", "2026-03-07T18:00:00Z"),
                serde_json::json!({
                    "timestamp": "2026-03-07T18:00:01Z",
                    "type": "response_item",
                    "payload": {
                        "type": "message",
                        "role": "user",
                        "content": [{"type": "input_text", "text": "hello once"}]
                    }
                }),
                serde_json::json!({
                    "timestamp": "2026-03-07T18:00:02Z",
                    "type": "response_item",
                    "payload": {
                        "type": "message",
                        "role": "assistant",
                        "content": [{"type": "output_text", "text": "hello twice"}]
                    }
                }),
            ],
        );
        assert!(fs::metadata(&path).unwrap().len() > 0);

        let output = run_distill_with_codex_home(tmp.path(), codex_home.path(), false);
        assert!(output.contains("hello once"));
        assert!(output.contains("hello twice"));
    }

    #[test]
    fn dry_run_does_not_stage_codex_pending() {
        let tmp = setup_state_dir();
        set_codex_enabled(tmp.path(), true);
        let codex_home = tempfile::tempdir().unwrap();
        write_codex_rollout(
            codex_home.path(),
            "sessions/session.jsonl",
            &[codex_session_meta("sess", "2026-03-07T18:00:00Z")],
        );

        let _ = run_distill_with_codex_home(tmp.path(), codex_home.path(), true);
        let state = LeiterState::load(&paths::state_path(tmp.path())).unwrap();
        assert!(state.codex.pending.is_empty());
        assert!(!paths::codex_meta_path(tmp.path()).exists());
    }

    #[test]
    fn legacy_codex_meta_file_is_ignored() {
        let tmp = setup_state_dir();
        set_codex_enabled(tmp.path(), true);
        let codex_home = tempfile::tempdir().unwrap();
        write_codex_rollout(
            codex_home.path(),
            "sessions/session.jsonl",
            &[
                codex_session_meta("sess", "2026-03-07T18:00:00Z"),
                serde_json::json!({
                    "timestamp": "2026-03-07T18:00:01Z",
                    "type": "response_item",
                    "payload": {
                        "type": "message",
                        "role": "assistant",
                        "content": [{"type": "output_text", "text": "should skip"}]
                    }
                }),
            ],
        );
        fs::write(paths::codex_meta_path(tmp.path()), "version = 999\n").unwrap();

        let output = run_distill_with_codex_home(tmp.path(), codex_home.path(), false);
        assert!(output.contains("should skip"));
    }

    #[test]
    fn changed_codex_session_repeats_until_mark_distilled() {
        let tmp = setup_state_dir();
        set_codex_enabled(tmp.path(), true);
        let codex_home = tempfile::tempdir().unwrap();
        write_codex_rollout(
            codex_home.path(),
            "sessions/session.jsonl",
            &[
                codex_session_meta("sess", "2026-03-07T18:00:00Z"),
                serde_json::json!({
                    "timestamp": "2026-03-07T18:00:01Z",
                    "type": "response_item",
                    "payload": {
                        "type": "message",
                        "role": "assistant",
                        "content": [{"type": "output_text", "text": "repeat me"}]
                    }
                }),
            ],
        );

        let first = run_distill_with_codex_home(tmp.path(), codex_home.path(), false);
        let second = run_distill_with_codex_home(tmp.path(), codex_home.path(), false);
        assert!(first.contains("repeat me"));
        assert!(second.contains("repeat me"));
    }

    #[test]
    fn malformed_codex_file_is_skipped() {
        let tmp = setup_state_dir();
        set_codex_enabled(tmp.path(), true);
        let codex_home = tempfile::tempdir().unwrap();
        let bad_path = codex_home.path().join("sessions/bad.jsonl");
        fs::create_dir_all(bad_path.parent().unwrap()).unwrap();
        fs::write(&bad_path, "not json\n").unwrap();

        let output = run_distill_with_codex_home(tmp.path(), codex_home.path(), false);
        assert!(output.contains("No new session logs to process"));
    }

    #[test]
    fn codex_disabled_ignores_rollouts_and_metadata() {
        let tmp = setup_state_dir();
        let codex_home = tempfile::tempdir().unwrap();
        write_codex_rollout(
            codex_home.path(),
            "sessions/session.jsonl",
            &[
                codex_session_meta("sess", "2026-03-07T18:00:00Z"),
                serde_json::json!({
                    "timestamp": "2026-03-07T18:00:01Z",
                    "type": "response_item",
                    "payload": {
                        "type": "message",
                        "role": "assistant",
                        "content": [{"type": "output_text", "text": "should stay hidden"}]
                    }
                }),
            ],
        );

        let mut out = Vec::new();
        distill_with_inputs(
            tmp.path(),
            None,
            Some(codex_home.path()),
            false,
            &mut out,
            false,
        )
        .unwrap();
        let output = bytes_to_string(out);
        assert!(!output.contains("should stay hidden"));
        assert!(output.contains("No new session logs to process"));
        assert!(!paths::codex_meta_path(tmp.path()).exists());
        let state = LeiterState::load(&paths::state_path(tmp.path())).unwrap();
        assert!(state.codex.pending.is_empty());
    }
}
