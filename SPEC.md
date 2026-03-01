# Leiter Spec

Leiter is a self-training system for Claude Code. Once enabled, Claude automatically learns user preferences, coding
practices, and workflow patterns across sessions. It works by logging session activity and periodically distilling those
logs into a persistent "soul" — a set of agent instructions that shape future agent behavior.

## Core Principle

The Claude agent does all the thinking. The `leiter` CLI is a thin helper that handles structured storage, timestamp
management, and context injection. The agent reads files, writes summaries, edits the soul, and decides what to
remember. The CLI never calls the Claude API.

## Architecture

The SessionEnd hook (rather than Stop) is used for session logging because Stop fires on every turn — not just session
end — which would block the agent on every response to write a log. SessionEnd fires once when the session actually
terminates and provides the transcript path directly, so no agent involvement is needed to save it.

The `leiter soul instill` and `leiter soul distill` commands share a single set of soul-writing guidelines (built into
the binary). This ensures consistent entry quality across both learning paths — inline preferences and transcript
distillation — while keeping normal session context minimal. The guidelines only appear when the agent is actively
writing to the soul.

```
┌──────────────────────────────────────────────────────────────┐
│                       Claude Code Session                    │
│                                                              │
│  SessionStart hook ──► leiter hook context ──► soul + agent  │
│                        leiter hook nudge        instructions  │
│                                                injected      │
│                                                              │
│  ... normal session ...                                      │
│                                                              │
│  User says "remember X" ──► /leiter-instill skill            │
│                           ──► agent edits soul.md            │
│                                                              │
│  User says "distill" ──► /leiter-distill skill               │
│                           ──► sub-agent: leiter soul distill │
│                           ──► sub-agent edits soul.md        │
│                        ──► agent: leiter soul mark-distilled │
│                                                              │
│  SessionEnd hook ──► leiter hook session-end                 │
│                      ──► copies transcript to logs/          │
└──────────────────────────────────────────────────────────────┘

~/.leiter/
├── soul.md              # The "leiter soul" — agent instructions
└── logs/
    ├── 20260223T173000Z-abc123.jsonl
    ├── 20260223T190000Z-def456.jsonl
    └── ...

~/.claude/skills/
├── leiter-setup/SKILL.md        # Each contains <!-- SCODE_LEITER_INSTALLED -->
├── leiter-distill/SKILL.md
├── leiter-instill/SKILL.md
└── leiter-teardown/SKILL.md
```

## State Directory

All state lives under a single directory. The default is `~/.leiter/`. If the `LEITER_HOME` environment variable is set,
it points directly to the state directory (so state lives under `$LEITER_HOME/`, not `$LEITER_HOME/.leiter/`). This
allows relocating leiter state for testing or multi-profile setups.

**No hardcoded paths:** All runtime output — agent-facing instructions, error messages, confirmation messages — must use
the resolved state directory path. The string `~/.leiter` must never appear in code that produces output; use the state
directory path obtained from `LEITER_HOME` (or the `$HOME/.leiter` fallback) instead. This ensures that when
`LEITER_HOME` is set, the agent and user always see the correct paths.

### Claude Code Home Directory

The Claude Code home directory is where leiter installs its plugin files (skill files). The default is `~/.claude/`. The
`leiter claude` subcommand accepts a `--claude-home <path>` flag to override the directory, primarily for testing.

### Plugin Files

`leiter claude install` writes skill files into the Claude Code home directory:

- **`<claude_home>/skills/leiter-setup/SKILL.md`** — skill that calls `leiter claude agent-setup-instructions` to
  configure hooks.
- **`<claude_home>/skills/leiter-distill/SKILL.md`** — skill for distilling session logs into the soul.
- **`<claude_home>/skills/leiter-instill/SKILL.md`** — skill for recording preferences. Description includes trigger
  keywords (remember, learn, always, never) so Claude can auto-match.
- **`<claude_home>/skills/leiter-teardown/SKILL.md`** — skill that calls `leiter claude agent-teardown-instructions` to
  remove hooks.

Each skill file contains the sentinel string `SCODE_LEITER_INSTALLED` as an HTML comment. `leiter claude uninstall`
checks for this sentinel to verify that leiter was installed before removing files.

All skill files are `const &str` templates built into the binary. They are written to disk by `leiter claude install`
and overwritten on re-run (idempotent).

### `~/.leiter/soul.md`

The soul file is a markdown document with YAML frontmatter. It contains learned preferences and instructions that are
injected into every Claude Code session.

The frontmatter contains metadata used by the CLI:

```markdown
---
last_distilled: 2026-02-23T17:00:00Z
soul_version: 2
setup_soft_epoch: 1
setup_hard_epoch: 1
---

(soul content here — see Soul Template)
```

- `last_distilled`: timestamp used by `leiter soul distill` to determine which session logs are new
- `soul_version`: integer matching the version of the soul template used to create this file, used by
  `leiter soul upgrade` to detect drift
- `setup_soft_epoch`: integer tracking the soft setup epoch. When the binary's expected soft epoch doesn't match the
  soul's value, `leiter hook context` outputs a nudge but still injects the soul. See Setup Epochs below
- `setup_hard_epoch`: integer tracking the hard setup epoch. When the binary's expected hard epoch doesn't match the
  soul's value, `leiter hook context` blocks the session (does not inject the soul). See Setup Epochs below

Both epoch fields default to 1 when absent (for backward compatibility with souls created before epochs were
introduced).

The agent edits the soul file directly using its Read/Edit/Write tools. The CLI only writes to this file during
`leiter claude install` to create the initial soul from the template; after that, all modifications are made by the
agent.

### Setup Epochs

The leiter binary may evolve in ways that require user action beyond just upgrading the binary — for example, re-running
`leiter claude install` to update hook configuration. Setup epochs detect this condition and alert the user.

There are two independent epochs, each a monotonic integer starting at 1:

- **`setup_soft_epoch`**: Bumped when a leiter upgrade introduces changes that benefit from user action but are not
  strictly required. A mismatch produces a nudge but does not block the session.
- **`setup_hard_epoch`**: Bumped when a leiter upgrade introduces changes that require user action before the session
  can function correctly. A mismatch blocks the session (the soul is not injected).

The binary has compiled-in expected values for both epochs. When `leiter hook context` runs, it compares the soul's
epoch values against the binary's expected values. The check uses exact equality — both older and newer souls are
flagged, since the binary cannot make assumptions about unknown future epochs.

Epochs are independent of `soul_version`. The soul version tracks template format changes (handled by
`leiter soul upgrade`). Epochs track integration changes (hooks, settings, etc.) that require user action outside the
soul file.

### Soul Template (built into the binary)

The `leiter` binary contains a built-in soul template (~1 page) that defines the initial structure and categories for
the soul. When `leiter claude install` creates `~/.leiter/soul.md`, it writes this template as the initial content (with
the `last_distilled` frontmatter prepended). The template nudges the agent toward capturing specific kinds of
information.

The template content is defined in source code as a well-identified constant (not inline in this spec). It should
include section headings and brief descriptions of what belongs in each section (e.g., communication style, coding
preferences, workflow patterns, tool preferences).

### `~/.leiter/logs/`

Session transcripts, one file per session. Named `<UTC_ISO8601_BASIC>-<session_id>.jsonl` (e.g.,
`20260223T173000Z-abc123.jsonl`) using UTC ISO 8601 basic format (`YYYYMMDDTHHMMSSZ`) for the timestamp and the Claude
Code session ID as a suffix. The session ID makes it easy to associate a log file with a specific session for debugging.
Each file is a session transcript (JSONL) copied from the Claude Code transcript path provided by the SessionEnd hook.

All timestamps in leiter — frontmatter values, log filenames, and CLI output — use UTC ISO 8601 format. Frontmatter uses
extended format (`2026-02-23T17:00:00Z`). Filenames use basic format (`20260223T173000Z`) to avoid colons and other
filesystem-unfriendly characters.

## Implementation

The `leiter` binary is a Rust CLI tool. Key technical choices:

- **CLI parsing:** `clap` (latest version, derive API).
- **Logging:** `tracing` crate, output to stderr. Default level is `INFO`. Verbosity flags: `-v` for `DEBUG`, `-vv` for
  `TRACE`, `-q` for `WARN`, `-qq` for `ERROR`, `--log-level=<LEVEL>` for explicit override. The `--log-level` flag takes
  precedence over `-v`/`-q` if both are provided.
- **stdout vs stderr:** All contractual output (agent instructions, soul content, session log contents, confirmation
  messages) goes to stdout. All diagnostic logging (`tracing`) goes to stderr.
- **Error handling:** `thiserror` for structured error types where callers need to match on variants. `anyhow` for
  propagation in top-level and command handler code where the specific error type doesn't matter.

## Version

The root command supports `--version` / `-V`. The displayed version is determined at build time:

- If the build runs on a commit with an exact git tag (e.g., `v0.3.0`): the tag is used as the version (with the `v`
  prefix stripped)
- Otherwise (main, feature branches, git unavailable): the version is `0.0.0-dev`

The version in `Cargo.toml` exists for cargo-dist and crates.io metadata; it is not used as the displayed version.

## CLI Commands

The `leiter` binary is assumed to be installed in `$PATH`.

### `leiter claude install`

First-time setup. Performs deterministic initialization of the state directory and writes plugin files (skills and
sentinel) to the Claude Code home directory.

**Deterministic steps:**

1. Create `~/.leiter/` directory (no-op if exists)
2. Create `~/.leiter/logs/` directory (no-op if exists)
3. If `~/.leiter/soul.md` does not exist, create it from the soul template with `last_distilled: 1970-01-01T00:00:00Z`,
   `soul_version` set to the current template version, and `setup_soft_epoch`/`setup_hard_epoch` set to the binary's
   current epoch values in the frontmatter. If `soul.md` already exists, update only the `setup_soft_epoch` and
   `setup_hard_epoch` fields to the binary's current values (preserving all other frontmatter and body content). If the
   existing frontmatter cannot be parsed, skip the epoch update silently
4. Verify the Claude Code home directory exists (error if not — Claude Code not installed)
5. Write all four skill files to their respective directories under `<claude_home>/skills/`. Overwrites existing files
   on re-run (idempotent)

**Output (stdout):** A success message listing the available skills and telling the user to run `/leiter-setup` to
configure hooks.

If any step fails, the output instructs the agent to relay the error to the user.

### `leiter claude uninstall`

Removes leiter plugin files from the Claude Code home directory. Does NOT touch `~/.leiter/` (soul and logs) or
`~/.claude/settings.json` (hooks are removed via the `agent-teardown-instructions` subcommand or manually).

**Behavior:**

1. Scan skill directories under `<claude_home>/skills/` for a `SKILL.md` containing `SCODE_LEITER_INSTALLED`
2. If no skill file contains the sentinel: error
3. Remove all four `<claude_home>/skills/leiter-*/` directories (best-effort, skip missing)

**Output (stdout):** A success message with guidance on how to remove hooks, fully clean up (`~/.leiter/`), and
re-enable later.

**Errors:** If the sentinel is missing or unreadable, exit with a non-zero code.

### `leiter claude agent-setup-instructions`

Outputs natural language instructions for the agent to configure Claude Code hooks in `~/.claude/settings.json`. This is
the same hook configuration content that `leiter claude install` used to output directly. It is called by the
`/leiter-setup` skill.

**Output (stdout):** Instructions including the exact JSON hook entries for `SessionStart` and `SessionEnd`, plus
three-case logic for handling fresh install, upgrade, and already-configured states. See Hook Configuration below for
the exact hook JSON. After hooks are configured, includes an optional permissions prompt (see Permissions below).

### `leiter claude agent-teardown-instructions`

Outputs natural language instructions for the agent to remove leiter hooks from `~/.claude/settings.json`. Called by the
`/leiter-teardown` skill.

**Output (stdout):** Instructions telling the agent to find and remove hook entries whose commands contain
`"leiter hook context"`, `"leiter hook nudge"`, or `"leiter hook session-end"`, clean up empty arrays, preserve
non-leiter hooks, remove leiter permission entries (see Permissions below), and provide cleanup/re-enable guidance to
the user.

### `leiter hook context`

Outputs the soul content and agent instructions. Called by the SessionStart hook.

**Behavior:**

1. If the soul file does not exist, output a message suggesting `leiter claude install` and return
2. Read the soul file and attempt to parse its frontmatter
3. If frontmatter is parseable, check setup epochs (see Setup Epochs):
   - If `setup_hard_epoch` does not exactly match the binary's expected value: output an error message and return
     without injecting the soul. The message differs based on direction — if the soul's epoch is lower, suggest running
     `leiter claude install`; if higher, suggest upgrading the binary
   - If `setup_soft_epoch` does not exactly match the binary's expected value: output a nudge message (different for
     older vs. newer soul) but continue to inject the soul normally
4. If frontmatter parsing fails, skip epoch checks and proceed (fail-open — a corrupt frontmatter should not block the
   session entirely)

**Output (stdout):**

1. A preamble explaining what leiter is and how the agent should interact with it. The preamble text is defined in
   source code. It must cover these topics with the specified constraints:

   **Identity:** A one-line description of leiter (self-training system that learns across sessions).

   **Soul file location:** Must include the resolved path to the soul file (the state directory joined with `soul.md`).
   Must tell the agent to use its Read/Edit/Write tools to modify this file directly.

   **When to instill preferences:** When the user says "remember", "learn", "instill", "always", "never", or similar
   preference-setting language. The agent should invoke the `/leiter-instill` skill.

   **Session transcripts:** Session transcripts are saved automatically by the SessionEnd hook. The agent does not need
   to do anything — no manual logging is required.

   **Distillation:** When the user asks to distill session logs, the agent should invoke the `/leiter-distill` skill.

   **Soul upgrade command:** Must include the literal command `leiter soul upgrade`. Explain that this is user-triggered
   (the user says "upgrade the leiter soul" or similar) and outputs migration instructions if the soul template is
   outdated.

2. The full contents of `~/.leiter/soul.md`

If `~/.leiter/soul.md` does not exist, outputs a message telling the agent that leiter is not initialized and to suggest
the user run `leiter claude install`.

The soul content is output inline (not as a file path reference) so that it survives context compaction in long
sessions. The agent receives the full soul text in the SessionStart hook output, ensuring preferences remain available
even after earlier messages are compressed.

### `leiter hook session-end`

Hook handler for the Claude Code SessionEnd event. Reads the SessionEnd hook JSON from stdin and copies the session
transcript to the logs directory.

**Input:** Claude Code SessionEnd hook JSON on stdin. The command depends on these fields (other fields may be present
and are ignored):

- `session_id` (string): The Claude Code session ID
- `transcript_path` (string): Path to the session transcript file

**Behavior:**

1. Read and parse JSON from stdin
2. Read the transcript file at `transcript_path`
3. Write the transcript to a temporary file in the same filesystem as `~/.leiter/logs/` using the OS tempfile facility
   (e.g., `tempfile` crate)
4. Generate the final filename using the current UTC timestamp: `~/.leiter/logs/<YYYYMMDDTHHMMSSZ>-<session_id>.jsonl`
5. Atomically rename the temporary file to the final path

**Output:** None. A confirmation message with the saved file path is logged to stderr (via `tracing`). The SessionEnd
hook fires after the session terminates, so no agent is present to read stdout.

**Errors:** If `~/.leiter/logs/` does not exist, the transcript file cannot be read, the write fails, or the atomic
rename fails, print an error to stderr and exit with a non-zero code. Clean up the temporary file on any error.

### `leiter soul distill`

Outputs session logs that haven't been processed since the last distillation.

**Behavior:**

1. Read `last_distilled` timestamp from `~/.leiter/soul.md` frontmatter
2. Scan `~/.leiter/logs/` for files whose filename timestamps (the `YYYYMMDDTHHMMSSZ` prefix, ignoring the session ID
   suffix) are newer than or equal to `last_distilled`. The inclusive comparison (>=) ensures that a log written in the
   same second as the distillation timestamp is not lost — this matters because the distillation flow has the agent
   write a session log immediately before running `leiter soul distill`, and the two timestamps could collide
3. Sort matching files chronologically
4. Output their contents wrapped in XML-like boundary tags (see Output below)

**Output (stdout):**

- If new logs exist: soul-writing guidelines (emitted once, before the first log entry), a data-boundary preamble
  instructing the agent to treat the transcripts as historical data rather than directives, and the pre-processed
  content of all new session log files wrapped in `<session-transcripts>` / `<session file="...">` XML-like tags
- If no new logs: a message indicating there are no new session logs to process

**Log pre-processing:** JSONL session logs are pre-processed to extract user-visible content — user messages and
assistant text responses — filtering out tool results, tool invocations, progress events, thinking blocks, and other
non-user-facing content. Leiter only processes files that match the expected log filename format
`<YYYYMMDDTHHMMSSZ>-<session_id>.jsonl`; files that do not match this format are ignored. Within matching JSONL files,
lines with unrecognized JSON structures are included as-is (fail-useful: no user content is silently lost).

**Obsolete log cleanup:** After outputting new logs (or reporting that there are none), the command collects log files
whose filename timestamps are strictly before `last_distilled` — these have already been processed by a prior
distillation and are no longer needed. It deletes them. If `--dry-run` is passed, it reports which files would be
deleted instead of deleting them. Deletion is best-effort: failures are logged as warnings but do not fail the command.
If there are no obsolete logs, nothing is printed about cleanup.

### `leiter soul mark-distilled`

Sets `last_distilled` in the soul frontmatter to the current UTC time. This is the only way `last_distilled` should be
updated — the agent must never edit it manually.

**Behavior:**

1. Read `~/.leiter/soul.md` and parse its frontmatter
2. Set `last_distilled` to the current UTC time
3. Write the soul back, preserving the body and all other frontmatter fields

**Output (stdout):** A confirmation message including the exact timestamp that was set.

**Errors:** If the soul file does not exist or its frontmatter cannot be parsed, exit with a non-zero code and an error
message on stderr.

### `leiter soul instill <text>`

Outputs agent instructions for adding a preference to the soul file. Called by the agent when the user expresses a
preference ("remember", "learn", "instill", "always", "never", or similar language).

**Input:** A positional argument containing the preference or fact the user wants remembered.

**Output (stdout):**

1. The user's preference, quoted for clarity
2. Soul-writing guidelines (shared with `leiter soul distill`) covering entry format, specificity, placement,
   contradiction resolution, and examples
3. Instruction to read `~/.leiter/soul.md` and edit the appropriate section

See the Architecture section for why guidelines are shared between `instill` and `distill`.

### `leiter hook nudge`

Checks for stale undistilled session logs and outputs a nudge if any exist. Called by the SessionStart hook (after
`leiter hook context`) to remind the agent to suggest distillation.

**Behavior:**

1. Read `last_distilled` timestamp from `~/.leiter/soul.md` frontmatter
2. Scan `~/.leiter/logs/` for files whose filename timestamps are >= `last_distilled` (same inclusive comparison as
   `leiter soul distill`)
3. If any such file has a timestamp older than 24 hours ago (`now - 24h`): output a nudge message (defined in source
   code)
4. Otherwise: output nothing

If the soul file does not exist or the logs directory does not exist, silently output nothing and exit successfully. If
the soul file cannot be read, its frontmatter cannot be parsed, or the logs directory cannot be read, also output
nothing and exit successfully (fail-open). The nudge must not break the session.

**Output (stdout):**

- If stale undistilled logs exist: a short nudge message reminding the agent to suggest distillation
- Otherwise: nothing (zero context pollution)

### `leiter soul upgrade`

Detects soul template drift and outputs agent instructions to migrate the existing soul to the current template format.
Invoked by the agent when the user asks to "upgrade the leiter soul".

**Behavior:**

1. Read `soul_version` from `~/.leiter/soul.md` frontmatter
2. Compare against the current template version built into the binary
3. If already up to date: output a message saying so
4. If outdated: output upgrade instructions for the agent (see below)

**Output when outdated:**

1. A changelog of what changed in each version between the user's current version and the latest, one brief summary per
   version (like a soul template changelog)
2. The full current template with its version number
3. Instructions for the agent to restructure the existing soul content into the new format while preserving all learned
   preferences, and to update `soul_version` in the frontmatter

The changelog entries are maintained in the source code as human- and agent-readable text. There is no required
structure — each entry is a brief prose description of what changed in that soul template version. New entries are added
when the soul template is modified in future code changes. The agent performs the actual soul file edits.

## Hook Configuration

The following hooks are configured in `~/.claude/settings.json` by the agent when the user runs `/leiter-setup` (which
calls `leiter claude agent-setup-instructions`):

### SessionStart Hook

```json
{
  "hooks": {
    "SessionStart": [
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
    ]
  }
}
```

Fires on every session start (new, resume, clear, compact). The stdout output is added as context for the agent. The
`leiter hook context` hook injects the soul and agent instructions; the `leiter hook nudge` hook outputs a distillation
reminder only when stale undistilled logs exist (otherwise it outputs nothing, adding zero context).

### SessionEnd Hook

```json
{
  "hooks": {
    "SessionEnd": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "leiter hook session-end"
          }
        ]
      }
    ]
  }
}
```

Fires once when the session terminates. The `leiter hook session-end` command reads the SessionEnd hook JSON from stdin
(which includes `session_id` and `transcript_path`) and copies the transcript to `~/.leiter/logs/`.

## Permissions

After configuring hooks, `agent-setup-instructions` offers two optional permission rules (each prompted separately):

1. **Bash commands:** `"Bash(leiter:*)"` — allows all leiter CLI commands without confirmation dialogs.
2. **Soul file access:** `"Read(<soul_path>)"`, `"Edit(<soul_path>)"`, and `"Write(<soul_path>)"` — allows reading,
   editing, and writing the soul file without confirmation dialogs. The soul path is resolved from the state directory
   (e.g., `~/.leiter/soul.md`).

Each rule is offered as a yes/no question. The user can accept one, both, or neither.

`agent-teardown-instructions` removes any entries in `permissions.allow` starting with `Bash(leiter` or referencing the
soul file path. Empty `permissions.allow` arrays and empty `permissions` objects are cleaned up.

## Flows

### First-Time Setup

1. User installs `leiter` binary
2. User runs `leiter claude install` from their terminal
3. The command creates `~/.leiter/` structure and writes skill files to `~/.claude/skills/`
4. User starts a Claude Code session and runs `/leiter-setup`
5. The skill calls `leiter claude agent-setup-instructions`, agent configures hooks in `~/.claude/settings.json`
6. User reviews and approves the settings change
7. Agent asks whether to add `Bash(leiter:*)` permission rule; user accepts or declines
8. On next session start, leiter is active

### Normal Session (After Setup)

1. Session starts → SessionStart hook fires → `leiter hook context` outputs soul + instructions, `leiter hook nudge`
   outputs a distillation reminder if stale logs exist → agent has leiter hook context
2. Normal session proceeds
3. Session ends → SessionEnd hook fires → `leiter hook session-end` copies transcript to `~/.leiter/logs/`

### User Asks the Agent to Learn Something

1. User says "learn to always use snake_case for Rust functions"
2. Agent invokes the `/leiter-instill` skill (auto-matched via trigger keywords)
3. Skill runs `leiter soul instill "always use snake_case for Rust functions"`
4. Agent receives writing guidelines and the quoted preference
5. Agent reads `~/.leiter/soul.md`, edits the appropriate section following the guidelines
6. Preference is active in all future sessions

### Soul Upgrade

1. User updates `leiter` binary to a newer version
2. User says "upgrade the leiter soul"
3. Agent runs `leiter soul upgrade`
4. If already current: agent relays that no upgrade is needed
5. If outdated: agent receives the upgrade instructions and new template
6. Agent reads current `~/.leiter/soul.md`, restructures it into the new format, and updates `soul_version` in the
   frontmatter

### Distillation

1. User says "distill my session logs" (or similar natural language)
2. Agent invokes the `/leiter-distill` skill
3. Skill spawns a sub-agent to handle distillation (keeps session log output out of the main context)
4. Sub-agent runs `leiter soul distill`, reads the output, and updates the soul with new learnings
5. After the sub-agent completes successfully, the main agent runs `leiter soul mark-distilled`

## Non-Goals (For Now)

- Multiple user profiles or project-specific souls
- Automatic distillation (always user-triggered)
- Soul backup
- API key management or direct Claude API calls from the CLI
