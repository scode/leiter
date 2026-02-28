//! `leiter agent-uninstall` — outputs instructions to remove leiter hooks.
//!
//! This command outputs natural language instructions for the agent to remove
//! leiter hooks from `~/.claude/settings.json`. It makes no filesystem changes.

use std::io::Write;
use std::path::Path;

use anyhow::Result;

use crate::templates::agent_uninstall_instructions;

pub fn run(state_dir: &Path, out: &mut impl Write) -> Result<()> {
    write!(out, "{}", agent_uninstall_instructions(state_dir))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_uninstall() -> String {
        let mut out = Vec::new();
        run(Path::new("/test/state"), &mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn output_contains_hook_detection_strings() {
        let output = run_uninstall();
        assert!(output.contains("leiter hook context"));
        assert!(output.contains("leiter hook nudge"));
        assert!(output.contains("leiter session-end"));
    }

    #[test]
    fn output_contains_cleanup_guidance() {
        let output = run_uninstall();
        assert!(output.contains("/test/state/"));
        assert!(output.contains("leiter agent-setup"));
    }
}
