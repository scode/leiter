//! `leiter instill` — output soul-writing instructions for a user preference.
//!
//! Takes the user's preference as a positional argument and outputs a
//! self-contained instruction block: the quoted preference, shared
//! soul-writing guidelines, and an edit instruction. This ensures
//! consistent entry quality whether the agent learns inline or via
//! distillation.

use std::io::Write;
use std::path::Path;

use anyhow::Result;

use crate::templates::SOUL_WRITING_GUIDELINES;

/// Run the instill command.
///
/// Outputs the user's preference (quoted), the shared soul-writing
/// guidelines, and an instruction to edit `~/.leiter/soul.md`.
pub fn run(_home: &Path, out: &mut impl Write, text: &str) -> Result<()> {
    writeln!(out, "The user wants you to remember:\n")?;
    for line in text.lines() {
        writeln!(out, "> {line}")?;
    }
    writeln!(out)?;
    write!(out, "{SOUL_WRITING_GUIDELINES}")?;
    writeln!(
        out,
        "Now read `~/.leiter/soul.md` and edit the appropriate section following the guidelines above."
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn run_instill(text: &str) -> String {
        let home = PathBuf::from("/unused");
        let mut out = Vec::new();
        run(&home, &mut out, text).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn output_contains_quoted_preference() {
        let output = run_instill("always use snake_case");
        assert!(output.contains("> always use snake_case"));
    }

    #[test]
    fn output_contains_guidelines() {
        let output = run_instill("test preference");
        assert!(output.contains("Soul-writing guidelines"));
    }

    #[test]
    fn output_contains_edit_instruction() {
        let output = run_instill("test preference");
        assert!(output.contains("~/.leiter/soul.md"));
        assert!(output.contains("edit the appropriate section"));
    }

    #[test]
    fn output_contains_remember_preamble() {
        let output = run_instill("test preference");
        assert!(output.contains("The user wants you to remember"));
    }
}
