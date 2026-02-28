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

The `leiter instill` and `leiter distill` commands share a single set of soul-writing guidelines (built into the
binary). This ensures consistent entry quality across both learning paths — inline preferences and transcript
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
│  User says "remember X" ──► agent calls leiter instill       │
│                           ──► agent edits soul.md            │
│                                                              │
│  User says "distill" ──► agent calls leiter distill          │
│                           ──► reads new logs                 │
│                           ──► agent edits soul.md            │
│                           ──► agent updates frontmatter      │
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
```

## State Directory

All state lives under a single directory. The default is `~/.leiter/`. If the `LEITER_HOME` environment variable is set,
it points directly to the state directory (so state lives under `$LEITER_HOME/`, not `$LEITER_HOME/.leiter/`). This
allows relocating leiter state for testing or multi-profile setups.

**No hardcoded paths:** All runtime output — agent-facing instructions, error messages, confirmation messages — must use
the resolved state directory path. The string `~/.leiter` must never appear in code that produces output; use the state
directory path obtained from `LEITER_HOME` (or the `$HOME/.leiter` fallback) instead. This ensures that when
`LEITER_HOME` is set, the agent and user always see the correct paths.

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

- `last_distilled`: timestamp used by `leiter distill` to determine which session logs are new
- `soul_version`: integer matching the version of the soul template used to create this file, used by
  `leiter soul-upgrade` to detect drift
- `setup_soft_epoch`: integer tracking the soft setup epoch. When the binary's expected soft epoch doesn't match the
  soul's value, `leiter hook context` outputs a nudge but still injects the soul. See Setup Epochs below
- `setup_hard_epoch`: integer tracking the hard setup epoch. When the binary's expected hard epoch doesn't match the
  soul's value, `leiter hook context` blocks the session (does not inject the soul). See Setup Epochs below

Both epoch fields default to 1 when absent (for backward compatibility with souls created before epochs were
introduced).

The agent edits the soul file directly using its Read/Edit/Write tools. The CLI only writes to this file during
`leiter setup install` to create the initial soul from the template; after that, all modifications are made by the
agent.

### Setup Epochs

The leiter binary may evolve in ways that require user action beyond just upgrading the binary — for example, re-running
`leiter setup install` to update hook configuration. Setup epochs detect this condition and alert the user.

There are two independent epochs, each a monotonic integer starting at 1:

- **`setup_soft_epoch`**: Bumped when a leiter upgrade introduces changes that benefit from user action but are not
  strictly required. A mismatch produces a nudge but does not block the session.
- **`setup_hard_epoch`**: Bumped when a leiter upgrade introduces changes that require user action before the session
  can function correctly. A mismatch blocks the session (the soul is not injected).

The binary has compiled-in expected values for both epochs. When `leiter hook context` runs, it compares the soul's
epoch values against the binary's expected values. The check uses exact equality — both older and newer souls are
flagged, since the binary cannot make assumptions about unknown future epochs.

Epochs are independent of `soul_version`. The soul version tracks template format changes (handled by
`leiter soul-upgrade`). Epochs track integration changes (hooks, settings, etc.) that require user action outside the
soul file.

### Soul Template (built into the binary)

The `leiter` binary contains a built-in soul template (~1 page) that defines the initial structure and categories for
the soul. When `leiter setup install` creates `~/.leiter/soul.md`, it writes this template as the initial content (with
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

## CLI Commands

The `leiter` binary is assumed to be installed in `$PATH`.

### `leiter setup install`

First-time setup. Performs deterministic initialization and then outputs natural language instructions for the agent to
configure Claude Code hooks.

**Deterministic steps:**

1. Create `~/.leiter/` directory (no-op if exists)
2. Create `~/.leiter/logs/` directory (no-op if exists)
3. If `~/.leiter/soul.md` does not exist, create it from the soul template with `last_distilled: 1970-01-01T00:00:00Z`,
   `soul_version` set to the current template version, and `setup_soft_epoch`/`setup_hard_epoch` set to the binary's
   current epoch values in the frontmatter. If `soul.md` already exists, update only the `setup_soft_epoch` and
   `setup_hard_epoch` fields to the binary's current values (preserving all other frontmatter and body content). If the
   existing frontmatter cannot be parsed, skip the epoch update silently

**Output (stdout):** Natural language instructions telling the agent to configure Claude Code hooks in
`~/.claude/settings.json`. The output includes:

1. The exact JSON hook entries to add (the `SessionStart` and `SessionEnd` hook objects shown in the Hook Configuration
   section below)
2. Instructions for the agent to:
   - Read `~/.claude/settings.json` (or create it with `{}` if it doesn't exist)
   - Check whether leiter hooks are already present by looking for hook commands containing `"leiter hook context"`,
     `"leiter hook nudge"`, or `"leiter hook session-end"`
   - If no leiter hooks are found, append the leiter hook groups to the existing `SessionStart` and `SessionEnd` arrays
     (creating those arrays if they don't exist), preserving all existing hooks
   - If leiter hooks are found but don't match the expected set (e.g., after a leiter upgrade changed the hook
     commands), replace the leiter hook entries with the current ones, preserving all non-leiter hooks. Create any
     missing hook groups (e.g., if `SessionEnd` has no leiter entry but `SessionStart` does). The match is based on the
     set of leiter command strings present, not on JSON formatting
   - If leiter hooks are found and already match the expected commands exactly, skip and report that hooks are already
     configured
   - Use the agent's Edit tool to make the changes

If any deterministic step fails, the output instructs the agent to relay the error to the user.

This command is designed to be run inside a Claude Code session where the agent acts on the output. If run outside a
session, the user sees the instructions and can paste them into a session or apply them manually.

### `leiter agent-uninstall`

Outputs natural language instructions for the agent to remove leiter hooks from `~/.claude/settings.json`. Makes no
filesystem changes — the command only emits instructions.

**Output (stdout):** Instructions telling the agent to:

1. Read `~/.claude/settings.json`
2. Find and remove hook entries whose commands contain `"leiter hook context"`, `"leiter hook nudge"`, or
   `"leiter hook session-end"` (the same detection strings used by `leiter setup install`)
3. If a hook group becomes empty after removal, remove the entire group object from its parent array
4. If a `SessionStart` or `SessionEnd` array becomes empty, remove it from the `hooks` object
5. Preserve all non-leiter hooks
6. Use the Edit tool to make changes
7. If no leiter hooks are found, report that hooks are already removed
8. After hook removal, tell the user how to fully clean up (`~/.leiter/` and the binary) and how to re-enable leiter

### `leiter hook context`

Outputs the soul content and agent instructions. Called by the SessionStart hook.

**Behavior:**

1. If the soul file does not exist, output a message suggesting `leiter setup install` and return
2. Read the soul file and attempt to parse its frontmatter
3. If frontmatter is parseable, check setup epochs (see Setup Epochs):
   - If `setup_hard_epoch` does not exactly match the binary's expected value: output an error message and return
     without injecting the soul. The message differs based on direction — if the soul's epoch is lower, suggest running
     `leiter setup install`; if higher, suggest upgrading the binary
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
   preference-setting language. The agent should run `leiter instill "<what the user wants remembered>"` and follow the
   instructions it outputs.

   **Session transcripts:** Session transcripts are saved automatically by the SessionEnd hook. The agent does not need
   to do anything — no manual logging is required.

   **Distillation command:** Must include the literal command `leiter distill`. Explain that this is user-triggered (the
   user says "distill" or similar), outputs new session logs, and the agent should then update the soul with new
   learnings and update `last_distilled` in the frontmatter to the current UTC ISO 8601 timestamp.

   **Soul upgrade command:** Must include the literal command `leiter soul-upgrade`. Explain that this is user-triggered
   (the user says "upgrade the leiter soul" or similar) and outputs migration instructions if the soul template is
   outdated.

2. The full contents of `~/.leiter/soul.md`

If `~/.leiter/soul.md` does not exist, outputs a message telling the agent that leiter is not initialized and to suggest
the user run `leiter setup install`.

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

**Output (stdout):** Confirmation message with the path of the created file.

**Errors:** If `~/.leiter/logs/` does not exist, the transcript file cannot be read, the write fails, or the atomic
rename fails, print an error to stderr and exit with a non-zero code. Clean up the temporary file on any error.

### `leiter distill`

Outputs session logs that haven't been processed since the last distillation.

**Behavior:**

1. Read `last_distilled` timestamp from `~/.leiter/soul.md` frontmatter
2. Scan `~/.leiter/logs/` for files whose filename timestamps (the `YYYYMMDDTHHMMSSZ` prefix, ignoring the session ID
   suffix) are newer than or equal to `last_distilled`. The inclusive comparison (>=) ensures that a log written in the
   same second as the distillation timestamp is not lost — this matters because the distillation flow has the agent
   write a session log immediately before running `leiter distill`, and the two timestamps could collide
3. Sort matching files chronologically
4. Output their contents, each preceded by a prominent separator: `=== BEGIN SESSION <filename> ===`

**Output (stdout):**

- If new logs exist: soul-writing guidelines (emitted once, before the first log entry) followed by the pre-processed
  content of all new session log files, each preceded by a `=== BEGIN SESSION <filename> ===` separator
- If no new logs: a message indicating there are no new session logs to process

**Log pre-processing:** JSONL session logs are pre-processed to extract user-visible content — user messages and
assistant text responses — filtering out tool results, tool invocations, progress events, thinking blocks, and other
non-user-facing content. For non-JSONL files or lines with unrecognized JSON structures, the raw content is included
as-is (fail-useful: no user content is silently lost).

**Obsolete log cleanup:** After outputting new logs (or reporting that there are none), the command collects log files
whose filename timestamps are strictly before `last_distilled` — these have already been processed by a prior
distillation and are no longer needed. It deletes them. If `--dry-run` is passed, it reports which files would be
deleted instead of deleting them. Deletion is best-effort: failures are logged as warnings but do not fail the command.
If there are no obsolete logs, nothing is printed about cleanup.

After the agent processes the distill output and updates the soul, the agent is responsible for updating the
`last_distilled` timestamp in the soul file's frontmatter to the current time.

### `leiter instill <text>`

Outputs agent instructions for adding a preference to the soul file. Called by the agent when the user expresses a
preference ("remember", "learn", "instill", "always", "never", or similar language).

**Input:** A positional argument containing the preference or fact the user wants remembered.

**Output (stdout):**

1. The user's preference, quoted for clarity
2. Soul-writing guidelines (shared with `leiter distill`) covering entry format, specificity, placement, contradiction
   resolution, and examples
3. Instruction to read `~/.leiter/soul.md` and edit the appropriate section

See the Architecture section for why guidelines are shared between `instill` and `distill`.

### `leiter hook nudge`

Checks for stale undistilled session logs and outputs a nudge if any exist. Called by the SessionStart hook (after
`leiter hook context`) to remind the agent to suggest distillation.

**Behavior:**

1. Read `last_distilled` timestamp from `~/.leiter/soul.md` frontmatter
2. Scan `~/.leiter/logs/` for files whose filename timestamps are >= `last_distilled` (same inclusive comparison as
   `leiter distill`)
3. If any such file has a timestamp older than 24 hours ago (`now - 24h`): output a nudge message (defined in source
   code)
4. Otherwise: output nothing

If the soul file does not exist or the logs directory does not exist, silently output nothing and exit successfully.
Leiter may not be initialized yet — the nudge must not break the session.

**Output (stdout):**

- If stale undistilled logs exist: a short nudge message reminding the agent to suggest distillation
- Otherwise: nothing (zero context pollution)

### `leiter soul-upgrade`

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

The following hooks are configured in `~/.claude/settings.json` by the agent during `leiter setup install`:

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

## Flows

### First-Time Setup

1. User installs `leiter` binary
2. In a Claude Code session, user says "set up leiter"
3. Agent runs `leiter setup install`
4. The command creates `~/.leiter/` structure and outputs instructions
5. Agent reads the instructions and edits `~/.claude/settings.json` to add hooks
6. User reviews and approves the settings change
7. On next session start, leiter is active

### Normal Session (After Setup)

1. Session starts → SessionStart hook fires → `leiter hook context` outputs soul + instructions, `leiter hook nudge`
   outputs a distillation reminder if stale logs exist → agent has leiter hook context
2. Normal session proceeds
3. Session ends → SessionEnd hook fires → `leiter hook session-end` copies transcript to `~/.leiter/logs/`

### User Asks the Agent to Learn Something

1. User says "learn to always use snake_case for Rust functions"
2. Agent runs `leiter instill "always use snake_case for Rust functions"`
3. Agent receives writing guidelines and the quoted preference
4. Agent reads `~/.leiter/soul.md`
5. Agent edits the appropriate section following the guidelines
6. Preference is active in all future sessions

### Soul Upgrade

1. User updates `leiter` binary to a newer version
2. User says "upgrade the leiter soul"
3. Agent runs `leiter soul-upgrade`
4. If already current: agent relays that no upgrade is needed
5. If outdated: agent receives the upgrade instructions and new template
6. Agent reads current `~/.leiter/soul.md`, restructures it into the new format, and updates `soul_version` in the
   frontmatter

### Distillation

1. User says "distill my session logs" (or similar natural language)
2. Agent runs `leiter distill`
3. Agent receives all session logs since last distillation
4. Agent reads current `~/.leiter/soul.md`
5. Agent edits the soul to incorporate new learnings
6. Agent updates `last_distilled` in the frontmatter to the current timestamp

## Non-Goals (For Now)

- Multiple user profiles or project-specific souls
- Automatic distillation (always user-triggered)
- Soul backup
- API key management or direct Claude API calls from the CLI
