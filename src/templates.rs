//! Agent-facing text constants and the soul template.
//!
//! All natural language that leiter outputs for the agent to read or act on
//! lives here. Keeping it in one place makes it easy to review the agent's
//! "interface" holistically and evolve it across versions.

/// Current soul template version. Bumped whenever the template structure
/// changes, so `leiter soul-upgrade` can detect drift.
pub const SOUL_TEMPLATE_VERSION: u32 = 1;

/// Initial content for `~/.leiter/soul.md` (body only, no frontmatter).
///
/// Section headings guide the agent toward capturing specific kinds of
/// preferences. The agent fills these in over time by editing the soul file
/// directly.
pub const SOUL_TEMPLATE: &str = "\
# Communication Style

How the user prefers to receive information — conciseness, level of detail,
tone, use of examples, etc.

# Coding Preferences

Language-specific conventions, formatting choices, naming patterns, preferred
libraries, and architectural patterns the user favors.

# Workflow Patterns

How the user works with Claude Code — preferred order of operations, when to
ask vs. act, commit and PR habits, testing expectations, etc.

# Tool Preferences

Which tools and commands the user prefers (e.g., specific test runners, build
systems, editors, shell commands). Tools to avoid.

# Project Context

Recurring project-specific knowledge — repo layouts, deployment targets,
important dependencies, domain terminology.

# Corrections and Lessons

Things the user has corrected or explicitly taught. Record these so the same
mistakes are not repeated.
";

/// Version changelog for the soul template. Each entry describes what changed
/// in that version, so `leiter soul-upgrade` can show the agent what to migrate.
pub const SOUL_TEMPLATE_CHANGELOG: &[(u32, &str)] = &[(
    1,
    "Initial soul template with sections for communication style, coding preferences, workflow patterns, tool preferences, project context, and corrections.",
)];

/// Preamble injected before the soul content by `leiter context`.
///
/// Covers all the topics the spec requires: identity, soul file location,
/// when to edit the soul, session logging, distillation, and soul upgrade.
pub const CONTEXT_PREAMBLE: &str = "\
Leiter is a self-training system that learns your preferences across Claude Code sessions.

Your soul file is at `~/.leiter/soul.md`. Use your Read/Edit/Write tools to modify it directly.

When the user says \"remember\", \"learn\", \"always\", \"never\", or similar preference-setting language, read the soul file, find the appropriate section, and add the preference. No CLI command is needed — edit the file directly.

You will be prompted to write a session log when this session ends (via the stop hook). You do not need to do anything proactively — the prompt will include instructions and the session ID.

When the user asks to distill session logs, run `leiter distill`. This outputs unprocessed session logs. Read through them, update the soul with new learnings, then update `last_distilled` in the soul file's frontmatter to the current UTC ISO 8601 timestamp (e.g., 2026-02-23T17:00:00Z).

When the user asks to upgrade the leiter soul, run `leiter soul-upgrade`. If the soul template is outdated, this outputs migration instructions and the new template. Follow the instructions to restructure the soul while preserving all learned preferences.

IMPORTANT: All `leiter` CLI commands that write to the filesystem (especially `leiter log`) must be run outside the sandbox (i.e., with sandbox disabled). The leiter state directory (`~/.leiter/`) is outside the default sandbox allowed paths and writes will fail with \"Operation not permitted\" otherwise.

---
";

/// Template for the stop hook blocking reason. `{session_id}` is replaced at
/// runtime with the actual session ID from the hook input.
pub const STOP_HOOK_PROMPT_TEMPLATE: &str = "\
Before stopping, please write a brief session log summarizing what was done in this session, any learnings for future sessions, and any challenges encountered. Pipe the log content to `leiter log --session-id {session_id}` (run outside the sandbox — leiter writes to ~/.leiter/ which is not in the sandbox allowed paths). If you have already written a session log in this session, you may skip this step.";

/// Instructions output by `leiter agent-setup` telling the agent how to
/// configure Claude Code hooks in `~/.claude/settings.json`.
pub const AGENT_SETUP_INSTRUCTIONS: &str = r#"Configure Claude Code hooks for leiter by editing `~/.claude/settings.json`.

Read `~/.claude/settings.json` (or create it with `{}` if it doesn't exist).

Check whether leiter hooks are already present by looking for commands containing `"leiter context"` and `"leiter stop-hook"` in the existing hooks.

If leiter hooks are NOT already present, add the following hook groups to the `hooks` object. If `SessionStart` or `Stop` arrays already exist, append the leiter entries to those arrays (preserving all existing hooks). If they don't exist, create them.

SessionStart hook group to add:
```json
{
  "hooks": [
    {
      "type": "command",
      "command": "leiter context"
    }
  ]
}
```

Stop hook group to add:
```json
{
  "hooks": [
    {
      "type": "command",
      "command": "leiter stop-hook"
    }
  ]
}
```

If leiter hooks are already present, skip and report that hooks are already configured.

Use your Edit tool to make the changes to `~/.claude/settings.json`.
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn soul_template_version_is_positive() {
        assert!(SOUL_TEMPLATE_VERSION > 0);
    }

    #[test]
    fn soul_template_contains_expected_sections() {
        for heading in [
            "# Communication Style",
            "# Coding Preferences",
            "# Workflow Patterns",
            "# Tool Preferences",
            "# Project Context",
            "# Corrections and Lessons",
        ] {
            assert!(
                SOUL_TEMPLATE.contains(heading),
                "soul template missing section: {heading}"
            );
        }
    }

    #[test]
    fn changelog_has_entry_for_current_version() {
        assert!(
            SOUL_TEMPLATE_CHANGELOG
                .iter()
                .any(|(v, _)| *v == SOUL_TEMPLATE_VERSION),
            "changelog missing entry for version {SOUL_TEMPLATE_VERSION}"
        );
    }

    #[test]
    fn context_preamble_contains_required_literals() {
        for literal in ["~/.leiter/soul.md", "leiter distill", "leiter soul-upgrade"] {
            assert!(
                CONTEXT_PREAMBLE.contains(literal),
                "context preamble missing: {literal}"
            );
        }
    }

    #[test]
    fn stop_hook_prompt_contains_session_id_placeholder() {
        assert!(STOP_HOOK_PROMPT_TEMPLATE.contains("{session_id}"));
    }

    #[test]
    fn agent_setup_instructions_contain_hook_commands() {
        assert!(AGENT_SETUP_INSTRUCTIONS.contains("leiter context"));
        assert!(AGENT_SETUP_INSTRUCTIONS.contains("leiter stop-hook"));
    }

    #[test]
    fn agent_setup_instructions_contain_hook_json_structure() {
        assert!(AGENT_SETUP_INSTRUCTIONS.contains(r#""type": "command""#));
        assert!(AGENT_SETUP_INSTRUCTIONS.contains(r#""command": "leiter context""#));
        assert!(AGENT_SETUP_INSTRUCTIONS.contains(r#""command": "leiter stop-hook""#));
    }
}
