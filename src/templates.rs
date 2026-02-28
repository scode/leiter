//! Agent-facing text templates and the soul template.
//!
//! All natural language that leiter outputs for the agent to read or act on
//! lives here. Keeping it in one place makes it easy to review the agent's
//! "interface" holistically and evolve it across versions.

use std::path::Path;

use crate::paths;

/// Current soul template version. Bumped whenever the template structure
/// changes, so `leiter soul-upgrade` can detect drift.
pub const SOUL_TEMPLATE_VERSION: u32 = 2;

/// Setup epoch for soft (nudge) compatibility checks. Only bumped when
/// a leiter upgrade introduces changes that benefit from user action but
/// are not strictly required.
pub const SETUP_SOFT_EPOCH: u32 = 1;

/// Setup epoch for hard (blocking) compatibility checks. Only bumped when
/// a leiter upgrade introduces changes that require user action before
/// the session can proceed.
pub const SETUP_HARD_EPOCH: u32 = 1;

/// Initial content for the soul file (body only, no frontmatter).
///
/// Section headings guide the agent toward capturing specific kinds of
/// preferences. The agent fills these in over time by editing the soul file
/// directly.
pub const SOUL_TEMPLATE: &str = "\
When new observations contradict existing entries, update the entry to reflect \
current behavior.

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

# Technology & Environment

Cross-project technology preferences — languages, frameworks, databases,
deployment targets, and infrastructure choices that apply across repos.

# What Works Well

Approaches, techniques, and interaction patterns that the user responds to
positively. Record these so they can be repeated.

# What to Avoid

Things the user has corrected, dislikes, or explicitly asked to stop.
Record these so the same mistakes are not repeated.
";

/// Version changelog for the soul template. Each entry describes what changed
/// in that version, so `leiter soul-upgrade` can show the agent what to migrate.
pub const SOUL_TEMPLATE_CHANGELOG: &[(u32, &str)] = &[
    (
        1,
        "Initial soul template with sections for communication style, coding preferences, workflow patterns, tool preferences, project context, and corrections.",
    ),
    (
        2,
        "Renamed 'Project Context' to 'Technology & Environment' (cross-project scope). Split 'Corrections and Lessons' into 'What Works Well' and 'What to Avoid'. Added lifecycle note: update entries when new observations contradict them.",
    ),
];

/// Guidelines for writing soul entries, shared by `leiter soul instill` and
/// `leiter distill`. Only emitted when the agent is actively writing to
/// the soul — never in the session preamble.
pub const SOUL_WRITING_GUIDELINES: &str = "\
## Soul-writing guidelines

Follow these rules when adding or updating entries in the soul file.

**Format:** Use concise bullets, one preference per bullet. Be specific and \
actionable — avoid vague statements.

**Placement:** Add each entry under the most appropriate section heading:
- Communication Style — tone, detail level, explanation preferences
- Coding Preferences — language conventions, patterns, libraries, architecture
- Workflow Patterns — order of operations, when to ask vs. act, commit habits
- Tool Preferences — specific tools, commands, runners, things to avoid
- Technology & Environment — cross-project stack choices (languages, frameworks, infra)
- What Works Well — approaches and patterns the user responds to positively
- What to Avoid — things the user corrected, dislikes, or asked to stop

**Contradiction resolution:** When a new preference contradicts an existing \
entry, update the existing entry to reflect the new behavior. Do not add a \
second conflicting entry. Do not remove entries just because they are old — \
only when they are contradicted.

**Examples of good entries:**

- Communication Style: `- Prefers concise responses; push back when wrong rather than agreeing.`
- Coding Preferences: `- Use snake_case for all Rust function and variable names.`
- Workflow Patterns: `- After running review swarm, fix feedback directly rather than asking.`
- Tool Preferences: `- Always use cargo-nextest instead of cargo test.`
- Technology & Environment: `- Default to PostgreSQL for relational data.`
- What Works Well: `- Presenting 2-3 options with a recommendation works better than open-ended questions.`
- What to Avoid: `- Do not add inline comments unless the logic is non-obvious.`
";

/// Preamble injected before the soul content by `leiter hook context`.
///
/// Covers all the topics the spec requires: identity, soul file location,
/// when to instill preferences, automatic transcript saving, distillation, and soul upgrade.
pub fn context_preamble(state_dir: &Path) -> String {
    let soul = paths::soul_path(state_dir).display().to_string();
    let dir = state_dir.display();
    format!(
        "Leiter is a self-training system that learns your preferences across Claude Code sessions.\n\
         \n\
         Your soul file is at `{soul}`. Use your Read/Edit/Write tools to modify it directly.\n\
         \n\
         When the user says \"remember\", \"learn\", \"instill\", \"always\", \"never\", or similar preference-setting language, run `leiter soul instill \"<what the user wants remembered>\"` and follow the instructions it outputs.\n\
         \n\
         Session transcripts are saved automatically when each session ends. No manual logging needed.\n\
         \n\
         When the user asks to distill session logs, run `leiter distill`. This outputs new session logs. Read through them, update the soul with new learnings, then update `last_distilled` in the soul file's frontmatter to the current UTC ISO 8601 timestamp (e.g., 2026-02-23T17:00:00Z).\n\
         \n\
         When the user asks to upgrade the leiter soul, run `leiter soul-upgrade`. If the soul template is outdated, this outputs migration instructions and the new template. Follow the instructions to restructure the soul while preserving all learned preferences.\n\
         \n\
         IMPORTANT: The `leiter hook session-end` hook command writes to `{dir}/` which is outside the default sandbox allowed paths. Ensure it is run outside the sandbox (i.e., with sandbox disabled) or writes will fail with \"Operation not permitted\".\n\
         \n\
         ---\n"
    )
}

/// Nudge message output by `leiter hook nudge` when stale undistilled logs exist.
pub const NUDGE_MESSAGE: &str = "\
ACTION REQUIRED: There are undistilled leiter session logs older than 24 hours. Before responding to the user's first message, briefly let them know by saying something like: \"You have undistilled leiter session logs. Want me to run a distillation?\" Keep it to one short sentence — do not explain what distillation is.
";

/// Migration instructions output by `leiter soul-upgrade` when the soul
/// template is outdated. Tells the agent how to restructure the existing
/// soul content into the new template format.
pub fn soul_upgrade_instructions(state_dir: &Path) -> String {
    let soul = paths::soul_path(state_dir).display().to_string();
    format!(
        "\
## Migration instructions

Rewrite `{soul}` to match the new template structure above.

Rules:
1. Read the current soul file completely before making any changes.
2. Every existing entry must appear in the rewritten soul. Do not drop, \
summarize, or merge entries unless they are exact duplicates.
3. Move each entry to the section where it best fits in the new template. \
If an entry fits multiple sections, place it in the most specific one.
4. Preserve the original wording of each entry. Do not rephrase or \
\"improve\" entries during migration — the meaning must be identical.
5. If an existing section has no equivalent in the new template, keep the \
entries and place them in the closest matching new section.
6. After rewriting, update `soul_version` in the frontmatter to the current \
version.
7. Do not add new entries or remove the section description placeholders \
from empty sections.
"
    )
}

/// Instructions output by `leiter setup uninstall` telling the agent how to
/// remove leiter hooks from `~/.claude/settings.json`.
pub fn agent_uninstall_instructions(state_dir: &Path) -> String {
    let dir = state_dir.display();
    format!(
        r#"Remove leiter hooks from Claude Code by editing `~/.claude/settings.json`.

Read `~/.claude/settings.json`.

Find and remove all hook entries whose `command` field contains `"leiter hook context"`, `"leiter hook nudge"`, or `"leiter hook session-end"`.

If removing leiter hooks causes a hook group object's `hooks` array to become empty, remove the entire group object from its parent array (e.g., from the `SessionStart` or `SessionEnd` array).

If a `SessionStart` or `SessionEnd` array becomes empty after removing all its groups, remove that key from the `hooks` object entirely.

Preserve all non-leiter hooks exactly as they are.

Use your Edit tool to make the changes to `~/.claude/settings.json`.

If no leiter hooks are found, report that leiter hooks are already removed.

After removing hooks, tell the user:
- To completely remove leiter, delete `{dir}/` (this removes the soul and all session logs) and uninstall the binary.
- To re-enable leiter later, paste the following into a Claude Code session: Run the shell command `leiter setup install` and follow the instructions it outputs on stdout.
"#
    )
}

/// Instructions output by `leiter setup install` telling the agent how to
/// configure Claude Code hooks in `~/.claude/settings.json`.
pub const AGENT_SETUP_INSTRUCTIONS: &str = r#"Configure Claude Code hooks for leiter by editing `~/.claude/settings.json`.

Read `~/.claude/settings.json` (or create it with `{}` if it doesn't exist).

Check whether leiter hooks are already present by looking for hook commands containing `"leiter hook context"`, `"leiter hook nudge"`, or `"leiter hook session-end"` anywhere in the existing hooks.

The desired leiter hooks are shown below. There are three cases:

1. **No leiter hooks found:** Add these hook groups to the `hooks` object. If `SessionStart` or `SessionEnd` arrays already exist, append the leiter entries to those arrays (preserving all existing hooks). If they don't exist, create them.

2. **Some leiter hooks found but the set of leiter command strings doesn't match what is shown below** (e.g., a command is missing, extra, or different — this means leiter was upgraded): Replace all existing leiter hook entries with the ones below, preserving all non-leiter hooks. Check both `SessionStart` and `SessionEnd` — if either group is missing its leiter entries, create them.

3. **Leiter hooks found and the command strings already match:** Skip and report that hooks are already configured.

SessionStart hook group:
```json
{
  "hooks": [
    {
      "type": "command",
      "command": "leiter hook context"
    },
    {
      "type": "command",
      "command": "leiter hook nudge"
    }
  ]
}
```

SessionEnd hook group:
```json
{
  "hooks": [
    {
      "type": "command",
      "command": "leiter hook session-end"
    }
  ]
}
```

Use your Edit tool to make the changes to `~/.claude/settings.json`.
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn soul_template_version_is_positive() {
        const { assert!(SOUL_TEMPLATE_VERSION > 0) };
    }

    #[test]
    fn soul_template_contains_expected_sections() {
        for heading in [
            "# Communication Style",
            "# Coding Preferences",
            "# Workflow Patterns",
            "# Tool Preferences",
            "# Technology & Environment",
            "# What Works Well",
            "# What to Avoid",
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
        let preamble = context_preamble(Path::new("/test/state"));
        for literal in [
            "/test/state/soul.md",
            "leiter distill",
            "leiter soul-upgrade",
            "leiter soul instill",
        ] {
            assert!(
                preamble.contains(literal),
                "context preamble missing: {literal}"
            );
        }
    }

    #[test]
    fn soul_template_contains_lifecycle_note() {
        assert!(
            SOUL_TEMPLATE.contains("contradict"),
            "soul template missing lifecycle note about contradiction resolution"
        );
    }

    #[test]
    fn soul_template_does_not_contain_v1_sections() {
        assert!(
            !SOUL_TEMPLATE.contains("# Project Context"),
            "soul template still contains old '# Project Context' section"
        );
        assert!(
            !SOUL_TEMPLATE.contains("# Corrections and Lessons"),
            "soul template still contains old '# Corrections and Lessons' section"
        );
    }

    #[test]
    fn soul_writing_guidelines_contains_section_names() {
        for section in [
            "Communication Style",
            "Coding Preferences",
            "Workflow Patterns",
            "Tool Preferences",
            "Technology & Environment",
            "What Works Well",
            "What to Avoid",
        ] {
            assert!(
                SOUL_WRITING_GUIDELINES.contains(section),
                "soul writing guidelines missing section: {section}"
            );
        }
    }

    #[test]
    fn soul_writing_guidelines_ends_with_newline() {
        assert!(
            SOUL_WRITING_GUIDELINES.ends_with('\n'),
            "SOUL_WRITING_GUIDELINES must end with a newline"
        );
    }

    #[test]
    fn soul_writing_guidelines_contains_contradiction_rule() {
        assert!(
            SOUL_WRITING_GUIDELINES.contains("contradict"),
            "soul writing guidelines missing contradiction resolution rule"
        );
    }

    #[test]
    fn agent_setup_instructions_contain_hook_commands() {
        assert!(AGENT_SETUP_INSTRUCTIONS.contains("leiter hook context"));
        assert!(AGENT_SETUP_INSTRUCTIONS.contains("leiter hook nudge"));
        assert!(AGENT_SETUP_INSTRUCTIONS.contains("leiter hook session-end"));
    }

    #[test]
    fn agent_setup_instructions_contain_hook_json_structure() {
        assert!(AGENT_SETUP_INSTRUCTIONS.contains(r#""type": "command""#));
        assert!(AGENT_SETUP_INSTRUCTIONS.contains(r#""command": "leiter hook context""#));
        assert!(AGENT_SETUP_INSTRUCTIONS.contains(r#""command": "leiter hook nudge""#));
        assert!(AGENT_SETUP_INSTRUCTIONS.contains(r#""command": "leiter hook session-end""#));
    }

    #[test]
    fn soul_upgrade_instructions_contain_required_elements() {
        let instructions = soul_upgrade_instructions(Path::new("/test/state"));
        assert!(instructions.contains("Migration instructions"));
        assert!(instructions.contains("soul_version"));
        assert!(instructions.contains("/test/state/soul.md"));
    }

    #[test]
    fn agent_uninstall_instructions_contain_hook_detection_strings() {
        let instructions = agent_uninstall_instructions(Path::new("/test/state"));
        assert!(instructions.contains("leiter hook context"));
        assert!(instructions.contains("leiter hook nudge"));
        assert!(instructions.contains("leiter hook session-end"));
    }

    #[test]
    fn agent_uninstall_instructions_contain_cleanup_guidance() {
        let instructions = agent_uninstall_instructions(Path::new("/test/state"));
        assert!(instructions.contains("/test/state/"));
        assert!(instructions.contains("leiter setup install"));
    }

    #[test]
    fn agent_uninstall_instructions_contain_spec_required_clauses() {
        let instructions = agent_uninstall_instructions(Path::new("/test/state"));
        assert!(instructions.contains("hook group"));
        assert!(instructions.contains("empty"));
        assert!(instructions.contains("SessionStart"));
        assert!(instructions.contains("SessionEnd"));
        assert!(instructions.contains("non-leiter hooks"));
        assert!(instructions.contains("already removed"));
    }

    #[test]
    fn nudge_message_is_not_empty() {
        assert!(!NUDGE_MESSAGE.trim().is_empty());
    }
}
