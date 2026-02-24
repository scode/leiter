# Leiter Spec

Leiter is a self-training system for Claude Code. Once enabled, Claude
automatically learns user preferences, coding practices, and workflow patterns
across sessions. It works by logging session activity and periodically
distilling those logs into a persistent "soul" — a set of agent instructions
that shape future agent behavior.

## Core Principle

The Claude agent does all the thinking. The `leiter` CLI is a thin helper that
handles structured storage, timestamp management, and context injection. The
agent reads files, writes summaries, edits the soul, and decides what to
remember. The CLI never calls the Claude API.

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│                    Claude Code Session                   │
│                                                         │
│  SessionStart hook ──► leiter context ──► soul + agent  │
│                                           instructions  │
│                                           injected      │
│                                                         │
│  ... normal session ...                                 │
│                                                         │
│  User says "remember X" ──► agent edits soul.md         │
│                                                         │
│  User says "distill" ──► agent calls leiter distill     │
│                           ──► reads new logs            │
│                           ──► agent edits soul.md       │
│                           ──► agent updates frontmatter │
│                                                         │
│  Stop hook ──► leiter stop-hook                         │
│                ──► blocks stop (if not already logging) │
│                ──► agent writes summary                 │
│                ──► pipes to leiter log                  │
│                ──► agent stops                          │
└─────────────────────────────────────────────────────────┘

~/.leiter/
├── soul.md              # The "leiter soul" — agent instructions
└── logs/
    ├── 20260223T173000Z-abc123.md
    ├── 20260223T190000Z-def456.md
    └── ...
```

## State Directory

All state lives under `~/.leiter/`. This is hardcoded for now regardless of
platform.

### `~/.leiter/soul.md`

The soul file is a markdown document with YAML frontmatter. It contains learned
preferences and instructions that are injected into every Claude Code session.

The frontmatter contains metadata used by the CLI:

```markdown
---
last_distilled: 2026-02-23T17:00:00Z
soul_version: 1
---

(soul content here — see Soul Template)
```

- `last_distilled`: timestamp used by `leiter distill` to determine which
  session logs are new
- `soul_version`: integer matching the version of the soul template used to
  create this file, used by `leiter soul-upgrade` to detect drift

The agent edits the soul file directly using its Read/Edit/Write tools. The CLI
only writes to this file during `leiter agent-setup` to create the initial soul
from the template; after that, all modifications are made by the agent.

### Soul Template (built into the binary)

The `leiter` binary contains a built-in soul template (~1 page) that defines
the initial structure and categories for the soul. When `leiter agent-setup`
creates `~/.leiter/soul.md`, it writes this template as the initial content
(with the `last_distilled` frontmatter prepended). The template nudges the
agent toward capturing specific kinds of information.

The template content is defined in source code as a well-identified constant
(not inline in this spec). It should include section headings and brief
descriptions of what belongs in each section (e.g., communication style,
coding preferences, workflow patterns, tool preferences).

### `~/.leiter/logs/`

Session logs, one file per session. Named
`<UTC_ISO8601_BASIC>-<session_id>.md` (e.g., `20260223T173000Z-abc123.md`)
using UTC ISO 8601 basic format (`YYYYMMDDTHHMMSSZ`) for the timestamp and the
Claude Code session ID as a suffix. The session ID makes it easy to associate a
log file with a specific session for debugging. Each file contains free-form
markdown written by the agent — typically a brief summary of what was done,
learnings, challenges, and things the user corrected.

All timestamps in leiter — frontmatter values, log filenames, and CLI output —
use UTC ISO 8601 format. Frontmatter uses extended format
(`2026-02-23T17:00:00Z`). Filenames use basic format (`20260223T173000Z`) to
avoid colons and other filesystem-unfriendly characters.

## Implementation

The `leiter` binary is a Rust CLI tool. Key technical choices:

- **CLI parsing:** `clap` (latest version, derive API).
- **Logging:** `tracing` crate, output to stderr. Default level is `INFO`.
  Verbosity flags: `-v` for `DEBUG`, `-vv` for `TRACE`, `-q` for `WARN`,
  `-qq` for `ERROR`, `--log-level=<LEVEL>` for explicit override. The
  `--log-level` flag takes precedence over `-v`/`-q` if both are provided.
- **stdout vs stderr:** All contractual output (agent instructions, soul
  content, session log contents, confirmation messages) goes to stdout. All
  diagnostic logging (`tracing`) goes to stderr.
- **Error handling:** `thiserror` for structured error types where callers need
  to match on variants. `anyhow` for propagation in top-level and command
  handler code where the specific error type doesn't matter.

## CLI Commands

The `leiter` binary is assumed to be installed in `$PATH`.

### `leiter agent-setup`

First-time setup. Performs deterministic initialization and then outputs
natural language instructions for the agent to configure Claude Code hooks.

**Deterministic steps:**
1. Create `~/.leiter/` directory (no-op if exists)
2. Create `~/.leiter/logs/` directory (no-op if exists)
3. Create `~/.leiter/soul.md` from the soul template, with
   `last_distilled: 1970-01-01T00:00:00Z` and `soul_version` set to the
   current template version in the frontmatter (skip if `soul.md` already
   exists)

**Output (stdout):**
Natural language instructions telling the agent to configure Claude Code hooks
in `~/.claude/settings.json`. The output includes:

1. The exact JSON hook entries to add (the `SessionStart` and `Stop` hook
   objects shown in the Hook Configuration section below)
2. Instructions for the agent to:
   - Read `~/.claude/settings.json` (or create it with `{}` if it doesn't
     exist)
   - Check whether leiter hooks are already present (by looking for commands
     containing `"leiter context"` and `"leiter stop-hook"`)
   - If not present, append the leiter hook groups to the existing
     `SessionStart` and `Stop` arrays (creating those arrays if they don't
     exist), preserving all existing hooks
   - If already present, skip and report that hooks are already configured
   - Use the agent's Edit tool to make the changes

If any deterministic step fails, the output instructs the agent to relay the
error to the user.

This command is designed to be run inside a Claude Code session where the agent
acts on the output. If run outside a session, the user sees the instructions
and can paste them into a session or apply them manually.

### `leiter context`

Outputs the soul content and agent instructions. Called by the SessionStart
hook.

**Output (stdout):**
1. A preamble explaining what leiter is and how the agent should interact with
   it. The preamble text is defined in source code. It must cover these topics
   with the specified constraints:

   **Identity:** A one-line description of leiter (self-training system that
   learns across sessions).

   **Soul file location:** Must include the literal path `~/.leiter/soul.md`.
   Must tell the agent to use its Read/Edit/Write tools to modify this file
   directly.

   **When to edit the soul directly:** When the user says "remember", "learn",
   "always", "never", or similar preference-setting language. The agent should
   read the soul, find the appropriate section, and add the preference. No CLI
   command is needed for this — the agent edits the file directly.

   **Session logging:** The agent will be prompted to write a session log when
   the session ends (via the stop hook). It does not need to do anything
   proactively — the prompt will include instructions and the session ID.

   **Distillation command:** Must include the literal command
   `leiter distill`. Explain that this is user-triggered (the user says
   "distill" or similar), outputs unprocessed session logs, and the agent
   should then update the soul with new learnings and update `last_distilled`
   in the frontmatter to the current UTC ISO 8601 timestamp.

   **Soul upgrade command:** Must include the literal command
   `leiter soul-upgrade`. Explain that this is user-triggered (the user says
   "upgrade the leiter soul" or similar) and outputs migration instructions
   if the soul template is outdated.

2. The full contents of `~/.leiter/soul.md`

If `~/.leiter/soul.md` does not exist, outputs a message telling the agent
that leiter is not initialized and to suggest the user run `leiter agent-setup`.

### `leiter log --session-id <id>`

Stores a session log. Reads the log content from stdin.

**Input:** Free-form markdown on stdin.

**Arguments:**
- `--session-id <id>` (required): The Claude Code session ID, used in the
  filename for debuggability.

**Behavior:**
1. Read all of stdin to completion (wait for stdin to close before proceeding)
2. Write the content to a temporary file in the same filesystem as
   `~/.leiter/logs/` using the OS tempfile facility (e.g., `tempfile` crate)
3. Generate the final filename using the current UTC timestamp (captured after
   stdin is fully read):
   `~/.leiter/logs/<YYYYMMDDTHHMMSSZ>-<session_id>.md`
4. Atomically rename the temporary file to the final path

The timestamp is captured after stdin closes, not before, so the filename
reflects when the log was actually received rather than when the command
started.

**Output (stdout):** Confirmation message with the path of the created file.

**Errors:** If `~/.leiter/logs/` does not exist, the write fails, or the
atomic rename fails, print an error to stderr and exit with a non-zero code.
Clean up the temporary file on any error.

### `leiter distill`

Outputs session logs that haven't been processed since the last distillation.

**Behavior:**
1. Read `last_distilled` timestamp from `~/.leiter/soul.md` frontmatter
2. Scan `~/.leiter/logs/` for files whose filename timestamps (the
   `YYYYMMDDTHHMMSSZ` prefix, ignoring the session ID suffix) are newer than
   or equal to `last_distilled`. The inclusive comparison (>=) ensures that a
   log written in the same second as the distillation timestamp is not lost —
   this matters because the distillation flow has the agent write a session log
   immediately before running `leiter distill`, and the two timestamps could
   collide
3. Sort matching files chronologically
4. Output their contents, each preceded by a header with the filename

**Output (stdout):**
- If new logs exist: the concatenated content of all new session log files,
  with filename headers separating each entry
- If no new logs: a message indicating there are no new session logs to process

After the agent processes the distill output and updates the soul, the agent is
responsible for updating the `last_distilled` timestamp in the soul file's
frontmatter to the current time.

### `leiter stop-hook`

Hook handler for the Claude Code Stop event. Reads the Stop hook JSON from
stdin and decides whether to block the stop.

**Input:** Claude Code Stop hook JSON on stdin. The command depends on these
fields (other fields may be present and are ignored):

- `session_id` (string): The Claude Code session ID
- `stop_hook_active` (boolean): `false` on the first stop of a turn not
  initiated by a stop hook; `true` when the current turn was initiated by a
  stop hook blocking a previous stop

**Behavior:**
- If `stop_hook_active` is `false`: output a blocking decision telling the
  agent to write a session log before stopping, including the `session_id`
  from the input JSON so the agent can pass it to `leiter log --session-id`
- If `stop_hook_active` is `true`: output an allow decision (exit 0 with no
  stdout, or a JSON object with `"decision": "allow"`)

Note: `stop_hook_active` is `false` on resumed sessions too (the resume is
user-initiated, not stop-hook-initiated). This means a resumed session will
also be prompted to write a session log. Duplicate logging from resume is
acceptable — better to capture extra signal than to lose it.

**Output when blocking:**

A JSON object with `"decision": "block"` and a `"reason"` containing the
session logging prompt. The exact prompt wording is maintained in the source
code alongside other agent-facing text (like the soul template and context
preamble). Example for illustration:

```json
{
  "decision": "block",
  "reason": "Before stopping, please write a brief session log summarizing what was done in this session, any learnings for future sessions, and any challenges encountered. Pipe the log content to `leiter log --session-id abc123`. If you have already written a session log in this session, you may skip this step."
}
```

### `leiter soul-upgrade`

Detects soul template drift and outputs agent instructions to migrate the
existing soul to the current template format. Invoked by the agent when the
user asks to "upgrade the leiter soul".

**Behavior:**
1. Read `soul_version` from `~/.leiter/soul.md` frontmatter
2. Compare against the current template version built into the binary
3. If already up to date: output a message saying so
4. If outdated: output upgrade instructions for the agent (see below)

**Output when outdated:**
1. A changelog of what changed in each version between the user's current
   version and the latest, one brief summary per version (like a soul template
   changelog)
2. The full current template with its version number
3. Instructions for the agent to restructure the existing soul content into the
   new format while preserving all learned preferences, and to update
   `soul_version` in the frontmatter

The changelog entries are maintained in the source code as human- and
agent-readable text. There is no required structure — each entry is a brief
prose description of what changed in that soul template version. New entries
are added when the soul template is modified in future code changes. The agent
performs the actual soul file edits.

## Hook Configuration

The following hooks are configured in `~/.claude/settings.json` by the agent
during `leiter agent-setup`:

### SessionStart Hook

```json
{
  "hooks": {
    "SessionStart": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "leiter context"
          }
        ]
      }
    ]
  }
}
```

Fires on every session start (new, resume, clear, compact). The stdout output
is added as context for the agent.

### Stop Hook

```json
{
  "hooks": {
    "Stop": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "leiter stop-hook"
          }
        ]
      }
    ]
  }
}
```

Fires when the agent finishes responding. The `leiter stop-hook` command reads
the hook JSON from stdin. When `stop_hook_active` is false (the agent was not
continued by a stop hook), it blocks the stop with instructions to write a
session log. When `stop_hook_active` is true (the agent was continued by a stop
hook and has had the chance to write a log), it allows the stop.

## Flows

### First-Time Setup

1. User installs `leiter` binary
2. In a Claude Code session, user says "set up leiter"
3. Agent runs `leiter agent-setup`
4. The command creates `~/.leiter/` structure and outputs instructions
5. Agent reads the instructions and edits `~/.claude/settings.json` to add hooks
6. User reviews and approves the settings change
7. On next session start, leiter is active

### Normal Session (After Setup)

1. Session starts → SessionStart hook fires → `leiter context` outputs soul +
   instructions → agent has leiter context
2. Normal session proceeds
3. Agent finishes → Stop hook fires → `leiter stop-hook` blocks with "write a
   session log"
4. Agent writes a markdown summary and pipes it to `leiter log`
5. Agent finishes again → Stop hook fires → `stop_hook_active` is true → stop
   allowed

### User Asks the Agent to Learn Something

1. User says "learn to always use snake_case for Rust functions"
2. Agent reads `~/.leiter/soul.md`
3. Agent edits the appropriate section to add the preference
4. Preference is active in all future sessions

### Soul Upgrade

1. User updates `leiter` binary to a newer version
2. User says "upgrade the leiter soul"
3. Agent runs `leiter soul-upgrade`
4. If already current: agent relays that no upgrade is needed
5. If outdated: agent receives the upgrade instructions and new template
6. Agent reads current `~/.leiter/soul.md`, restructures it into the new
   format, and updates `soul_version` in the frontmatter

### Distillation

1. User says "distill my session logs" (or similar natural language)
2. Agent writes a session log for the current session first (so it's included)
3. Agent runs `leiter distill`
4. Agent receives all session logs since last distillation
5. Agent reads current `~/.leiter/soul.md`
6. Agent edits the soul to incorporate new learnings
7. Agent updates `last_distilled` in the frontmatter to the current timestamp

## Non-Goals (For Now)

- Cross-platform path handling (hardcoded to `~/.leiter/`)
- Multiple user profiles or project-specific souls
- Automatic distillation (always user-triggered)
- Soul backup
- Session log rotation or cleanup
- API key management or direct Claude API calls from the CLI
