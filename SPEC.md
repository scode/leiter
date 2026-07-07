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
│  Session start (both active this revision):                  │
│    <claude_home>/CLAUDE.md block ──► soul inlined            │
│    SessionStart hook ──► leiter hook context ──► soul +      │
│                          leiter hook nudge      instructions │
│    Soul is seen twice until the hooks are dismantled.        │
│                                                              │
│  ... normal session ...                                      │
│                                                              │
│  "instill X" ──► leiter skill ──► leiter soul instill        │
│                  ──► agent edits soul.md ──► leiter sync     │
│                                                              │
│  "distill" ──► leiter skill                                  │
│                  ──► sub-agent: leiter soul distill          │
│                  ──► sub-agent edits soul.md                 │
│                  ──► agent: leiter soul mark-distilled       │
│                                                              │
│  "show soul" ──► leiter skill ──► leiter soul show           │
│                  ──► agent displays verbatim                 │
│                                                              │
│  "upgrade soul" ──► leiter skill ──► leiter soul upgrade     │
│                  ──► agent restructures soul.md              │
│                  ──► agent: leiter soul mark-upgraded        │
│                                                              │
│  SessionEnd hook ──► leiter hook session-end                 │
│                      ──► copies transcript to logs/          │
└──────────────────────────────────────────────────────────────┘

~/.leiter/
├── leiter.toml          # Main leiter settings
├── soul.md              # The "leiter soul" — agent instructions (pure markdown, no frontmatter)
├── state.toml           # Leiter-managed metadata (epochs, watermarks, sync hashes); agent never edits
└── logs/
    ├── 20260223T173000Z-abc123.jsonl
    ├── 20260223T190000Z-def456.jsonl
    └── ...

~/.claude/
├── CLAUDE.md               # Carries the managed soul block (SCODE_LEITER_BEGIN/END)
└── skills/
    └── leiter/SKILL.md     # Contains <!-- SCODE_LEITER_INSTALLED -->

~/.codex/                   # Only present when codex = true
└── AGENTS.md               # Carries the managed soul block (SCODE_LEITER_BEGIN/END)
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

The Claude Code home directory is where leiter installs its plugin file (the single skill) and the managed soul block in
`CLAUDE.md`. The default is `~/.claude/`. The `leiter claude` subcommand accepts a `--claude-home <path>` flag to
override the directory, primarily for testing. It also accepts `--codex-home <path>`, used only when `codex = true` and
the command re-syncs the Codex `AGENTS.md` block (see `leiter claude install`); both flags exist to keep install and
uninstall drivable against fixture directories in tests.

### Codex Home Directory

The Codex home directory is where leiter writes the managed soul block for Codex (`<codex_home>/AGENTS.md`). The default
is `~/.codex/`. The `leiter codex` subcommand accepts a `--codex-home <path>` flag to override it, mirroring
`--claude-home` on the `leiter claude` subcommand and primarily for testing. This is a distinct flag from the
`--codex-home` on `leiter soul distill`, which is unrelated plumbing scoped to Codex session scanning.

### Plugin Files

`leiter claude install` writes a single skill file into the Claude Code home directory:

- **`<claude_home>/skills/leiter/SKILL.md`** — the one consolidated leiter skill. It exists purely for auto-matching
  convenience; the underlying mechanism is always the CLI. Its description carries the trigger keywords (remember,
  learn, instill, always, never, distill, soul, upgrade) so Claude routes matching requests to it. Its body routes by
  intent: instill → `leiter soul instill`; distill → the existing sub-agent flow (`leiter soul distill` in a sub-agent,
  then `leiter soul mark-distilled` — a later revision repoints this at a standalone distill command); show →
  `leiter soul
  show`, displayed verbatim in a fenced code block (keeping the existing fence-length instruction so
  backticks in the soul cannot break out of the fence); upgrade → `leiter soul upgrade` then
  `leiter soul mark-upgraded`; and after any direct edit of the soul file → `leiter sync`, so the managed blocks pick up
  the change.

The skill file contains the sentinel string `SCODE_LEITER_INSTALLED` as an HTML comment. `leiter claude uninstall`
checks for this sentinel to verify that leiter was installed before removing files.

Transitional note: this one skill replaces the previous six `leiter-*` skills. The `/leiter-setup` and
`/leiter-teardown` skills in particular are gone from fresh installs, but the commands they used to call —
`leiter claude agent-setup-instructions` and `leiter claude agent-teardown-instructions` — still exist and can be run
directly. They (and the hooks they configure) are dismantled in a later revision. `leiter claude install` removes any of
the old `leiter-*` skill directories whose `SKILL.md` still carries the sentinel, so upgrading a box collapses the old
set down to the one skill.

The skill file is a `const &str` template built into the binary. It is written to disk by `leiter claude install` and
overwritten on re-run (idempotent).

### Managed soul-delivery blocks

Both harnesses receive the soul through a managed block — a span delimited by the sentinel comments
`<!-- SCODE_LEITER_BEGIN -->` and `<!-- SCODE_LEITER_END -->` — written into a file the harness already reads at session
start. For Claude that file is `<claude_home>/CLAUDE.md`, and the block is written on every `leiter claude install` and
every `leiter sync`, unconditionally. For Codex it is `<codex_home>/AGENTS.md`, written only when `codex = true` in
`leiter.toml`.

The block opens with a short preamble and then inlines the soul body verbatim. The preamble states leiter's identity (a
self-training system that learns across sessions), the resolved soul path (the state directory joined with `soul.md`),
and how the agent should act on the user's language: instill on "remember"/"learn"/"always"/"never" (or similar) by
running `leiter soul instill`; distill on request via the distillation flow; show the soul on request via
`leiter soul show`; upgrade on request via `leiter soul upgrade`; and — the point that matters most for a delivery model
built on a materialized copy — run `leiter sync` after any direct edit of the soul file, so the managed blocks are
brought back in line. The soul body follows the preamble inside a backtick code fence whose length is computed at write
time: at least three backticks, and always strictly longer than the longest run of consecutive backticks anywhere in the
soul body, so a soul that itself contains fenced code blocks cannot terminate the fence early.

Why the soul is inlined rather than imported. Claude Code supports `@path` imports in `CLAUDE.md`, which would be a
tempting way to deliver an always-current soul without materializing a copy. That was rejected on evidence, not
preference. Imports in `CLAUDE.md` resolve, and they nest — an import inside an imported file also resolves (verified
2026-07-07 against a claude 2.1.20x-era CLI). The soul is agent-writable, so an import-delivered soul would let anything
that can write `soul.md` pull arbitrary readable files into every session by planting an `@import` of them. Imports
inside a code fence, by contrast, were verified inert. That is exactly what makes the inlined fenced copy safe, and it
has a second benefit: incidental `@`-tokens in soul prose (decorators, email addresses) inside the fence are never
interpreted as imports either. Codex has no import syntax at all, so inlining is the only option there regardless — and
using one delivery model for both harnesses keeps the sync and staleness handling uniform.

The sentinel comment text must say, in the file itself, that the block is machine-managed by leiter and that edits
belong in the soul file at its resolved path — not between the sentinels. A human opening `CLAUDE.md` or `AGENTS.md` is
thereby told where to make changes and that hand edits inside the block will be reported and refused by `leiter sync`
(see below).

Writer semantics. The block writer targets exactly one file and behaves by what it finds:

- Target file absent → create it containing only the block.
- Present with neither sentinel → append the block, preserving all existing content byte-for-byte.
- Present with both sentinels → replace only the span between (and including) them, leaving everything outside the span
  byte-for-byte untouched.
- Present with a `BEGIN` sentinel but no matching `END` → error and refuse to touch the file. The managed span cannot be
  located unambiguously, and guessing risks corrupting user content.

Every block write is atomic: leiter writes a temporary file in the same directory and renames it over the target, so a
crash mid-write never leaves a torn `CLAUDE.md` or `AGENTS.md`.

Symlinked targets are followed, not replaced. Dotfiles-managed setups routinely make `CLAUDE.md` or `AGENTS.md` a
symlink into a config repo; a naive rename-over would swap the symlink for a regular file and silently disconnect that
setup. When the target exists, leiter canonicalizes it and performs the temp-file-plus-rename in the resolved file's own
directory. A dangling symlink (target path resolves to nothing) is an error, not a create — writing "through" a broken
link cannot match the user's intent.

Leiter records two SHA-256 hashes per target in `state.toml` (see `~/.leiter/state.toml`): the soul body it last
materialized into that block (for staleness detection) and the full block content it last wrote (for clobber detection).
These drive `leiter sync` and the opportunistic re-sync described next.

#### Opportunistic re-sync

`leiter sync` is the explicit way to re-materialize the blocks, but the agent typically edits the soul _after_ leiter's
involvement in a command has ended, so there is a structural gap: the block on disk drifts from the soul between syncs.
To heal that gap, every state-mutating leiter command that runs to completion also re-materializes any managed block
whose recorded soul hash no longer matches the current soul body — the same work `leiter sync` does, run as a side
effect. The commands that do this are `leiter soul mark-distilled`, `leiter soul mark-upgraded`, `leiter config set`,
`leiter claude install`, `leiter codex install`, and the pending-watermark staging path of `leiter soul distill`.

The same clobber guard applies: a block hand-edited since leiter last wrote it is warned about and left alone, never
silently clobbered. Here a refusal is only a warning — it never turns the host command into a failure. A
`mark-distilled` that cannot heal a hand-edited `CLAUDE.md` still succeeds at marking distilled; the user is simply told
to run `leiter sync --force`.

Read-only surfaces never write blocks. `leiter soul show` and any `--dry-run` path report or display without healing, so
their output reflects on-disk state rather than a state they silently repaired. The explicit `leiter sync` step that the
`leiter soul instill` guidelines end with (see that command) is the primary close of the gap; opportunistic re-sync is
the backstop for every path that does not run it.

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
codex = false
```

`codex` defaults to `false` when the file is missing. When true, Codex support is active: it gates the Codex portions of
`leiter soul distill` and `leiter soul mark-distilled` and the delivery of the `AGENTS.md` managed block.
`leiter codex
install` is the command that flips it on. When false, `leiter soul distill` and
`leiter soul mark-distilled` must not read Codex rollout files, must not consult the `[codex.*]` tables in
`~/.leiter/state.toml`, and must not modify those tables' contents. Since `state.toml` is core state (not
Codex-specific), commands still load and rewrite the file as a whole — the requirement is that a disabled gate leaves
the `[codex.*]` table contents exactly as they were, and epoch validation and `last_distilled` work unaffected. This
gate is Codex-only: the external Claude session scan and its `[claude.*]` tables are always active and are never gated
on this flag.

The key `codex` replaces the older `enable_codex_experimental` with identical gating semantics. For backward
compatibility, loading still accepts `enable_codex_experimental` as an alias (same meaning); saving always writes the
new `codex` key, so a config loaded with the legacy name is rewritten to `codex` the next time leiter persists it.

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

[sync.claude_md]
soul_hash = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
block_hash = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"

[sync.agents_md]
# same shape; present only when codex = true and the AGENTS.md block has been synced

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
- `[sync.claude_md]` and `[sync.agents_md]` (optional): per-target sync tracking for the managed soul-delivery blocks
  (see Managed soul-delivery blocks). Each table holds two SHA-256 hex digests. `soul_hash` is the soul body last
  materialized into that block — comparing it against the current soul body is how leiter detects a stale block.
  `block_hash` is the full block content leiter last wrote to that target — comparing it against the block currently on
  disk is how leiter detects a hand edit inside the managed span (the clobber guard). An absent table means that target
  has never been synced. A `[sync.agents_md]` table may linger after Codex is disabled — uninstall deliberately leaves
  `state.toml` alone — which is harmless: a target whose file lacks the block is recreated without `--force` on the next
  sync, regardless of stale hashes.

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
3. If the block-relevant soul state is stale, opportunistically re-sync the managed blocks (see Opportunistic re-sync)
4. Persist the updated config back to `~/.leiter/leiter.toml`

**Supported keys:**

- `codex`: boolean (`true` or `false`). The legacy key name `enable_codex_experimental` is also accepted and is written
  back as `codex`; when it is used, the confirmation output includes a note that the name is deprecated.

**Output (stdout):** A confirmation message of the form `<key> set to <value>`.

**Errors:** Unknown keys and invalid values exit with a non-zero code.

### `leiter claude install`

First-time setup. Performs deterministic initialization of the state directory, writes the plugin file (the single
skill) to the Claude Code home directory, and writes the managed soul-delivery blocks.

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
5. Write the single `<claude_home>/skills/leiter/SKILL.md` skill file, overwriting on re-run (idempotent). Remove any of
   the old `<claude_home>/skills/leiter-*/` directories whose `SKILL.md` carries the `SCODE_LEITER_INSTALLED` sentinel —
   this collapses a previous six-skill install down to the one skill and is safe to re-run
6. Write the managed soul-delivery block into `<claude_home>/CLAUDE.md` (always), and into `<codex_home>/AGENTS.md` when
   `codex = true` — effectively a `leiter sync` of both targets, using the block writer and clobber-guard semantics from
   Managed soul-delivery blocks. This runs after the state/soul convergence so the block reflects the just-materialized
   soul, and it records the `[sync.*]` hashes. A fresh install has no recorded block hash, so both targets are written
   unconditionally

**Output (stdout):** A success message confirming what was written — the `CLAUDE.md` managed soul block (and the
`AGENTS.md` block when `codex = true`) and the single `leiter` skill — and noting that the soul is now delivered inline
via the managed block, so soul injection no longer depends on a hook. Because the `/leiter-setup` skill is gone, the
message points users who still want the session-logging and nudge hooks at running
`leiter claude
agent-setup-instructions` directly; those hooks remain available in this revision (the external Claude
scan already reads transcripts directly, so they are no longer required for distillation to see sessions).

If any step fails, the output instructs the agent to relay the error to the user.

### `leiter claude uninstall`

Removes leiter's skill and the managed soul block from the Claude Code home directory. Does NOT touch `~/.leiter/`
(soul, logs, and state) or `~/.claude/settings.json` (hooks are removed via the `agent-teardown-instructions` subcommand
or manually). It also leaves the Codex `AGENTS.md` block alone — that is removed by `leiter codex uninstall`.

**Behavior:**

1. Remove the managed soul-delivery block from `<claude_home>/CLAUDE.md`, preserving all other file content
   byte-for-byte. The file is left in place without the block, not deleted. A missing `CLAUDE.md`, or a `CLAUDE.md` with
   no managed block, is a no-op for this step
2. Scan skill directories under `<claude_home>/skills/` for a `SKILL.md` containing `SCODE_LEITER_INSTALLED` (the one
   `leiter/` skill, plus any stale old `leiter-*` skills) and remove them (best-effort, skip missing)
3. If nothing leiter-owned was found at all — no sentinel-bearing skill and no managed block to remove — error

Block removal runs before the skill check on purpose: a partial uninstall (skills gone, block left behind by an earlier
failure) must be completable by re-running the command, and a skills-first sentinel bail would make the block removal
permanently unreachable on retry.

The recorded `[sync.claude_md]` hashes in `state.toml` are deliberately left as-is (uninstall does not touch
`~/.leiter/`). This is harmless: with the block gone from disk, a later `leiter claude install` finds no managed block
and recreates it without needing `--force`.

**Output (stdout):** A success message with guidance on how to remove hooks, fully clean up (`~/.leiter/`), and
re-enable later.

**Errors:** Exit non-zero only when there was nothing leiter-owned to remove (neither sentinel-bearing skills nor a
managed block).

### `leiter codex install`

Enables Codex support and delivers the soul to it. This is the one command that flips Codex on.

**Flags:**

- `--codex-home <path>`: override the Codex home directory (default `~/.codex/`), primarily for testing. Mirrors
  `--claude-home` on the `leiter claude` subcommand.

**Behavior:**

1. Validate state (see Setup Epochs). If incompatible, exit with an error
2. Set `codex = true` in `~/.leiter/leiter.toml` (creating the file if absent, preserving other keys)
3. Write the managed soul-delivery block into `<codex_home>/AGENTS.md` using the block writer semantics from Managed
   soul-delivery blocks, and record the `[sync.agents_md]` hashes
4. Because this is a state-mutating command, opportunistically re-sync the `CLAUDE.md` block if it has gone stale (see
   Opportunistic re-sync)

**Output (stdout):** A confirmation that Codex is enabled and the `AGENTS.md` block was written.

### `leiter codex uninstall`

Removes the Codex managed soul block and disables Codex support. It must work regardless of the current config value — a
user who set `codex = false` first must still be able to clean up the block — so it never gates on the flag.

**Flags:**

- `--codex-home <path>`: as above.

**Behavior:**

1. Validate state (see Setup Epochs). If incompatible, exit with an error
2. Remove the managed block from `<codex_home>/AGENTS.md`, preserving all other file content byte-for-byte. A missing
   `AGENTS.md`, or a present file with no managed block, is a no-op success — there is nothing to remove
3. Set `codex = false` in `~/.leiter/leiter.toml`

As with `leiter claude uninstall`, the recorded `[sync.agents_md]` hashes are left as-is; they are harmless once the
block is gone, and a later `leiter codex install` recreates the block without `--force`.

**Output (stdout):** A confirmation that Codex is disabled and the `AGENTS.md` block was removed (or that there was
nothing to remove).

### `leiter sync`

Re-materializes the managed soul-delivery blocks from the current soul, bringing each target back in line with
`~/.leiter/soul.md`. This is the command the agent (or a human) runs after editing the soul directly, and the primary
way the `CLAUDE.md` and `AGENTS.md` copies stay current.

**Flags:**

- `--force`: overwrite a block that has been hand-edited since leiter last wrote it (see clobber guard).

**Behavior:**

1. Validate state (see Setup Epochs). If incompatible, exit with an error
2. Determine the target set: always `CLAUDE.md`; additionally `AGENTS.md` when `codex = true` in `leiter.toml`
3. For each target, reconcile the on-disk block against the recorded `[sync.*]` hashes and the current soul body (see
   the outcomes below), writing the block where needed
4. After the block writes succeed, update the `[sync.*]` hashes in `state.toml` in a single atomic write. Writing the
   blocks first and committing the hashes only on success keeps the recorded state from ever getting ahead of reality

**Clobber guard and per-target outcomes.** For each target, leiter compares the block currently on disk against that
target's recorded `block_hash`, and the recorded `soul_hash` against the current soul body:

- On-disk block matches `block_hash` and `soul_hash` equals the current soul body → **already current**; nothing is
  written.
- On-disk block matches `block_hash` but the soul body has changed → stale copy; re-materialize the block and record the
  new hashes → **synced**.
- The target file has no managed block at all → (re)create it and record the hashes → **synced**. Nothing user-authored
  is at risk, so this needs no `--force`.
- A block is present but does not match `block_hash` → someone edited inside the managed span. Leiter warns, naming the
  target, and **refuses** that target unless `--force` is passed (which re-materializes and re-records). One hand-edited
  block does not block the others — every other target still syncs.

**Output (stdout):** One line per target reporting its outcome — synced, already current, or refused (hand-edited, rerun
with `--force`).

### `leiter claude agent-setup-instructions`

Outputs natural language instructions for the agent to configure Claude Code hooks in `~/.claude/settings.json`. This is
the same hook configuration content that `leiter claude install` used to output directly.

Transitional note: this command used to be invoked by the `/leiter-setup` skill, which no longer exists (the six skills
are consolidated into one — see Plugin Files). The command itself stays runnable directly, so a user who wants the
session-logging and nudge hooks can still get these instructions. It is retired along with the hooks in a later
revision.

**Behavior:** Validates state (see Setup Epochs). If incompatible, exits with an error.

**Output (stdout):** Instructions including the exact JSON hook entries for `SessionStart` and `SessionEnd`, plus
three-case logic for handling fresh install, upgrade, and already-configured states. See Hook Configuration below for
the exact hook JSON. After hooks are configured, includes an optional permissions prompt (see Permissions below).

### `leiter claude agent-teardown-instructions`

Outputs natural language instructions for the agent to remove leiter hooks from `~/.claude/settings.json`.

Transitional note: this command used to be invoked by the `/leiter-teardown` skill, which no longer exists. Like
`agent-setup-instructions`, the command stays runnable directly until the hooks are dismantled in a later revision.

**Behavior:** Validates state (see Setup Epochs). If incompatible, exits with an error.

**Output (stdout):** Instructions telling the agent to find and remove hook entries whose commands contain
`"leiter hook context"`, `"leiter hook nudge"`, or `"leiter hook session-end"`, clean up empty arrays, preserve
non-leiter hooks, remove leiter permission entries (see Permissions below), and provide cleanup/re-enable guidance to
the user.

### `leiter hook context`

Outputs the soul content and agent instructions. Called by the SessionStart hook.

Transitional note: as of this revision the soul is also delivered by the managed `CLAUDE.md` block (see Managed
soul-delivery blocks), so on a box that still has the hook configured the agent sees the soul twice at session start —
harmless duplication that resolves when the hook is dismantled in a later revision. This preamble also still names the
previous per-topic skills (`/leiter-instill`, `/leiter-distill`, `/leiter-soul`, `/leiter-soul-upgrade`), which have
been consolidated into the single `leiter` skill; the consolidated skill auto-matches the same trigger keywords, and the
managed block carries the current routing guidance. The mismatch is cosmetic and is retired with the hook.

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
   `--claude-home`). This scan is unconditional — it is **not** gated on `codex` (that gate is Codex-only). Discovery is
   recursive but fail-useful about Claude Code's undocumented layout: only regular files (symlinks are never followed),
   only `.jsonl` files whose filename stem parses as a UUID (that stem is the session id), everything else silently
   skipped; unreadable files or directories are warned about and skipped, never fatal. See Claude session scanning below
   for the full contract
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
8. If `codex = true`, best-effort scan Codex rollout transcripts under `<codex_home>/sessions/**/*.jsonl` and
   `<codex_home>/archived_sessions/**/*.jsonl` (default `~/.codex/`, overridable with `--codex-home`)
9. If `codex = true`, for each Codex rollout file, read the leading `session_meta` record and use `payload.id` as the
   stable session ID. Files without a readable leading `session_meta` record are skipped with a warning
10. If `codex = true`, read the Codex `[codex.committed]` watermarks from the validated `state.toml`. (Unlike the former
    `codex-meta.toml`, `state.toml` is core state validated in step 1, so an unreadable or invalid state file is already
    a hard command error there — there is no separate warn-and-skip path for it. The warn-and-default behavior for
    `leiter.toml` in step 4 is unchanged.)
11. If `codex = true`, for each Codex session ID, compare the current file watermark (`path`, `size_bytes`, `mtime_utc`)
    to the `[codex.committed]` watermark in `state.toml`. If unchanged, skip the session completely. If changed (or
    new), re-read the full rollout file and emit the full canonicalized session so the LLM sees the entire updated
    context
12. Sort the combined Claude output chronologically, interleaving legacy `~/.leiter/logs/` sessions (keyed by filename
    timestamp, as today) with external Claude sessions (keyed by their session timestamp — see Claude session scanning
    for how that timestamp is derived — then session id as a tiebreak). Sort changed Codex sessions separately by
    session timestamp (from `session_meta.payload.timestamp`) and then session ID
13. Output the Claude transcript content (legacy and external, interleaved), and also Codex transcript content when
    enabled, wrapped in XML-like boundary tags (see Output below)
14. If `--dry-run` is not set, replace the `[claude.pending]` map in `~/.leiter/state.toml` with this run's changed
    Claude sessions and set `pending_scan_started_utc` to the time this run's scan began (an atomic rewrite preserving
    all other state fields). This staging is **not** gated on `codex`, and it happens even when the Claude home could
    not be resolved and the scan was skipped — pending means "exactly what this run showed the LLM", and a run that
    scanned nothing showed nothing; leaving a stale pending map would let the next `mark-distilled` commit watermarks
    for sessions this cycle never emitted. If writing state fails, warn and continue. Staging may happen before the
    emission step completes; this is safe because `mark-distilled` is only ever run after a distill that succeeded end
    to end (a failed distill is rerun, restaging from current reality)
15. If `codex = true` and `--dry-run` is not set, replace the `[codex.pending]` map in `~/.leiter/state.toml` with the
    changed sessions from this run (an atomic rewrite preserving all other state fields). If writing state fails, warn
    and continue
16. On the non-dry-run staging path only, opportunistically re-sync any stale managed block (see Opportunistic re-sync).
    A `--dry-run` distill writes no state and must not touch the blocks. Hand-edited blocks are warned about, never
    clobbered, and a refusal does not fail the command

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
external scan is always active in this revision; it is not gated on `codex`.

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
unexpected. When `codex = false`, the command must not read Codex rollout files and must not read or modify the contents
of the `[codex.*]` tables in `~/.leiter/state.toml` (it still loads and rewrites the file as a whole; the disabled gate
just passes those tables through unchanged).

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
   **not** gated on `codex`, since the external Claude scan is always active
4. Set `last_distilled` to the staged `pending_scan_started_utc` (clearing that field), falling back to the current UTC
   time when nothing is staged (a mark without a preceding non-dry-run distill). If `codex = true`, also merge the
   `[codex.pending]` map into `[codex.committed]` and clear `[codex.pending]`
5. Write `state.toml` back in a single atomic write, preserving all other fields. There is no separate best-effort path
   for either merge: both the Claude and (when enabled) Codex promotions ride the same write as `last_distilled`, so the
   whole thing either commits or the command fails. (The old warn-and-continue behavior existed because Codex watermarks
   lived in a separate best-effort file; that split no longer exists.)
6. On successful commit, opportunistically re-sync any stale managed block (see Opportunistic re-sync). A block that was
   hand-edited is warned about, never clobbered, and a refusal here does not fail the command

When `codex = false`, `leiter soul mark-distilled` must not consult or modify the contents of the `[codex.*]` tables in
`~/.leiter/state.toml` — the rewrite that updates `last_distilled` passes them through unchanged.

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
4. A final instruction to run `leiter sync` after editing, so the managed soul-delivery blocks pick up the change. This
   step is the primary close of the structural gap where the agent edits the soul after leiter's involvement ended (see
   Opportunistic re-sync); the guidelines end with it

See the Architecture section for why guidelines are shared between `instill` and `distill`.

### `leiter soul show`

Outputs the full soul file wrapped in XML boundary tags for safe verbatim display. Invoked through the consolidated
`leiter` skill (its show route) when the user asks to see their soul.

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
  distillation (instead of asking the user). This is opt-in via `leiter claude agent-setup-instructions` option 3.

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
Invoked through the consolidated `leiter` skill (its upgrade route), or directly by the agent when the user asks to
upgrade the soul using natural language.

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
3. On success, opportunistically re-sync any stale managed block (see Opportunistic re-sync) — an upgrade restructures
   the soul, so the delivered copies would otherwise be stale until the next command. Hand-edited blocks are warned
   about, never clobbered, and a refusal does not fail the command

**Output (stdout):** A confirmation message including the version that was set.

## Hook Configuration

The following hooks are configured in `~/.claude/settings.json` by the agent when it runs
`leiter claude agent-setup-instructions` (formerly triggered by the `/leiter-setup` skill, which no longer exists — see
Plugin Files). In this revision hooks are optional: the soul reaches the agent through the managed `CLAUDE.md` block
regardless, and the external Claude scan reads transcripts directly, so these hooks add session-end archiving and
distillation nudges rather than being required for leiter to function. They are dismantled entirely in a later revision.

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
into auto-distillation during `leiter claude agent-setup-instructions` (option 3), the nudge command is configured as
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
3. The command creates the `~/.leiter/` structure, writes the single `leiter` skill to `~/.claude/skills/`, and writes
   the managed soul block into `~/.claude/CLAUDE.md` (and `~/.codex/AGENTS.md` when `codex = true`)
4. On the next session start, the harness reads the managed block, so the agent has the soul and leiter instructions
   with no hook involved
5. Optionally, to capture session transcripts through the SessionEnd hook and receive distillation nudges, the user has
   the agent run `leiter claude agent-setup-instructions` and configure hooks in `~/.claude/settings.json` with the
   user's approval. The `/leiter-setup` skill that used to drive this is gone in this revision, but the command remains
6. Agent presents optional features (Bash permissions, soul file access, auto-distillation); user accepts any
   combination or none

### Normal Session (After Setup)

1. Session starts → the agent has the soul from the managed `CLAUDE.md` block. If hooks are configured, the SessionStart
   hook also fires → `leiter hook context` outputs soul + instructions (a harmless second copy this revision) and
   `leiter hook nudge` outputs a distillation reminder if stale logs exist
2. Normal session proceeds
3. Session ends → if the SessionEnd hook is configured, `leiter hook session-end` copies the transcript to
   `~/.leiter/logs/`. Either way, `leiter soul distill`'s external scan reads the live transcript from
   `~/.claude/projects/` directly

### User Asks the Agent to Learn Something

1. User says "instill", "remember", "always", "never", etc. — the agent auto-matches the consolidated `leiter` skill
2. The skill runs `leiter soul instill "always use snake_case for Rust functions"`
3. Agent receives writing guidelines and the quoted preference
4. Agent reads `~/.leiter/soul.md`, edits the appropriate section following the guidelines, then runs `leiter sync` so
   the managed blocks pick up the new preference
5. Preference is active in all future sessions, delivered via the managed block

### Soul Upgrade

1. User updates `leiter` binary to a newer version
2. User says "upgrade the leiter soul" — the agent auto-matches the consolidated `leiter` skill (its upgrade route)
3. The skill runs `leiter soul upgrade`
4. If already current: agent relays that no upgrade is needed
5. If outdated: agent receives the upgrade instructions and new template
6. Agent reads current `~/.leiter/soul.md`, restructures it into the new format, then runs `leiter soul mark-upgraded`
   to record the new `soul_version` in `state.toml` (the agent never edits version metadata directly). `mark-upgraded`
   opportunistically re-syncs the managed blocks so the restructured soul is delivered right away

### Distillation

1. User says "distill" or similar — the agent auto-matches the consolidated `leiter` skill (its distill route)
2. The skill spawns a sub-agent to handle distillation (keeps session log output out of the main context)
3. Sub-agent runs `leiter soul distill`, reads the output, updates the soul with new learnings, and returns a concise
   summary of what it added, modified, or removed
4. After the sub-agent completes successfully, the main agent always runs `leiter soul mark-distilled` — even if the
   sub-agent found no new preferences to add. This advances `last_distilled` and commits the external Claude scan's
   `pending` watermarks so unchanged sessions are not re-processed; when Codex support is enabled it also commits the
   Codex `pending` watermarks. `mark-distilled` also opportunistically re-syncs the managed blocks, so a soul edited
   during distillation reaches future sessions
5. Main agent relays the sub-agent's summary to the user so they can see what distillation changed

## Non-Goals (For Now)

- Multiple user profiles or project-specific souls
- Automatic distillation by default (opt-in via setup)
- Soul backup
- API key management or direct Claude API calls from the CLI
