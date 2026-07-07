//! `leiter hook nudge` — silent SessionStart hook tombstone.
//!
//! The retired `--auto-distill` flag is still accepted by the CLI so old
//! `settings.json` hook entries do not start failing after a binary upgrade.
//! The command itself is intentionally inert: `leiter hook context` owns the
//! migration pointer, and hookless leiter has no session-start nudge behavior.

use std::io::Write;
use std::path::Path;

use anyhow::Result;

pub fn run(_state_dir: &Path, _out: &mut impl Write, _auto_distill: bool) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::test_support::bytes_to_string;

    fn run_nudge(auto_distill: bool) -> String {
        let tmp = tempfile::tempdir().unwrap();
        let mut out = Vec::new();
        run(tmp.path(), &mut out, auto_distill).unwrap();
        bytes_to_string(out)
    }

    #[test]
    fn nudge_without_auto_distill_outputs_nothing() {
        let output = run_nudge(false);
        assert!(output.is_empty());
    }

    #[test]
    fn nudge_with_auto_distill_outputs_nothing() {
        let output = run_nudge(true);
        assert!(output.is_empty());
    }
}
