//! Sync engine for managed soul-delivery blocks.
//!
//! The engine owns the clobber guard: leiter only overwrites a present managed
//! block when the block still matches the hash it previously recorded, unless
//! the caller explicitly asks to force the write. Missing blocks are recreated
//! without force because no user-authored managed span is at risk.

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};

use crate::managed_block::{
    WriteOutcome, compose_block, read_current_block, sha256_hex, write_managed_block,
};
use crate::paths;
use crate::state::{LeiterState, SyncHashes};

/// One managed prompt target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncTarget {
    /// Claude Code's `<claude_home>/CLAUDE.md`.
    ClaudeMd,
    /// Codex's `<codex_home>/AGENTS.md`.
    AgentsMd,
}

impl SyncTarget {
    /// Human-readable target label used in CLI output.
    pub fn label(self) -> &'static str {
        match self {
            Self::ClaudeMd => "CLAUDE.md",
            Self::AgentsMd => "AGENTS.md",
        }
    }
}

/// Outcome for one target sync attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncOutcome {
    /// The recorded soul hash and on-disk block already match.
    AlreadyCurrent { target: SyncTarget, path: PathBuf },
    /// A stale or missing block was written and state hashes were updated.
    Synced { target: SyncTarget, path: PathBuf },
    /// A missing block was recreated. This is operationally a sync, but it is
    /// useful to distinguish in tests and future status output.
    Recreated { target: SyncTarget, path: PathBuf },
    /// The target had a managed block that did not match the recorded hash.
    Refused { target: SyncTarget, path: PathBuf },
}

impl SyncOutcome {
    /// Whether this target refused to overwrite a hand-edited block.
    pub fn is_refused(&self) -> bool {
        matches!(self, Self::Refused { .. })
    }

    /// Contractual CLI line for this target.
    pub fn line(&self) -> String {
        match self {
            Self::AlreadyCurrent { target, path } => {
                format!("{} already current ({})", target.label(), path.display())
            }
            Self::Synced { target, path } | Self::Recreated { target, path } => {
                format!("{} synced ({})", target.label(), path.display())
            }
            Self::Refused { target, path } => format!(
                "{} refused: managed block was hand-edited in {}; rerun `leiter sync --force` to overwrite it",
                target.label(),
                path.display()
            ),
        }
    }
}

/// Homes used to locate the managed prompt files.
pub struct SyncHomes<'a> {
    /// Claude Code home. `None` for callers that only touch the Codex target
    /// (e.g. `leiter codex install`), so they never have to pass a placeholder
    /// path they do not actually mean. A `None` here skips the `CLAUDE.md`
    /// target even when the selection asks for it.
    pub claude_home: Option<&'a Path>,
    /// Codex home. Required only when the Codex target is enabled.
    pub codex_home: Option<&'a Path>,
}

/// Reconcile all enabled targets and persist hash updates once.
///
/// Blocks are written before `state.toml` is saved, so recorded hashes never
/// get ahead of the files they describe. A refused target does not stop other
/// targets from syncing; callers decide whether a refusal should fail the host
/// command.
pub fn sync_blocks(
    state_dir: &Path,
    state: &mut LeiterState,
    soul: &str,
    homes: SyncHomes<'_>,
    codex_enabled: bool,
    force: bool,
) -> Result<Vec<SyncOutcome>> {
    sync_blocks_selected(
        state_dir,
        state,
        soul,
        homes,
        SyncSelection {
            claude_md: true,
            agents_md: codex_enabled,
        },
        force,
    )
}

/// Reconcile selected targets and persist hash updates once.
pub fn sync_blocks_selected(
    state_dir: &Path,
    state: &mut LeiterState,
    soul: &str,
    homes: SyncHomes<'_>,
    selection: SyncSelection,
    force: bool,
) -> Result<Vec<SyncOutcome>> {
    let soul_path = paths::soul_path(state_dir);
    let desired_block = compose_block(&soul_path, soul);
    let soul_hash = sha256_hex(soul);
    let block_hash = sha256_hex(&desired_block);
    let desired_hashes = SyncHashes {
        soul_hash,
        block_hash,
    };

    let mut outcomes = Vec::new();
    let mut recorded_any = false;

    if selection.claude_md
        && let Some(claude_home) = homes.claude_home
    {
        let claude_path = paths::claude_md_path(claude_home);
        let (outcome, record) = sync_one(
            SyncTarget::ClaudeMd,
            &claude_path,
            state.sync.claude_md.as_ref(),
            &desired_hashes,
            &desired_block,
            force,
        )?;
        if record {
            state.sync.claude_md = Some(desired_hashes.clone());
            recorded_any = true;
        }
        outcomes.push(outcome);
    }

    if selection.agents_md
        && let Some(codex_home) = homes.codex_home
    {
        let agents_path = paths::agents_md_path(codex_home);
        // A hard failure here must not lose the hashes for a target that was
        // already written above: recorded state may lag reality (self-corrects
        // as a refusal next sync), but must never claim a write that did not
        // happen — so persist what is known before propagating the error.
        let result = sync_one(
            SyncTarget::AgentsMd,
            &agents_path,
            state.sync.agents_md.as_ref(),
            &desired_hashes,
            &desired_block,
            force,
        );
        let (outcome, record) = match result {
            Ok(pair) => pair,
            Err(err) => {
                if recorded_any {
                    state
                        .save(&paths::state_path(state_dir))
                        .context("while persisting hashes for targets written before a failure")?;
                }
                return Err(err);
            }
        };
        if record {
            state.sync.agents_md = Some(desired_hashes);
            recorded_any = true;
        }
        outcomes.push(outcome);
    }

    if recorded_any {
        state.save(&paths::state_path(state_dir))?;
    }

    Ok(outcomes)
}

/// Target mask used by command-specific sync paths.
#[derive(Debug, Clone, Copy)]
pub struct SyncSelection {
    /// Whether to reconcile `CLAUDE.md`.
    pub claude_md: bool,
    /// Whether to reconcile `AGENTS.md`.
    pub agents_md: bool,
}

/// Opportunistically heal targets whose recorded soul hash is stale.
///
/// This helper intentionally skips never-synced targets. Opportunistic sync is
/// a backstop for materialized blocks that leiter already owns; first delivery
/// still happens through install or explicit `leiter sync`.
pub fn opportunistic_resync(
    state_dir: &Path,
    state: &mut LeiterState,
    soul: &str,
    homes: SyncHomes<'_>,
    codex_enabled: bool,
) -> Result<Vec<SyncOutcome>> {
    let current_soul_hash = sha256_hex(soul);
    let claude_stale = state
        .sync
        .claude_md
        .as_ref()
        .is_some_and(|hashes| hashes.soul_hash != current_soul_hash);
    let agents_stale = codex_enabled
        && state
            .sync
            .agents_md
            .as_ref()
            .is_some_and(|hashes| hashes.soul_hash != current_soul_hash);

    if !claude_stale && !agents_stale {
        return Ok(Vec::new());
    }

    sync_blocks_selected(
        state_dir,
        state,
        soul,
        homes,
        SyncSelection {
            claude_md: claude_stale,
            agents_md: agents_stale,
        },
        false,
    )
}

/// Opportunistically re-sync, reporting refusals and failures as warnings.
///
/// The state-mutating commands that heal blocks as a side effect
/// (`config set`, `mark-distilled`, `mark-upgraded`, `codex install`) all share
/// this reporting shape: a refused (hand-edited) block prints a `warning:` line
/// but never fails the host command, and a hard sync error is likewise
/// downgraded to a warning so the primary operation still succeeds.
/// `leiter soul distill` deliberately does not use this — its post-session
/// context has no agent reading stdout, so it routes the same outcomes through
/// `tracing::warn!` instead.
pub fn resync_and_warn(
    out: &mut impl Write,
    state_dir: &Path,
    state: &mut LeiterState,
    soul: &str,
    homes: SyncHomes<'_>,
    codex_enabled: bool,
) -> Result<()> {
    match opportunistic_resync(state_dir, state, soul, homes, codex_enabled) {
        Ok(outcomes) => {
            for outcome in outcomes {
                if outcome.is_refused() {
                    writeln!(out, "warning: {}", outcome.line())?;
                }
            }
        }
        Err(err) => writeln!(out, "warning: opportunistic sync failed: {err}")?,
    }
    Ok(())
}

/// Reconcile one target. The bool in the result is "record the desired hashes
/// for this target" — set on writes, and also when the on-disk block already
/// equals the desired block but the recorded hashes do not (so an identical
/// restored block becomes owned without a pointless rewrite).
fn sync_one(
    target: SyncTarget,
    path: &Path,
    recorded: Option<&SyncHashes>,
    desired_hashes: &SyncHashes,
    desired_block: &str,
    force: bool,
) -> Result<(SyncOutcome, bool)> {
    let current_block = read_current_block(path)?;

    let Some(current_block) = current_block else {
        write_managed_block(path, desired_block)?;
        return Ok((
            SyncOutcome::Recreated {
                target,
                path: path.to_path_buf(),
            },
            true,
        ));
    };

    let current_block_hash = sha256_hex(&current_block);

    // Already-current is decided against the freshly composed DESIRED block,
    // not the recorded soul hash: a binary upgrade can change the preamble or
    // fence composition without any soul edit, and that must count as stale —
    // otherwise no amount of syncing (even --force) could refresh an old
    // preamble. This check runs before the clobber guard on purpose: a block
    // that already equals what leiter would write is current no matter how it
    // got there (e.g. a dotfiles restore of an identical block).
    if current_block_hash == desired_hashes.block_hash {
        let record = recorded != Some(desired_hashes);
        return Ok((
            SyncOutcome::AlreadyCurrent {
                target,
                path: path.to_path_buf(),
            },
            record,
        ));
    }

    let matches_recorded = recorded.is_some_and(|hashes| hashes.block_hash == current_block_hash);
    if !matches_recorded && !force {
        return Ok((
            SyncOutcome::Refused {
                target,
                path: path.to_path_buf(),
            },
            false,
        ));
    }

    let outcome = match write_managed_block(path, desired_block)? {
        WriteOutcome::Created | WriteOutcome::Appended => SyncOutcome::Recreated {
            target,
            path: path.to_path_buf(),
        },
        WriteOutcome::Replaced => SyncOutcome::Synced {
            target,
            path: path.to_path_buf(),
        },
    };
    Ok((outcome, true))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::LeiterState;

    fn initialized_state() -> LeiterState {
        LeiterState::fresh()
    }

    #[test]
    fn missing_block_recreates_without_force_and_records_hashes() {
        let state_tmp = tempfile::tempdir().unwrap();
        let claude_tmp = tempfile::tempdir().unwrap();
        let mut state = initialized_state();
        state.save(&paths::state_path(state_tmp.path())).unwrap();

        let outcomes = sync_blocks(
            state_tmp.path(),
            &mut state,
            "soul\n",
            SyncHomes {
                claude_home: Some(claude_tmp.path()),
                codex_home: None,
            },
            false,
            false,
        )
        .unwrap();

        assert!(matches!(outcomes[0], SyncOutcome::Recreated { .. }));
        assert!(state.sync.claude_md.is_some());
    }

    #[test]
    fn hand_edited_block_refuses_without_force_and_force_overwrites() {
        let state_tmp = tempfile::tempdir().unwrap();
        let claude_tmp = tempfile::tempdir().unwrap();
        let mut state = initialized_state();
        state.save(&paths::state_path(state_tmp.path())).unwrap();

        sync_blocks(
            state_tmp.path(),
            &mut state,
            "soul\n",
            SyncHomes {
                claude_home: Some(claude_tmp.path()),
                codex_home: None,
            },
            false,
            false,
        )
        .unwrap();
        let claude_md = paths::claude_md_path(claude_tmp.path());
        let tampered = std::fs::read_to_string(&claude_md)
            .unwrap()
            .replace("soul", "edited soul");
        std::fs::write(&claude_md, tampered).unwrap();

        let refused = sync_blocks(
            state_tmp.path(),
            &mut state,
            "new soul\n",
            SyncHomes {
                claude_home: Some(claude_tmp.path()),
                codex_home: None,
            },
            false,
            false,
        )
        .unwrap();
        assert!(matches!(refused[0], SyncOutcome::Refused { .. }));

        let forced = sync_blocks(
            state_tmp.path(),
            &mut state,
            "new soul\n",
            SyncHomes {
                claude_home: Some(claude_tmp.path()),
                codex_home: None,
            },
            false,
            true,
        )
        .unwrap();
        assert!(matches!(forced[0], SyncOutcome::Synced { .. }));
        assert!(
            std::fs::read_to_string(claude_md)
                .unwrap()
                .contains("new soul")
        );
    }

    #[test]
    fn present_differing_block_without_recorded_hash_refuses() {
        let state_tmp = tempfile::tempdir().unwrap();
        let claude_tmp = tempfile::tempdir().unwrap();
        let mut state = initialized_state();
        state.save(&paths::state_path(state_tmp.path())).unwrap();
        // A block composed from a DIFFERENT soul: present, unrecorded, and not
        // what leiter would write — the conservative refusal case.
        let block = compose_block(&paths::soul_path(state_tmp.path()), "some other soul\n");
        std::fs::write(paths::claude_md_path(claude_tmp.path()), block).unwrap();

        let outcomes = sync_blocks(
            state_tmp.path(),
            &mut state,
            "soul\n",
            SyncHomes {
                claude_home: Some(claude_tmp.path()),
                codex_home: None,
            },
            false,
            false,
        )
        .unwrap();

        assert!(matches!(outcomes[0], SyncOutcome::Refused { .. }));
        assert!(state.sync.claude_md.is_none());
    }

    /// The dotfiles-restore case: an on-disk block identical to what leiter
    /// would compose is adopted as current — hashes recorded, nothing written,
    /// no `--force` needed — because a byte-identical block is current no
    /// matter how it got there.
    #[test]
    fn present_identical_block_without_recorded_hash_is_adopted() {
        let state_tmp = tempfile::tempdir().unwrap();
        let claude_tmp = tempfile::tempdir().unwrap();
        let mut state = initialized_state();
        state.save(&paths::state_path(state_tmp.path())).unwrap();
        let block = compose_block(&paths::soul_path(state_tmp.path()), "soul\n");
        std::fs::write(paths::claude_md_path(claude_tmp.path()), &block).unwrap();

        let outcomes = sync_blocks(
            state_tmp.path(),
            &mut state,
            "soul\n",
            SyncHomes {
                claude_home: Some(claude_tmp.path()),
                codex_home: None,
            },
            false,
            false,
        )
        .unwrap();

        assert!(matches!(outcomes[0], SyncOutcome::AlreadyCurrent { .. }));
        let recorded = state.sync.claude_md.as_ref().expect("hashes adopted");
        assert_eq!(recorded.block_hash, sha256_hex(&block));
        let persisted = LeiterState::load(&paths::state_path(state_tmp.path())).unwrap();
        assert!(persisted.sync.claude_md.is_some());
    }

    /// A second sync of an unchanged soul against an unchanged template must be
    /// a pure no-op: the outcome is AlreadyCurrent and the file is not rewritten
    /// (content and mtime both unchanged).
    #[test]
    fn second_sync_unchanged_is_already_current_and_untouched() {
        let state_tmp = tempfile::tempdir().unwrap();
        let claude_tmp = tempfile::tempdir().unwrap();
        let mut state = initialized_state();
        state.save(&paths::state_path(state_tmp.path())).unwrap();

        sync_blocks(
            state_tmp.path(),
            &mut state,
            "soul\n",
            SyncHomes {
                claude_home: Some(claude_tmp.path()),
                codex_home: None,
            },
            false,
            false,
        )
        .unwrap();
        let claude_md = paths::claude_md_path(claude_tmp.path());
        let content_before = std::fs::read(&claude_md).unwrap();
        let mtime_before = std::fs::metadata(&claude_md).unwrap().modified().unwrap();

        let outcomes = sync_blocks(
            state_tmp.path(),
            &mut state,
            "soul\n",
            SyncHomes {
                claude_home: Some(claude_tmp.path()),
                codex_home: None,
            },
            false,
            false,
        )
        .unwrap();

        assert!(matches!(outcomes[0], SyncOutcome::AlreadyCurrent { .. }));
        assert_eq!(std::fs::read(&claude_md).unwrap(), content_before);
        assert_eq!(
            std::fs::metadata(&claude_md).unwrap().modified().unwrap(),
            mtime_before,
            "an already-current target must not be rewritten"
        );
    }

    /// A soul body containing backtick runs (including a quad run) must fence
    /// correctly, and a repeat sync of the same soul must recognize the block as
    /// current rather than churning it.
    #[test]
    fn soul_with_backtick_runs_syncs_then_is_already_current() {
        let state_tmp = tempfile::tempdir().unwrap();
        let claude_tmp = tempfile::tempdir().unwrap();
        let mut state = initialized_state();
        state.save(&paths::state_path(state_tmp.path())).unwrap();
        let soul = "```rust\nfn f() {}\n```\nand ```` a quad run\n";

        let first = sync_blocks(
            state_tmp.path(),
            &mut state,
            soul,
            SyncHomes {
                claude_home: Some(claude_tmp.path()),
                codex_home: None,
            },
            false,
            false,
        )
        .unwrap();
        assert!(matches!(
            first[0],
            SyncOutcome::Recreated { .. } | SyncOutcome::Synced { .. }
        ));

        let second = sync_blocks(
            state_tmp.path(),
            &mut state,
            soul,
            SyncHomes {
                claude_home: Some(claude_tmp.path()),
                codex_home: None,
            },
            false,
            false,
        )
        .unwrap();
        assert!(matches!(second[0], SyncOutcome::AlreadyCurrent { .. }));
    }

    /// A block whose recorded `soul_hash` still equals the current soul but
    /// whose composed bytes have drifted (here: a different embedded soul path,
    /// standing in for a binary preamble/fence change) must count as stale and
    /// re-sync — the already-current decision is against the desired block, not
    /// the soul hash alone.
    #[test]
    fn stale_desired_block_syncs_even_with_matching_soul_hash() {
        let state_tmp = tempfile::tempdir().unwrap();
        let claude_tmp = tempfile::tempdir().unwrap();
        let mut state = initialized_state();

        let stale_block = compose_block(Path::new("/elsewhere/soul.md"), "soul\n");
        let claude_md = paths::claude_md_path(claude_tmp.path());
        std::fs::write(&claude_md, &stale_block).unwrap();
        state.sync.claude_md = Some(SyncHashes {
            soul_hash: sha256_hex("soul\n"),
            block_hash: sha256_hex(&stale_block),
        });
        state.save(&paths::state_path(state_tmp.path())).unwrap();

        let outcomes = sync_blocks(
            state_tmp.path(),
            &mut state,
            "soul\n",
            SyncHomes {
                claude_home: Some(claude_tmp.path()),
                codex_home: None,
            },
            false,
            false,
        )
        .unwrap();

        assert!(
            matches!(outcomes[0], SyncOutcome::Synced { .. }),
            "outcome: {:?}",
            outcomes[0]
        );
        let desired = compose_block(&paths::soul_path(state_tmp.path()), "soul\n");
        assert_eq!(std::fs::read_to_string(&claude_md).unwrap(), desired);
    }

    /// Partial multi-target sync: a hand-edited CLAUDE.md is refused while a
    /// merely-stale AGENTS.md still syncs. The refused target's recorded hashes
    /// must stay put, the synced target's must advance, and both outcomes must
    /// be persisted to `state.toml`.
    #[test]
    fn partial_sync_refuses_hand_edit_but_syncs_other_and_persists() {
        let state_tmp = tempfile::tempdir().unwrap();
        let claude_tmp = tempfile::tempdir().unwrap();
        let codex_tmp = tempfile::tempdir().unwrap();
        let mut state = initialized_state();
        state.save(&paths::state_path(state_tmp.path())).unwrap();

        sync_blocks(
            state_tmp.path(),
            &mut state,
            "soul\n",
            SyncHomes {
                claude_home: Some(claude_tmp.path()),
                codex_home: Some(codex_tmp.path()),
            },
            true,
            false,
        )
        .unwrap();
        let recorded_claude_before = state.sync.claude_md.clone().unwrap();

        let claude_md = paths::claude_md_path(claude_tmp.path());
        let tampered = std::fs::read_to_string(&claude_md)
            .unwrap()
            .replace("soul", "hand edited");
        std::fs::write(&claude_md, tampered).unwrap();

        let outcomes = sync_blocks(
            state_tmp.path(),
            &mut state,
            "new soul\n",
            SyncHomes {
                claude_home: Some(claude_tmp.path()),
                codex_home: Some(codex_tmp.path()),
            },
            true,
            false,
        )
        .unwrap();

        assert!(
            outcomes.iter().any(|o| matches!(
                o,
                SyncOutcome::Refused {
                    target: SyncTarget::ClaudeMd,
                    ..
                }
            )),
            "CLAUDE.md should be refused: {outcomes:?}"
        );
        assert!(
            outcomes.iter().any(|o| matches!(
                o,
                SyncOutcome::Synced {
                    target: SyncTarget::AgentsMd,
                    ..
                }
            )),
            "AGENTS.md should sync: {outcomes:?}"
        );

        assert_eq!(
            state.sync.claude_md.as_ref().unwrap(),
            &recorded_claude_before
        );
        let persisted = LeiterState::load(&paths::state_path(state_tmp.path())).unwrap();
        assert_eq!(
            persisted.sync.claude_md.as_ref().unwrap(),
            &recorded_claude_before
        );
        assert_eq!(
            persisted.sync.agents_md.as_ref().unwrap().soul_hash,
            sha256_hex("new soul\n")
        );
    }

    /// Hard-failure flavor: CLAUDE.md is written first, then AGENTS.md's write
    /// fails hard (dangling symlink). The already-written target's hashes must
    /// still be persisted to disk before the error propagates, so recorded
    /// state never claims a write that did not happen — and never loses one that
    /// did.
    #[test]
    #[cfg(unix)]
    fn hard_failure_persists_earlier_target_hashes() {
        let state_tmp = tempfile::tempdir().unwrap();
        let claude_tmp = tempfile::tempdir().unwrap();
        let codex_tmp = tempfile::tempdir().unwrap();
        let mut state = initialized_state();
        state.save(&paths::state_path(state_tmp.path())).unwrap();

        let agents_md = paths::agents_md_path(codex_tmp.path());
        std::os::unix::fs::symlink(codex_tmp.path().join("missing.md"), &agents_md).unwrap();

        let err = sync_blocks(
            state_tmp.path(),
            &mut state,
            "soul\n",
            SyncHomes {
                claude_home: Some(claude_tmp.path()),
                codex_home: Some(codex_tmp.path()),
            },
            true,
            false,
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("dangling")
                || err.chain().any(|e| e.to_string().contains("dangling")),
            "error should stem from the dangling AGENTS.md link: {err:?}"
        );

        assert!(paths::claude_md_path(claude_tmp.path()).is_file());
        let persisted = LeiterState::load(&paths::state_path(state_tmp.path())).unwrap();
        assert!(
            persisted.sync.claude_md.is_some(),
            "CLAUDE.md hashes must survive the AGENTS.md failure"
        );
        assert!(persisted.sync.agents_md.is_none());
    }

    #[test]
    fn codex_target_is_gated() {
        let state_tmp = tempfile::tempdir().unwrap();
        let claude_tmp = tempfile::tempdir().unwrap();
        let codex_tmp = tempfile::tempdir().unwrap();
        let mut state = initialized_state();
        state.save(&paths::state_path(state_tmp.path())).unwrap();

        sync_blocks(
            state_tmp.path(),
            &mut state,
            "soul\n",
            SyncHomes {
                claude_home: Some(claude_tmp.path()),
                codex_home: Some(codex_tmp.path()),
            },
            false,
            false,
        )
        .unwrap();
        assert!(!paths::agents_md_path(codex_tmp.path()).exists());

        sync_blocks(
            state_tmp.path(),
            &mut state,
            "soul\n",
            SyncHomes {
                claude_home: Some(claude_tmp.path()),
                codex_home: Some(codex_tmp.path()),
            },
            true,
            false,
        )
        .unwrap();
        assert!(paths::agents_md_path(codex_tmp.path()).exists());
    }
}
