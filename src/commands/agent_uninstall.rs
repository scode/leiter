//! `leiter agent-uninstall` — outputs instructions to remove leiter hooks.
//!
//! This command outputs natural language instructions for the agent to remove
//! leiter hooks from `~/.claude/settings.json`. It makes no filesystem changes.

use std::io::Write;

use anyhow::Result;

use crate::templates::AGENT_UNINSTALL_INSTRUCTIONS;

pub fn run(out: &mut impl Write) -> Result<()> {
    write!(out, "{AGENT_UNINSTALL_INSTRUCTIONS}")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_uninstall() -> String {
        let mut out = Vec::new();
        run(&mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn output_contains_hook_detection_strings() {
        let output = run_uninstall();
        assert!(output.contains("leiter context"));
        assert!(output.contains("leiter nudge"));
        assert!(output.contains("leiter session-end"));
    }

    #[test]
    fn output_contains_cleanup_guidance() {
        let output = run_uninstall();
        assert!(output.contains("~/.leiter/"));
        assert!(output.contains("leiter agent-setup"));
    }
}
