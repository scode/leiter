//! `leiter context` — inject soul content and agent instructions into the session.
//!
//! Called by the SessionStart hook on every session start. Outputs the preamble
//! (explaining how to interact with leiter) followed by the full soul file, so
//! the agent has all learned preferences in context.

use std::fs;
use std::io::Write;
use std::path::Path;

use anyhow::Result;

use crate::paths;
use crate::templates::CONTEXT_PREAMBLE;

/// Run the context command.
///
/// If the soul file exists, outputs the preamble then the soul content.
/// If it doesn't exist, outputs a message suggesting `leiter agent-setup`.
/// Either way, exits successfully — the SessionStart hook should never fail
/// the session.
pub fn run(home: &Path, out: &mut impl Write) -> Result<()> {
    let soul_path = paths::soul_path(home);

    if !soul_path.exists() {
        write!(out, "Leiter is not initialized. Run `leiter agent-setup` to set up.\n")?;
        return Ok(());
    }

    let soul_content = fs::read_to_string(&soul_path)?;
    write!(out, "{CONTEXT_PREAMBLE}{soul_content}")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::agent_setup;

    fn run_context(home: &Path) -> String {
        let mut out = Vec::new();
        run(home, &mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    fn setup_and_context(home: &Path) -> String {
        agent_setup::run(home, &mut Vec::new()).unwrap();
        run_context(home)
    }

    #[test]
    fn with_soul_output_starts_with_preamble() {
        let tmp = tempfile::tempdir().unwrap();
        let output = setup_and_context(tmp.path());
        assert!(output.starts_with(CONTEXT_PREAMBLE));
    }

    #[test]
    fn without_soul_suggests_agent_setup() {
        let tmp = tempfile::tempdir().unwrap();
        let output = run_context(tmp.path());
        assert!(output.contains("not initialized"));
        assert!(output.contains("leiter agent-setup"));
    }

    #[test]
    fn preamble_contains_required_elements() {
        let tmp = tempfile::tempdir().unwrap();
        let output = setup_and_context(tmp.path());
        assert!(output.contains("~/.leiter/soul.md"));
        assert!(output.contains("Read/Edit/Write"));
        assert!(output.contains("remember"));
        assert!(output.contains("session log"));
        assert!(output.contains("leiter distill"));
        assert!(output.contains("leiter soul-upgrade"));
    }

    #[test]
    fn soul_content_reproduced_verbatim() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        agent_setup::run(home, &mut Vec::new()).unwrap();

        let soul_content = fs::read_to_string(paths::soul_path(home)).unwrap();
        let output = run_context(home);

        assert!(output.ends_with(&soul_content));
    }
}
