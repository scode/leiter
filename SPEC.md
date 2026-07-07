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
│  /leiter-instill (or "instill X") ──► /leiter-instill skill   │
│                           ──► agent edits soul.md            │
│                                                              │
│  /leiter-distill (or "distill") ──► /leiter-distill skill     │
│                           ──► sub-agent: leiter soul distill │
│                           ──► sub-agent edits soul.md        │
│                        ──► agent: leiter soul mark-distilled │
│                                                              │
│  /leiter-soul ──► leiter soul show ──► agent displays         │
│                                       soul verbatim          │
│                                                              │
│  /leiter-soul-upgrade ──► leiter soul upgrade                │
│                        ──► agent restructures soul.md        │
│                        ──► agent: leiter soul mark-upgraded  │
│                                                              │
│  SessionEnd hook ──► leiter hook session-end                 │
│                      ──► copies transcript to logs/          │
└──────────────────────────────────────────────────────────────┘

~/.leiter/
├── leiter.toml          # Main leiter settings
├── soul.md              # The "leiter soul" — agent instructions (pure markdown, no frontmatter)
├── state.toml           # Leiter-managed metadata (epochs, watermarks); agent never edits
└── logs/
    ├── 20260223T173000Z-abc123.jsonl
    ├── 20260223T190000Z-def456.jsonl
    └── ...

~/.claude/skills/
├── leiter-setup/SKILL.md        # Each contains <!-- SCODE_LEITER_INSTALLED -->
├── leiter-distill/SKILL.md
├── leiter-instill/SKILL.md
├── leiter-soul/SKILL.md
├── leiter-soul-upgrade/SKILL.md
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
- **`<claude_home>/skills/leiter-soul/SKILL.md`** — skill for showing the current soul file contents. Runs
  `leiter soul show` and displays the output verbatim.
- **`<claude_home>/skills/leiter-soul-upgrade/SKILL.md`** — skill for upgrading the soul template to the latest version.
  Runs `leiter soul upgrade` and follows its migration instructions.
- **`<claude_home>/skills/leiter-teardown/SKILL.md`** — skill that calls `leiter claude agent-teardown-instructions` to
  remove hooks.

Each skill file contains the sentinel string `SCODE_LEITER_INSTALLED` as an HTML comment. `leiter claude uninstall`
checks for this sentinel to verify that leiter was installed before removing files.

All skill files are `const &str` templates built into the binary. They are written to disk by `leiter claude install`
and overwritten on re-run (idempotent).

### `~/.leiter/soul.md`

The soul file is a pure markdown document with no frontmatter. It contains the learned preferences and instructions that
are injected into every Claude Code session. All CLI-managed metadata lives in `~/.leiter/state.toml` instead (see
below), so the file the agent reads is exactly the file the user's preferences live in — nothing else.

The agent owns the entire file and edits it directly using its Read/Edit/Write tools. The CLI writes to this file only
once, at `leiter claude install`, to lay down the initial template. It never writes the soul afterward: `mark-distilled`
and every other command touch only `state.toml`. Because no CLI metadata lives in the soul, the agent can no longer
corrupt leiter's bookkeeping by editing it — a malformed soul is at worst a malformed preferences document.

### Setup Epochs

The leiter binary may evolve in ways that require user action beyond just upgrading the binary — for example, re-running
`leiter claude install` to update hook configuration. Setup epochs detect this condition and alert the user.

There are two independent epochs, each a monotonic integer starting at 1:

- **`setup_soft_epoch`**: Bumped when a leiter upgrade introduces changes that benefit from user action but are not
  strictly required. A mismatch produces a nudge but does not block the session.
- **`setup_hard_epoch`**: Bumped when a leiter upgrade introduces changes that require user action before the session
  can function correctly. A mismatch blocks the session (the soul is not injected).

The binary has compiled-in expected values for both epochs. Epoch values are stored in `~/.leiter/state.toml`. Every
command except `session-end` validates the state file's epoch values against the binary's expected values before doing
any work. Hard epoch checks use exact equality — both older and newer state files are flagged. Soft epoch mismatches in
either direction produce a nudge but do not block commands. `leiter claude install` additionally migrates a
behind-the-binary soft epoch forward on re-run, and refuses to run when the state file is ahead of the binary (to avoid
downgrading). This validation is implemented as a single shared function used by all commands, preventing drift between
individual command implementations.

Either epoch field defaults to 1 when absent from `state.toml` (for backward compatibility with state files written
before that field existed).

A missing `state.toml` means leiter is not initialized — the same condition as a missing soul, with the same "run
`leiter claude install`" response.

Corrupt state (a `state.toml` that cannot be parsed, or whose `version` is unsupported) is treated equivalently to a
hard epoch mismatch — it blocks the command entirely, since epochs cannot be verified. Recovery is to delete
`state.toml` and re-run `leiter claude install`. The tradeoff is deliberate: deleting `state.toml` discards the
distillation watermarks and stamps a fresh `last_distilled` of the re-install time. Because that timestamp is also the
floor for the external Claude scan, sessions that ran before the re-install are treated as already distilled and are not
re-read — so recovery does not flood the next distill with redundant history. The cost lands the other way: any session
that had genuinely not been distilled yet but predates the re-install is skipped (its watermark is gone and its file
mtime is below the new floor). That is still not destructive — no user data is lost, only a slice of pre-recovery
learning opportunity — and it mirrors the "learning starts at install" semantics of a fresh install.

`session-end` is exempt from epoch checks. It only copies transcript files to a known directory, and losing session data
is worse than any epoch-related risk.

Epochs are independent of `soul_version`. The soul version tracks template format changes (handled by
`leiter soul upgrade`). Epochs track integration changes (hooks, settings, etc.) that require user action outside the
soul file.

#### Legacy layout migration

A migration routine converts the pre-`state.toml` layout — a `soul.md` carrying YAML frontmatter, plus an optional
`~/.leiter/codex-meta.toml` — into the current layout: a `state.toml` populated from the old frontmatter and codex-meta
fields, a frontmatter-free `soul.md`, and no `codex-meta.toml` (it is deleted). The routine is re-runnable from any
crash point: it keeps an existing `state.toml` rather than overwriting it, rewrites the soul atomically (temp file plus
rename — the soul is the one file whose contents cannot be regenerated), and tolerates an already-stripped soul as a
resumed half-migration. In this revision the routine exists and is unit-tested but is not yet invoked by any command; a
later revision wires it into `leiter claude install`. Until then only fresh installs produce `state.toml`, and
pre-existing installs are handled when that later revision lands. This is acceptable because releases are cut
explicitly, so no user is exposed to the interim main-branch-only state.

#### Epoch Error Messages Delivered to the User

When `leiter hook context` or `leiter hook nudge` detects an incompatibility, the output is an instruction to the agent.
The agent must deliver the quoted message to the user **verbatim** — the instruction must use strong compliance language
(e.g. "EXACTLY this (word for word)") to maximize the chance the agent relays it unchanged. The exact user-facing
phrases for each case:

- **Setup outdated** (state hard epoch < binary): "Leiter setup needs to be re-run — please run `leiter claude install`
  in your terminal and follow the instructions, then start a new session."
- **Binary outdated** (state hard epoch > binary): "Your leiter binary is older than your soul file expects — please
  upgrade leiter, then start a new session."
- **Corrupt state**: "The leiter state file is corrupt. Please delete [path] and run `leiter claude install` to
  re-initialize, then start a new session." Here `[path]` is the resolved `state.toml` path.
- **State unreadable** (I/O or permission error, as opposed to corrupt content): "The leiter state file could not be
  read. Please check file permissions on [path], then start a new session." This deliberately does not suggest deleting
  the file — an unreadable state file may be fully intact, and deleting it would discard watermarks for no reason.
- **Soul unreadable**: "The leiter soul file could not be read. Please check file permissions on [path], then start a
  new session."

The instruction must also tell the agent not to attempt leiter commands for the remainder of the session.

For soft epoch mismatches, the agent is instructed to briefly mention that optional improvements are available (or that
the binary is a bit behind) and suggest the appropriate action (re-run install or upgrade). The nudge explicitly notes
there are no breaking changes. These are nudges, not verbatim scripts — the agent is told to keep it to one short
sentence.

### Soul Template (built into the binary)

The `leiter` binary contains a built-in soul template (~1 page) that defines the initial structure and categories for
the soul. When `leiter claude install` creates `~/.leiter/soul.md`, it writes this template as the initial content
verbatim — no frontmatter is prepended, since all metadata lives in `state.toml`. The template nudges the agent toward
capturing specific kinds of information.

The template content is defined in source code as a well-identified constant (not inline in this spec). It should
include section headings and brief descriptions of what belongs in each section (e.g., communication style, coding
preferences, workflow patterns, tool preferences).

### `~/.leiter/logs/`

Session transcripts, one file per session. Named `<UTC_ISO8601_BASIC>-<session_id>.jsonl` (e.g.,
`20260223T173000Z-abc123.jsonl`) using UTC ISO 8601 basic format (`YYYYMMDDTHHMMSSZ`) for the timestamp and the Claude
Code session ID as a suffix. The session ID makes it easy to associate a log file with a specific session for debugging.
Each file is a session transcript (JSONL) copied from the Claude Code transcript path provided by the SessionEnd hook.

All timestamps in leiter — `state.toml` values, log filenames, and CLI output — use UTC ISO 8601 format. `state.toml`
uses extended format (`2026-02-23T17:00:00Z`). Filenames use basic format (`20260223T173000Z`) to avoid colons and other
filesystem-unfriendly characters.

### `~/.leiter/leiter.toml`

Main leiter settings stored as TOML.

Logical shape:

```toml
enable_codex_experimental = false
```

`enable_codex_experimental` defaults to `false` when the file is missing. When false, `leiter soul distill` and
`leiter soul mark-distilled` must not read Codex rollout files, must not consult the `[codex.*]` tables in
`~/.leiter/state.toml`, and must not modify those tables' contents. Since `state.toml` is core state (not
Codex-specific), commands still load and rewrite the file as a whole — the requirement is that a disabled gate leaves
the `[codex.*]` table contents exactly as they were, and epoch validation and `last_distilled` work unaffected. This
gate is Codex-only: the external Claude session scan and its `[claude.*]` tables are always active and are never gated
on this flag.

### `~/.leiter/state.toml`

All CLI-managed metadata, stored as TOML. Leiter owns this file; the agent must never edit it. It unifies what used to
be split between the soul frontmatter (`last_distilled`, `soul_version`, both setup epochs) and `codex-meta.toml` (the
Codex distillation watermarks). Writes are atomic: leiter writes a temporary file in the same directory and renames it
over `state.toml`, so a crash mid-write never leaves a torn file.

Logical shape:

```toml
version = 1
soul_version = 2
setup_soft_epoch = 2
setup_hard_epoch = 1
last_distilled = 2026-07-01T12:00:00Z

[claude.committed."<session_id>"]
path = "/home/alice/.claude/projects/-home-alice-proj/<uuid>.jsonl"
size_bytes = 12345
mtime_utc = 2026-07-01T11:59:00Z
session_timestamp_utc = 2026-07-01T11:40:12Z # optional
# latest_event_timestamp_utc omitted for Claude sessions

[claude.pending."<session_id>"]
# same fields as committed

[codex.committed."<session_id>"]
path = "/Users/alice/.codex/sessions/2026/03/07/rollout-....jsonl"
size_bytes = 12345
mtime_utc = 2026-03-07T18:10:00Z
session_timestamp_utc = 2026-03-07T18:06:57Z # optional
latest_event_timestamp_utc = 2026-03-07T18:09:25Z # optional

[codex.pending."<session_id>"]
# same fields as committed
```

- `version`: the state-file schema version, currently `1`. An unrecognized (unsupported) version is an error, handled
  the same as corrupt state (see Setup Epochs).
- `soul_version`, `setup_soft_epoch`, `setup_hard_epoch`, `last_distilled`: the fields that previously lived in soul
  frontmatter, with identical meaning. `soul_version` drives `leiter soul upgrade`; the epochs drive Setup Epochs
  validation; `last_distilled` drives which session logs `leiter soul distill` treats as new.
- `pending_scan_started_utc` (optional): the scan-start time staged by the most recent non-dry-run distill.
  `mark-distilled` consumes it as the new `last_distilled` (see that command). Absent when no distill has staged
  anything since the last mark.

The `[codex.*]` tables carry the Codex distillation watermarks previously kept in `codex-meta.toml`, with the same
semantics. `committed` is the last successfully marked-distilled Codex watermark set. `pending` is staged by
`leiter soul distill` and promoted into `committed` by `leiter soul mark-distilled`. The dedupe watermark is per-session
file state (`path`, `size_bytes`, `mtime_utc`) rather than a single global timestamp because Codex sessions can be
resumed and appended.

The `[claude.*]` tables are the exact same shape and serve the same role for the external Claude session scan (see
Claude session scanning under `leiter soul distill`). The fields carry identical meaning, with one difference: for a
Claude session the optional `session_timestamp_utc` holds the first record timestamp found in the transcript file (used
for ordering), and `latest_event_timestamp_utc` is omitted — the Claude canonicalizer does not track a distinct
latest-event time the way the Codex path does.

`pending` exists so `mark-distilled` can commit the exact file state that `distill` actually showed to the LLM. Without
`pending`, `mark-distilled` would have to either leave those watermarks untouched forever or re-scan the session stores
at mark time and risk committing a newer file state than the LLM actually saw if a session changed between the two
commands. This holds for both `[claude.*]` and `[codex.*]`.

The tables are nested under `[codex.*]` and `[claude.*]` (rather than a flat `[committed]`/`[pending]`) so each
harness's watermark map lives in its own namespace and neither collides with the other.

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

### `leiter config set <key> <value>`

Writes a persistent setting to `~/.leiter/leiter.toml`.

**Behavior:**

1. Load `~/.leiter/leiter.toml` if it exists; if it is unreadable or invalid, warn and continue from defaults
2. Validate the key/value pair
3. Persist the updated config back to `~/.leiter/leiter.toml`

**Supported keys:**

- `enable_codex_experimental`: boolean (`true` or `false`)

**Output (stdout):** A confirmation message of the form `<key> set to <value>`.

**Errors:** Unknown keys and invalid values exit with a non-zero code.

### `leiter claude install`

First-time setup. Performs deterministic initialization of the state directory and writes plugin files (skills and
sentinel) to the Claude Code home directory.

**Deterministic steps:**

1. Create `~/.leiter/` directory (no-op if exists)
2. Create `~/.leiter/logs/` directory (no-op if exists)
3. Converge the soul/state pair by which of the two files exist. "Fresh state" below means a `state.toml` with
   `version = 1`, `last_distilled` set to the current UTC time, `soul_version` set to the current template version, and
   both epochs set to the binary's current values. Stamping install time (rather than the Unix epoch) matters because
   `last_distilled` now also acts as the floor for the external Claude scan (see Claude session scanning): under an
   epoch-0 convention a fresh install's very first distill would ingest the entire Claude Code retention window of
   sessions that predate the install, which is not what the old hook-based system did — it only ever learned from
   sessions that ran after setup. Stamping install time preserves that "learning starts at install" semantics. "Verify
   epochs" means: hard epochs must exactly match (any mismatch is an error, using the direction-specific messages from
   Setup Epochs); a soft epoch behind the binary is migrated forward by rewriting `state.toml` (preserving all other
   fields); a soft epoch ahead of the binary is an error; corrupt or unsupported-version `state.toml` is an error. The
   four cases:
   - **Neither exists** (fresh install): write fresh state, then the template soul.
   - **Soul exists, state missing**: two situations share this shape, discriminated by whether the soul still parses as
     YAML frontmatter. If it does, this is the pre-`state.toml` legacy layout: fail with an error saying migration
     arrives in a later revision (see Legacy layout migration). If it does not, this is the documented corrupt-state
     recovery path (the user deleted `state.toml`): write fresh state and keep the soul untouched — preferences survive,
     watermarks reset.
   - **Soul missing, state exists**: verify epochs first, then recreate the soul from the template. The existing state
     is kept, not reset — its watermarks may be perfectly valid.
   - **Both exist**: verify epochs.

   Ordering invariant: whenever both files are written, `state.toml` is written before `soul.md`, and validation runs
   before any write. This keeps a torn or refused install self-healing: dying between the two writes leaves the
   soul-missing/state-present shape, which the next run repairs, never the soul-present/state-missing shape that reads
   as a legacy layout
4. Verify the Claude Code home directory exists (error if not — Claude Code not installed)
5. Write all six skill files to their respective directories under `<claude_home>/skills/`. Overwrites existing files on
   re-run (idempotent)

**Output (stdout):** A success message listing the available skills and telling the user to run `/leiter-setup` to
configure hooks.

If any step fails, the output instructs the agent to relay the error to the user.

### `leiter claude uninstall`

Removes leiter plugin files from the Claude Code home directory. Does NOT touch `~/.leiter/` (soul and logs) or
`~/.claude/settings.json` (hooks are removed via the `agent-teardown-instructions` subcommand or manually).

**Behavior:**

1. Scan skill directories under `<claude_home>/skills/` for a `SKILL.md` containing `SCODE_LEITER_INSTALLED`
2. If no skill file contains the sentinel: error
3. Remove all six `<claude_home>/skills/leiter-*/` directories (best-effort, skip missing)

**Output (stdout):** A success message with guidance on how to remove hooks, fully clean up (`~/.leiter/`), and
re-enable later.

**Errors:** If the sentinel is missing or unreadable, exit with a non-zero code.

### `leiter claude agent-setup-instructions`

Outputs natural language instructions for the agent to configure Claude Code hooks in `~/.claude/settings.json`. This is
the same hook configuration content that `leiter claude install` used to output directly. It is called by the
`/leiter-setup` skill.

**Behavior:** Validates state (see Setup Epochs). If incompatible, exits with an error.

**Output (stdout):** Instructions including the exact JSON hook entries for `SessionStart` and `SessionEnd`, plus
three-case logic for handling fresh install, upgrade, and already-configured states. See Hook Configuration below for
the exact hook JSON. After hooks are configured, includes an optional permissions prompt (see Permissions below).

### `leiter claude agent-teardown-instructions`

Outputs natural language instructions for the agent to remove leiter hooks from `~/.claude/settings.json`. Called by the
`/leiter-teardown` skill.

**Behavior:** Validates state (see Setup Epochs). If incompatible, exits with an error.

**Output (stdout):** Instructions telling the agent to find and remove hook entries whose commands contain
`"leiter hook context"`, `"leiter hook nudge"`, or `"leiter hook session-end"`, clean up empty arrays, preserve
non-leiter hooks, remove leiter permission entries (see Permissions below), and provide cleanup/re-enable guidance to
the user.

### `leiter hook context`

Outputs the soul content and agent instructions. Called by the SessionStart hook.

**Behavior:**

1. Validate state (see Setup Epochs). If the soul is missing, `state.toml` is corrupt (unparseable or unsupported
   version), or there is a hard epoch mismatch: output an error message and return without injecting the soul
2. If `setup_soft_epoch` in `state.toml` does not exactly match the binary's expected value: output a nudge message
   (different for older vs. newer state) but continue to inject the soul normally
3. Output the preamble and full soul content (the soul file is emitted as-is; it has no frontmatter to strip)

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

   **Soul viewing:** When the user asks to see or view their soul, the agent should invoke the `/leiter-soul` skill.

   **Soul upgrade:** When the user asks to upgrade the leiter soul (or runs `/leiter-soul-upgrade`), the agent should
   invoke the `/leiter-soul-upgrade` skill.

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

**No-op case:** If the transcript file does not exist (e.g., Claude Code does not write a transcript for zero-turn
sessions), log a debug message and exit successfully. No log file is created.

**Errors:** If `~/.leiter/logs/` does not exist, the transcript file exists but cannot be read, the write fails, or the
atomic rename fails, print an error to stderr and exit with a non-zero code. Clean up the temporary file on any error.

### `leiter soul distill`

Outputs session logs that haven't been processed since the last distillation. In this revision it draws from two Claude
sources that coexist: the legacy `~/.leiter/logs/` files copied by the SessionEnd hook, and the external scan of Claude
Code's own session store (see Claude session scanning). Both stay active — the hook still runs — and their output is
deduplicated by session id so a session present in both places is emitted once. A later revision removes the hook and
with it the legacy path.

**Flags:**

- `--dry-run`: report what would be deleted during obsolete-log cleanup instead of deleting, and skip the pending
  watermark writes.
- `--claude-home <path>`: override the Claude home directory whose `projects/` subtree the external scan walks (default
  `~/.claude/`). Primarily for testing. This is a distinct flag from the `--claude-home` on the `leiter claude`
  subcommand, which is unrelated plumbing scoped to plugin installation.
- `--codex-home <path>`: override the Codex home directory the Codex scan walks (default `~/.codex/`). Primarily for
  testing.

**Behavior:**

1. Validate state (see Setup Epochs). If incompatible, exit with an error
2. Read `last_distilled` timestamp from the validated `state.toml`
3. Scan `~/.leiter/logs/` for files whose filename timestamps (the `YYYYMMDDTHHMMSSZ` prefix, ignoring the session ID
   suffix) are newer than or equal to `last_distilled`. The inclusive comparison (>=) ensures that a log written in the
   same second as the distillation timestamp is not lost — this matters because the distillation flow has the agent
   write a session log immediately before running `leiter soul distill`, and the two timestamps could collide
4. Load `~/.leiter/leiter.toml`. If it is unreadable or invalid, warn and use defaults
5. Scan the external Claude session store under `<claude_home>/projects/` (default `~/.claude/`, overridable with
   `--claude-home`). This scan is unconditional — it is **not** gated on `enable_codex_experimental` (that gate is
   Codex-only). Discovery is recursive but fail-useful about Claude Code's undocumented layout: only regular files
   (symlinks are never followed), only `.jsonl` files whose filename stem parses as a UUID (that stem is the session
   id), everything else silently skipped; unreadable files or directories are warned about and skipped, never fatal. See
   Claude session scanning below for the full contract
6. Read the Claude `[claude.committed]` watermarks from the validated `state.toml` (same core-state note as the Codex
   watermarks in step 10). For each discovered session, compare the current file watermark (`path`, `size_bytes`,
   `mtime_utc`) to the committed watermark. If unchanged, skip the session completely. If changed or new, re-read the
   full transcript and emit the full canonicalized session so the LLM sees the entire updated context. One exception is
   the `last_distilled` floor: a discovered session with no committed watermark whose file mtime is strictly older than
   `last_distilled` is treated as already distilled under the pre-scan regime and skipped without emission (and without
   staging a watermark). See Claude session scanning
7. Dedupe the external scan against legacy logs: any session id the external store accounts for suppresses the
   `~/.leiter/logs/` file bearing the same session-id suffix. "Accounts for" means the session is being emitted from the
   external copy this run, or its committed watermark matched unchanged — the latter covers the ordinary idle-exit
   sequence where the SessionEnd hook copies a log after `mark-distilled` already committed the session, which would
   otherwise re-emit the whole session from the legacy copy. Floor-skipped sessions are deliberately not in the
   suppression set: for those the legacy log may be the only copy that would ever emit. The suppressed legacy file still
   participates in obsolete-log cleanup below
8. If `enable_codex_experimental = true`, best-effort scan Codex rollout transcripts under
   `<codex_home>/sessions/**/*.jsonl` and `<codex_home>/archived_sessions/**/*.jsonl` (default `~/.codex/`, overridable
   with `--codex-home`)
9. If `enable_codex_experimental = true`, for each Codex rollout file, read the leading `session_meta` record and use
   `payload.id` as the stable session ID. Files without a readable leading `session_meta` record are skipped with a
   warning
10. If `enable_codex_experimental = true`, read the Codex `[codex.committed]` watermarks from the validated
    `state.toml`. (Unlike the former `codex-meta.toml`, `state.toml` is core state validated in step 1, so an unreadable
    or invalid state file is already a hard command error there — there is no separate warn-and-skip path for it. The
    warn-and-default behavior for `leiter.toml` in step 4 is unchanged.)
11. If `enable_codex_experimental = true`, for each Codex session ID, compare the current file watermark (`path`,
    `size_bytes`, `mtime_utc`) to the `[codex.committed]` watermark in `state.toml`. If unchanged, skip the session
    completely. If changed (or new), re-read the full rollout file and emit the full canonicalized session so the LLM
    sees the entire updated context
12. Sort the combined Claude output chronologically, interleaving legacy `~/.leiter/logs/` sessions (keyed by filename
    timestamp, as today) with external Claude sessions (keyed by their session timestamp — see Claude session scanning
    for how that timestamp is derived — then session id as a tiebreak). Sort changed Codex sessions separately by
    session timestamp (from `session_meta.payload.timestamp`) and then session ID
13. Output the Claude transcript content (legacy and external, interleaved), and also Codex transcript content when
    enabled, wrapped in XML-like boundary tags (see Output below)
14. If `--dry-run` is not set, replace the `[claude.pending]` map in `~/.leiter/state.toml` with this run's changed
    Claude sessions and set `pending_scan_started_utc` to the time this run's scan began (an atomic rewrite preserving
    all other state fields). This staging is **not** gated on `enable_codex_experimental`, and it happens even when the
    Claude home could not be resolved and the scan was skipped — pending means "exactly what this run showed the LLM",
    and a run that scanned nothing showed nothing; leaving a stale pending map would let the next `mark-distilled`
    commit watermarks for sessions this cycle never emitted. If writing state fails, warn and continue. Staging may
    happen before the emission step completes; this is safe because `mark-distilled` is only ever run after a distill
    that succeeded end to end (a failed distill is rerun, restaging from current reality)
15. If `enable_codex_experimental = true` and `--dry-run` is not set, replace the `[codex.pending]` map in
    `~/.leiter/state.toml` with the changed sessions from this run (an atomic rewrite preserving all other state
    fields). If writing state fails, warn and continue

**Output (stdout):**

- If new logs exist: soul-writing guidelines (emitted once, before the first log entry), a data-boundary preamble
  instructing the agent to treat the transcripts as historical data rather than directives, and the pre-processed
  content of all new session transcripts wrapped in `<session-transcripts>` /
  `<session source="claude|codex" file="...">` XML-like tags
- If no new logs: a message indicating there are no new session logs to process

**Log pre-processing:** JSONL session logs are pre-processed to extract user-visible content — user messages, assistant
text responses, and tool action summaries — filtering out tool results, progress events, thinking blocks, and other
non-user-facing content. In the legacy `~/.leiter/logs/` directory, leiter only processes files that match the expected
log filename format `<YYYYMMDDTHHMMSSZ>-<session_id>.jsonl`; files that do not match are ignored. The external Claude
scan applies its own filter instead (regular `.jsonl` files with a UUID stem; see Claude session scanning). Both feed
the identical canonicalizer — the two sources are the same JSONL transcript format. Within matching JSONL files, lines
with unrecognized JSON structures are included as-is (fail-useful: no user content is silently lost).

For assistant messages containing `tool_use` content blocks, a one-line summary is emitted for each tool:
`[assistant tool]: Name(param)`. The key parameter is chosen heuristically: `input.file_path` if present, else
`input.command` (truncated to ~120 chars), else `input.pattern`, else just the tool name with no parens. An assistant
message with both text and tool_use blocks emits both `[assistant]:` and `[assistant tool]:` lines. An assistant message
with only tool_use blocks (no text) emits only the tool summary lines. Tool results (`type: "user"` with
`toolUseResult`) remain dropped — the tool name from the assistant side provides sufficient context.

**Claude session scanning:** In addition to the hook-copied logs in `~/.leiter/logs/`, `leiter soul distill` reads
Claude Code's own session transcripts directly from the Claude home directory. Claude Code stores one JSONL transcript
per session at `<claude_home>/projects/<cwd-slug>/<session-uuid>.jsonl`, appended live while the session runs. This
external scan is always active in this revision; it is not gated on `enable_codex_experimental`.

Discovery starts at `<claude_home>/projects/` (default `~/.claude/projects/`, override the home with `--claude-home`)
and recurses, but it is deliberately fail-useful about this undocumented layout. It considers only regular files; it
never follows symlinks — a UUID-named symlink must not be able to pull an arbitrary readable file into the distill
output. The symlink guarantee is enforced at read time, not just at discovery: the content read verifies the opened
handle still refers to the same regular file seen during the walk (filesystem identity check) and reads through that
handle, so a file swapped for a symlink between the two moments is skipped rather than followed. It considers only
`.jsonl` files whose filename stem parses as a UUID, and that stem is taken as the session id. If multiple discovered
files share a session id (not something Claude Code normally produces), the newest wins — latest mtime, then largest
size — and only that file is emitted and watermarked; anything else would stage one watermark while emitting both,
leaving the loser to re-emit forever. Everything else — subdirectories like `memory/`, stray non-UUID files,
non-`.jsonl` files — is silently skipped. Files and directories that cannot be read are warned about and skipped; an
unreadable entry is never fatal to the command. Because the emitted `file` label embeds arbitrary directory names from
this undocumented layout, characters that could forge or break the `<session>` tag attribute (quotes, angle brackets,
control characters) are replaced before emission.

The watermark model is identical to Codex: each discovered session's `(path, size_bytes, mtime_utc)` is compared against
its `[claude.committed]` entry. Unchanged sessions are skipped entirely. Changed or new sessions are re-read and emitted
in full. Emitting the whole session on any change is what handles a session distilled mid-run and later resumed — Claude
Code materializes a resumed session as a **new** transcript file that duplicates the prior history, so re-emitting in
full keeps the LLM's view complete rather than stitching deltas.

The `last_distilled` floor guards the first scan-enabled distill from re-ingesting history the soul already learned.
Sessions distilled through the old hook-copied-logs flow are still sitting in the Claude home within Claude Code's
retention window, so without a floor the first external scan would re-emit all of them. The rule: a discovered session
with **no** committed watermark whose file mtime is strictly older than `last_distilled` is treated as already distilled
under the pre-scan regime and is skipped without emission — and it stages no watermark, so it stays invisible until its
file actually changes. A session that **does** have a committed watermark follows the ordinary watermark comparison
regardless of mtime (its history is by definition already accounted for).

Ordering: external Claude sessions sort by session timestamp — the first `timestamp` field found among the file's
leading records, where header inspection reads at most a small bounded number of lines — falling back to the file mtime
when no such field is found, then by session id as a tiebreak. In the combined Claude output these interleave
chronologically with the legacy `~/.leiter/logs/` sessions (which sort by their filename timestamp, as today). Codex
ordering is unchanged and separate.

Emission uses the same `<session source="claude" file="...">` wrapping and the same canonicalization and filtering as
the hook-copied logs, because they are byte-for-byte the same file format. For an externally scanned session the `file`
label is the transcript path relative to the Claude home (e.g. `projects/-home-alice-proj/<uuid>.jsonl`).

A scanned session that canonicalizes to no user-visible content is not emitted, but it is still staged in
`[claude.pending]` (mirroring the Codex behavior) so that once `leiter soul mark-distilled` commits it, the session
stops being rescanned on every run.

Dedupe against the legacy logs is what keeps this coexistence clean. In this revision the SessionEnd hook still copies
every finished transcript into `~/.leiter/logs/`, so most sessions exist in both places at once. A session id emitted
from the external scan suppresses the legacy log file bearing the same session-id suffix — the external copy is the same
or fresher content. The suppressed legacy file still participates in obsolete-log cleanup exactly as it does today. When
a later revision removes the SessionEnd hook, the legacy path and this dedupe go away with it.

Codex rollout files use a different event schema and are canonicalized separately. Leiter keeps user-visible user
messages, assistant/user-facing output text, commentary updates shown to the user, and one-line tool call summaries. It
drops developer/system scaffolding, reasoning, token counts, raw tool results, and other machine-only noise.

**Codex access constraints:** Codex support must never read SQLite, must never write or delete anything under
`~/.codex/`, and must never fail the overall distill command when the Codex directory is missing, malformed, or
unexpected. When `enable_codex_experimental = false`, the command must not read Codex rollout files and must not read or
modify the contents of the `[codex.*]` tables in `~/.leiter/state.toml` (it still loads and rewrites the file as a
whole; the disabled gate just passes those tables through unchanged).

**Obsolete log cleanup:** After outputting new logs (or reporting that there are none), the command collects log files
whose filename timestamps are strictly before `last_distilled` — these have already been processed by a prior
distillation and are no longer needed. It deletes them. If `--dry-run` is passed, it reports which files would be
deleted instead of deleting them. Deletion is best-effort: failures are logged as warnings but do not fail the command.
If there are no obsolete logs, nothing is printed about cleanup. Codex rollout files are never deleted or modified.

### `leiter soul mark-distilled`

Commits the last distill run's cutoff: `last_distilled` advances to the scan-start time that run staged in
`pending_scan_started_utc`, not to the mark-time wall clock. The distinction closes a loss window — a session created
after distill's scan but before the mark would sit below a mark-time floor forever (no committed watermark, mtime too
old), and the same window would delete hook-copied logs as obsolete without ever distilling them. A scan-start cutoff
guarantees everything born after the scan is the next run's responsibility. This is the only way `last_distilled` should
be updated — the agent must never edit it manually. This command never writes `soul.md`.

**Behavior:**

1. Validate state (see Setup Epochs). If incompatible, exit with an error
2. Load `~/.leiter/leiter.toml`. If it is unreadable or invalid, warn and use defaults
3. Merge the `[claude.pending]` map into `[claude.committed]` and clear `[claude.pending]`. This always happens — it is
   **not** gated on `enable_codex_experimental`, since the external Claude scan is always active
4. Set `last_distilled` to the staged `pending_scan_started_utc` (clearing that field), falling back to the current UTC
   time when nothing is staged (a mark without a preceding non-dry-run distill). If `enable_codex_experimental = true`,
   also merge the `[codex.pending]` map into `[codex.committed]` and clear `[codex.pending]`
5. Write `state.toml` back in a single atomic write, preserving all other fields. There is no separate best-effort path
   for either merge: both the Claude and (when enabled) Codex promotions ride the same write as `last_distilled`, so the
   whole thing either commits or the command fails. (The old warn-and-continue behavior existed because Codex watermarks
   lived in a separate best-effort file; that split no longer exists.)

When `enable_codex_experimental = false`, `leiter soul mark-distilled` must not consult or modify the contents of the
`[codex.*]` tables in `~/.leiter/state.toml` — the rewrite that updates `last_distilled` passes them through unchanged.

**Output (stdout):** A confirmation message including the exact timestamp that was set.

**Errors:** If state is incompatible (soul missing, corrupt state, or epoch mismatch), exit with a non-zero code and an
error message on stderr.

### `leiter soul instill <text>`

Outputs agent instructions for adding a preference to the soul file. Called by the agent when the user expresses a
preference ("remember", "learn", "instill", "always", "never", or similar language).

**Input:** A positional argument containing the preference or fact the user wants remembered.

**Behavior:**

1. Validate state (see Setup Epochs). If incompatible, exit with an error

**Output (stdout):**

1. The user's preference, quoted for clarity
2. Soul-writing guidelines (shared with `leiter soul distill`) covering entry format, specificity, placement,
   contradiction resolution, recording judgment, and examples
3. Instruction to read `~/.leiter/soul.md` and edit the appropriate section

See the Architecture section for why guidelines are shared between `instill` and `distill`.

### `leiter soul show`

Outputs the full soul file wrapped in XML boundary tags for safe verbatim display. Called by the `/leiter-soul` skill
when the user asks to see their soul.

**Behavior:**

1. Validate state (see Setup Epochs). If incompatible, exit with an error

**Output (stdout):**

The full contents of `~/.leiter/soul.md` wrapped in `<leiter-soul-content>` / `</leiter-soul-content>` XML tags. The
soul is pure preferences content with no frontmatter, so the whole file is the learned preferences the user wants to see
— there is no internal metadata to strip.

The XML boundary tags, combined with skill instructions that tell the agent to display content verbatim in a fenced code
block, mitigate the risk of the agent interpreting soul content as directives. The skill instructions tell the agent to
use enough backtick characters in the fence to avoid conflicts with any backticks in the soul content, since the soul
body may contain markdown including fenced code blocks.

### `leiter hook nudge`

Checks for stale undistilled session logs and outputs a nudge if any exist. Called by the SessionStart hook (after
`leiter hook context`) to remind the agent to suggest distillation.

**Flags:**

- `--auto-distill`: Use a 4-hour threshold instead of 24 hours, and output an instruction for the agent to run
  distillation (instead of asking the user). This is opt-in via `/leiter-setup` option 3.

**Behavior:**

1. Validate state (see Setup Epochs). If `state.toml`, the soul, or the logs directory does not exist, silently output
   nothing and exit successfully. If `state.toml` is corrupt (unparseable or unsupported version) or there is a hard
   epoch mismatch, output an error message and exit successfully (the hook must never fail the session). If the logs
   directory cannot be read, silently output nothing
2. Read `last_distilled` timestamp from the validated `state.toml`
3. Scan `~/.leiter/logs/` for files whose filename timestamps are >= `last_distilled` (same inclusive comparison as
   `leiter soul distill`)
4. If any such file has a timestamp older than the threshold (`now - 24h`, or `now - 4h` with `--auto-distill`): output
   a message (defined in source code)
5. Otherwise: output nothing

**Output (stdout):**

- Without `--auto-distill`: if stale undistilled logs exist (24h), a short nudge message reminding the agent to suggest
  distillation
- With `--auto-distill`: if stale undistilled logs exist (4h), an instruction for the agent to invoke distillation
- Otherwise: nothing (zero context pollution)

### `leiter soul upgrade`

Detects soul template drift and outputs agent instructions to migrate the existing soul to the current template format.
Invoked by the `/leiter-soul-upgrade` skill (or directly by the agent when the user asks to upgrade the soul using
natural language).

**Behavior:**

1. Validate state (see Setup Epochs). If incompatible, exit with an error
2. Compare `soul_version` in `state.toml` against the current template version built into the binary
3. If already up to date: output a message saying so
4. If outdated: output upgrade instructions for the agent (see below). This command does not write `soul_version` itself
   — the version advances only via `leiter soul mark-upgraded`, so a botched or abandoned restructuring leaves the soul
   correctly reported as outdated and re-running upgrade re-emits the instructions

**Output when outdated:**

1. A changelog of what changed in each version between the user's current version and the latest, one brief summary per
   version (like a soul template changelog)
2. The full current template with its version number
3. Instructions for the agent to restructure the existing soul content into the new format while preserving all learned
   preferences, then run `leiter soul mark-upgraded` as the final step. The agent edits only the soul file;
   `soul_version` lives in `state.toml` and is written only by the CLI

The changelog entries are maintained in the source code as human- and agent-readable text. There is no required
structure — each entry is a brief prose description of what changed in that soul template version. New entries are added
when the soul template is modified in future code changes. The agent performs the actual soul file edits.

### `leiter soul mark-upgraded`

Sets `soul_version` in `~/.leiter/state.toml` to the binary's current template version. This is the only way
`soul_version` advances — the agent must never edit it. Run by the agent as the final step of the upgrade flow, after
restructuring the soul.

The failure direction is deliberate, mirroring `mark-distilled`: if the agent restructures the soul but forgets this
step, the soul merely keeps reporting as outdated and the next `leiter soul upgrade` re-emits instructions (harmless
re-prompt). The optimistic alternative — bumping the version when instructions are emitted — would mark a botched
upgrade as done.

**Behavior:**

1. Validate state (see Setup Epochs). If incompatible, exit with an error
2. Set `soul_version` to the binary's current template version and write `state.toml` back atomically, preserving all
   other fields

**Output (stdout):** A confirmation message including the version that was set.

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
reminder only when stale undistilled logs exist (otherwise it outputs nothing, adding zero context). If the user opts
into auto-distillation during `/leiter-setup` (option 3), the nudge command is configured as
`leiter hook nudge --auto-distill`, which uses a 4-hour threshold and instructs the agent to run distillation.

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

After configuring hooks, `agent-setup-instructions` offers three optional features:

1. **Bash commands:** `"Bash(leiter:*)"` — allows all leiter CLI commands without confirmation dialogs.
2. **Soul file access:** `"Read(<soul_path>)"`, `"Edit(<soul_path>)"`, and `"Write(<soul_path>)"` — allows reading,
   editing, and writing the soul file without confirmation dialogs. Claude Code's `permissions.allow` uses
   gitignore-style path matching: `/path` is project-relative, `//path` is absolute, and `~/path` is home-relative. A
   bare absolute path like `/Users/alice/.leiter/soul.md` would be interpreted as project-relative and never match. The
   soul path must be formatted as `~/.leiter/soul.md` (when under `$HOME`) or `//path/to/soul.md` (otherwise).
3. **Auto-distillation:** Changes the nudge hook command from `leiter hook nudge` to `leiter hook nudge --auto-distill`,
   so the agent runs distillation at session start when stale logs exist (4h threshold) instead of asking the user.

The user can accept any combination, all, or none.

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
7. Agent presents optional features (Bash permissions, soul file access, auto-distillation); user accepts any
   combination or none
8. On next session start, leiter is active

### Normal Session (After Setup)

1. Session starts → SessionStart hook fires → `leiter hook context` outputs soul + instructions, `leiter hook nudge`
   outputs a distillation reminder if stale logs exist (or instructs distillation when `--auto-distill` is enabled) →
   agent has leiter hook context
2. Normal session proceeds
3. Session ends → SessionEnd hook fires → `leiter hook session-end` copies transcript to `~/.leiter/logs/`

### User Asks the Agent to Learn Something

1. User runs `/leiter-instill` (or says "instill", "remember", "always", "never", etc. — the agent auto-matches the
   skill)
2. Skill runs `leiter soul instill "always use snake_case for Rust functions"`
3. Agent receives writing guidelines and the quoted preference
4. Agent reads `~/.leiter/soul.md`, edits the appropriate section following the guidelines
5. Preference is active in all future sessions

### Soul Upgrade

1. User updates `leiter` binary to a newer version
2. User runs `/leiter-soul-upgrade` (or says "upgrade the leiter soul" — the agent auto-matches the skill)
3. Skill runs `leiter soul upgrade`
4. If already current: agent relays that no upgrade is needed
5. If outdated: agent receives the upgrade instructions and new template
6. Agent reads current `~/.leiter/soul.md`, restructures it into the new format, then runs `leiter soul mark-upgraded`
   to record the new `soul_version` in `state.toml` (the agent never edits version metadata directly)

### Distillation

1. User runs `/leiter-distill` (or says "distill" or similar — the agent auto-matches the skill)
2. Skill spawns a sub-agent to handle distillation (keeps session log output out of the main context)
3. Sub-agent runs `leiter soul distill`, reads the output, updates the soul with new learnings, and returns a concise
   summary of what it added, modified, or removed
4. After the sub-agent completes successfully, the main agent always runs `leiter soul mark-distilled` — even if the
   sub-agent found no new preferences to add. This advances `last_distilled` and commits the external Claude scan's
   `pending` watermarks so unchanged sessions are not re-processed; when experimental Codex support is enabled it also
   commits the Codex `pending` watermarks
5. Main agent relays the sub-agent's summary to the user so they can see what distillation changed

## Non-Goals (For Now)

- Multiple user profiles or project-specific souls
- Automatic distillation by default (opt-in via setup)
- Soul backup
- API key management or direct Claude API calls from the CLI
