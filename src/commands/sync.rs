//! `leiter sync` — re-materialize managed soul-delivery blocks.

use std::io::Write;
use std::path::Path;

use anyhow::{Result, bail};

use crate::config::load_config_best_effort;
use crate::paths;
use crate::sync::{SyncHomes, SyncOutcome, sync_blocks};
use crate::validation::{ValidationStatus, validate_state};

/// Run the explicit sync command.
///
/// Every enabled target is attempted, and each outcome is printed. Refusals
/// are surfaced as a command error after printing so automation does not miss
/// a hand-edited managed block.
///
/// `claude_home_override`/`codex_home_override` exist only for testability:
/// `leiter sync` has no `--claude-home`/`--codex-home` flags of its own — this
/// is the one command whose entire purpose is materializing the blocks, so
/// unlike the opportunistic-resync commands it must hard-fail (not warn and
/// skip) when a home cannot be resolved. Production callers (`main.rs`) pass
/// `None` for both, so [`paths::resolve_claude_home`]/[`paths::resolve_codex_home`]
/// fall through to the real defaults; only unit tests inject a temp
/// directory here.
pub fn run(
    state_dir: &Path,
    force: bool,
    out: &mut impl Write,
    claude_home_override: Option<&Path>,
    codex_home_override: Option<&Path>,
) -> Result<()> {
    let (mut state, soul) = match validate_state(state_dir) {
        ValidationStatus::Compatible { state, soul, .. } => (state, soul),
        ValidationStatus::Incompatible(reason) => bail!("{}", reason.user_message()),
    };

    let config = load_config_best_effort(state_dir);
    let claude_home = paths::resolve_claude_home(claude_home_override).ok_or_else(|| {
        anyhow::anyhow!("Claude home directory not found; set $HOME or pass an override")
    })?;
    let codex_home = paths::resolve_codex_home(codex_home_override, config.codex);
    if config.codex && codex_home.is_none() {
        bail!("Codex home directory not found; set $HOME or pass an override");
    }

    let outcomes = sync_blocks(
        state_dir,
        &mut state,
        &soul,
        SyncHomes {
            claude_home: Some(&claude_home),
            codex_home: codex_home.as_deref(),
        },
        config.codex,
        force,
    )?;

    print_outcomes(out, &outcomes)?;
    if outcomes.iter().any(SyncOutcome::is_refused) {
        bail!(
            "one or more managed blocks were refused; rerun `leiter sync --force` to overwrite hand edits"
        );
    }
    Ok(())
}

/// Print one contractual line per target outcome.
pub fn print_outcomes(out: &mut impl Write, outcomes: &[SyncOutcome]) -> Result<()> {
    for outcome in outcomes {
        writeln!(out, "{}", outcome.line())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::test_support::{bytes_to_string, setup_state_dir};

    /// Regression coverage for home-directory dependency injection: `run()`
    /// must write into the injected override, proving the command threads it
    /// through to `sync_blocks` rather than resolving a real `~/.claude`.
    #[test]
    fn run_creates_claude_md_in_injected_home() {
        let tmp = setup_state_dir();
        let claude_home = tempfile::tempdir().unwrap();

        let mut out = Vec::new();
        run(tmp.path(), false, &mut out, Some(claude_home.path()), None).unwrap();

        let output = bytes_to_string(out);
        assert!(output.contains("CLAUDE.md synced"));
        assert!(paths::claude_md_path(claude_home.path()).is_file());
    }

    #[test]
    fn run_reports_and_fails_on_hand_edited_block_without_force() {
        let tmp = setup_state_dir();
        let claude_home = tempfile::tempdir().unwrap();
        run(
            tmp.path(),
            false,
            &mut Vec::new(),
            Some(claude_home.path()),
            None,
        )
        .unwrap();
        let claude_md = paths::claude_md_path(claude_home.path());
        let tampered = std::fs::read_to_string(&claude_md)
            .unwrap()
            .replace("soul", "hand edited");
        std::fs::write(&claude_md, tampered).unwrap();

        let mut out = Vec::new();
        let err = run(tmp.path(), false, &mut out, Some(claude_home.path()), None).unwrap_err();

        assert!(err.to_string().contains("--force"));
        assert!(bytes_to_string(out).contains("refused"));
    }
}
