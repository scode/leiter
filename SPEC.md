# Leiter Spec

Leiter is a self-training system for Claude Code. Once enabled, Claude automatically learns user preferences, coding
practices, and workflow patterns across sessions. It works by logging session activity and periodically distilling those
logs into a persistent "soul" — a set of agent instructions that shape future agent behavior.

## Core Principle

The Claude agent does all the thinking. The `leiter` CLI is a thin helper that handles structured storage, timestamp
management, and context injection. The agent reads files, writes summaries, edits the soul, and decides what to
remember. Leiter never calls a model API directly. It does, however, orchestrate one: `leiter distill` invokes an agent
CLI (`claude -p`) as a subprocess to do the soul-editing thinking. That is an intentional relaxation of the older "the
CLI never touches a model" stance — leiter shells out to a harness that already owns authentication and model selection
rather than talking to any API itself.

## Architecture

Leiter runs hookless. The soul reaches each harness through a managed block that the harness already reads at session
start — `<claude_home>/CLAUDE.md` for Claude, `<codex_home>/AGENTS.md` for Codex — so no SessionStart hook is needed to
inject it. Session transcripts are read directly from the harnesses' own session stores (Claude Code's `projects/` tree
and the Codex rollout directories), so no SessionEnd hook is needed to capture them. Distillation runs through
`leiter distill`, which shells out to a headless agent CLI; the single consolidated `leiter` skill is a convenience
trigger that runs the same command.

Hooks survive only as a migration artifact. A box set up before 0.9.0 still carries `leiter hook` entries in
`~/.claude/settings.json`. The SessionStart hooks now run one-release tombstones whose only job on an incompatible box
is to deliver the migration instructions, while the SessionEnd hook keeps archiving transcripts until the tombstones are
deleted — a user who defers migration must lose nothing (see the `leiter hook *` commands and Migration from hook-based
setups); `leiter claude
install` performs the actual migration and emits the settings-cleanup instructions. The
tombstoned subcommands are all removed in the release after 0.9.0.

The `leiter soul instill` and `leiter soul distill` commands share a single set of soul-writing guidelines (built into
the binary). This ensures consistent entry quality across both learning paths — inline preferences and transcript
distillation — while keeping normal session context minimal. The guidelines only appear when the agent is actively
writing to the soul.

```
┌──────────────────────────────────────────────────────────────┐
│                       Claude Code Session                    │
│                                                              │
│  Session start:                                              │
│    <claude_home>/CLAUDE.md block ──► soul inlined            │
│    (no hook — the managed block is the sole delivery path)   │
│                                                              │
│  ... normal session ...                                      │
│                                                              │
│  "instill X" ──► leiter skill ──► leiter soul instill        │
│                  ──► agent edits soul.md ──► leiter sync     │
│                                                              │
│  "distill"/cron ──► leiter distill (headless)                │
│                  ──► agent edits soul.md from transcripts    │
│                  ──► leiter commits state + re-syncs blocks  │
│                  (skill triggers it; cron runs it unattended)│
│                                                              │
│  "show soul" ──► leiter skill ──► leiter soul show           │
│                  ──► agent displays verbatim                 │
│                                                              │
│  "upgrade soul" ──► leiter skill ──► leiter soul upgrade     │
│                  ──► agent restructures soul.md              │
│                  ──► agent: leiter soul mark-upgraded        │
│                                                              │
│  transcripts ──► read in place from <claude_home>/projects/  │
│                  by leiter distill (no SessionEnd hook)       │
│                                                              │
│  Migrating box only (pre-0.9.0 hooks still configured):      │
│    SessionStart ──► leiter hook context ──► migration note   │
│    SessionEnd   ──► leiter hook session-end ──► archives      │
│                     transcript (still works this release)     │
└──────────────────────────────────────────────────────────────┘

~/.leiter/
├── leiter.toml          # Main leiter settings
├── soul.md              # The "leiter soul" — agent instructions (pure markdown, no frontmatter)
├── state.toml           # Leiter-managed metadata (epochs, watermarks, sync hashes); agent never edits
└── logs/                # Legacy: hook-copied transcripts. Only present on pre-migration boxes;
    ├── 20260223T173000Z-abc123.jsonl   #   the first post-migration distill drains these and
    ├── 20260223T190000Z-def456.jsonl   #   removes the directory (see leiter distill).
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
  intent: instill → `leiter soul instill`; distill → `leiter distill` (a single headless flow — the skill runs the
  command directly, does not spawn a sub-agent, and never runs a mark step, since leiter commits its own state); show →
  `leiter soul
  show`, displayed verbatim in a fenced code block (keeping the existing fence-length instruction so
  backticks in the soul cannot break out of the fence); upgrade → `leiter soul upgrade` then
  `leiter soul mark-upgraded`; and after any direct edit of the soul file → `leiter sync`, so the managed blocks pick up
  the change.

The skill file contains the sentinel string `SCODE_LEITER_INSTALLED` as an HTML comment. `leiter claude uninstall`
checks for this sentinel to verify that leiter was installed before removing files.

Transitional note: this one skill replaces the previous six `leiter-*` skills. The old `/leiter-setup` and
`/leiter-teardown` skills that invoked `leiter claude agent-setup-instructions` and
`leiter claude agent-teardown-instructions` are gone. Both commands survive one more release as tombstones so those old
skill invocations (and any muscle-memory direct runs) do not hit a clap unknown-command error:
`agent-setup-instructions` no longer configures any hooks — it just reports that hooks are retired — while
`agent-teardown-instructions` stays functional and still emits hook-removal instructions, which is exactly what a
migrating box needs and what `leiter claude install`'s migration output points at. Both are removed in the release after
0.9.0, together with the `hook *` subcommands. `leiter claude install` removes any of the old `leiter-*` skill
directories whose `SKILL.md` still carries the sentinel, so upgrading a box collapses the old set down to the one skill.

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
running `leiter soul instill`; distill on request by running `leiter distill`; show the soul on request via
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
To heal that gap, an enumerated set of state-mutating leiter commands re-materialize, on successful completion, any
managed block whose recorded soul hash no longer matches the current soul body — the same work `leiter sync` does, run
as a side effect. The participating commands are exactly: `leiter soul mark-distilled`, `leiter soul mark-upgraded`,
`leiter config set`, `leiter codex install`, the pending-watermark staging paths of `leiter soul distill` and
`leiter distill`, and `leiter claude install` (which syncs with its own force semantics — see that command). The
uninstall commands mutate state but deliberately do not participate: they are removal flows, and healing a block on the
way out would be the opposite of what was asked.

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
`leiter claude install` to migrate an old integration model to a new one. Setup epochs detect this condition and alert
the user.

There are two independent epochs, each a monotonic integer starting at 1:

- **`setup_soft_epoch`**: Bumped when a leiter upgrade introduces changes that benefit from user action but are not
  strictly required. A mismatch produces a nudge but does not block the session.
- **`setup_hard_epoch`**: Bumped when a leiter upgrade introduces changes that require user action before the session
  can function correctly. A mismatch blocks every leiter command until install is re-run. It does not — cannot — block
  soul delivery on a hookless box: the managed `CLAUDE.md` block is static content the harness reads with no leiter
  involvement, so a hard-mismatched box keeps its (possibly stale) soul. Only the migrating-box `hook context` tombstone
  additionally declines to inject.

As of 0.9.0 the binary expects `setup_hard_epoch = 2` and `setup_soft_epoch = 2`. The hard epoch's `1 → 2` bump is the
hookless-migration trigger: a box set up before 0.9.0 still records `setup_hard_epoch = 1` (or has no `state.toml` at
all — see the legacy layout below), so the moment its upgraded binary runs any validating command — including the
`leiter hook context` tombstone fired by its still-configured SessionStart hook — the exact-equality check fails and the
box is routed to migration (see the `leiter hook *` tombstones, `leiter claude install`, and Migration from hook-based
setups). Nothing about the 0.9.0 bump is special-cased; the same epoch machinery that has always blocked on a hard
mismatch is exactly what serves it. A compiled-in guard test pins the expected epoch constants, so bumping either value
is a deliberate edit that must land in the same commit as the migration logic the bump implies — never an accidental
drift.

The binary has compiled-in expected values for both epochs. Epoch values are stored in `~/.leiter/state.toml`. Every
command except `session-end` (and the inert `hook nudge` tombstone, which reads nothing at all) validates the state
file's epoch values against the binary's expected values before doing any work. Hard epoch checks use exact equality —
both older and newer state files are flagged. Soft epoch mismatches in either direction are tolerated: commands proceed
normally, and the mismatch surfaces as an advisory line in `leiter status` (with hooks retired there is no session-start
channel to nudge through; status is where a user checks leiter's health). `leiter claude install` additionally migrates
a behind-the-binary soft epoch forward on re-run, and refuses to run when the state file is ahead of the binary (to
avoid downgrading). This validation is implemented as a single shared function used by all commands, preventing drift
between individual command implementations.

Either epoch field defaults to 1 when absent from `state.toml` (for backward compatibility with state files written
before that field existed).

A missing `state.toml` is no longer uniformly "not initialized". Validation discriminates on the soul. When `state.toml`
is missing but `soul.md` exists and still parses as YAML frontmatter, that is the pre-0.9.0 **legacy layout** — a
distinct incompatibility with its own migration message (see Epoch Error Messages). When `state.toml` is missing and
there is no frontmatter-bearing soul — no soul at all, or a soul that is already pure content — leiter is genuinely not
initialized, the same condition as a missing soul, with the same "run `leiter claude install`" response. Both cases
funnel the box to `leiter claude install`; only the message the agent relays differs. Because the discrimination lives
in the one shared validation function, every command that validates state routes a legacy box to install — including old
skills like `/leiter-distill`, whose route runs `leiter soul distill` and now surfaces the legacy-layout message instead
of half-running.

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
resumed half-migration. `leiter claude install` runs it in the legacy quadrant of its state/soul convergence — the case
where `soul.md` carries frontmatter but no `state.toml` exists yet (see `leiter claude install`). The migrated
`last_distilled` carries over verbatim from the old frontmatter: it becomes the floor for the external Claude scan (see
Claude session scanning), so a migrated box does not re-ingest the history the soul already learned — which is precisely
the floor's purpose. The setup epochs are the one thing that does **not** carry over: the routine itself stamps the
binary's current epoch values into the `state.toml` it writes, because running install is exactly the action the
hard-epoch bump requires — a migrated box that kept `setup_hard_epoch = 1` would re-trip the migration message on its
next session forever. The stamp rides the routine's atomic state write rather than being an install afterstep on
purpose: a crash between "state written with old epochs" and "epochs fixed" would leave a hard mismatch that install's
own epoch verification then refuses, a dead end no rerun escapes.

#### Epoch Error Messages Delivered to the User

When a validating command detects an incompatibility, the output is an instruction to the agent. (On a migrating box the
`leiter hook context` tombstone usually delivers it first, since its still-configured SessionStart hook fires before the
agent does anything else; `leiter hook nudge` no longer emits anything at all — see that command.) The agent must
deliver the quoted message to the user **verbatim** — the instruction must use strong compliance language (e.g. "EXACTLY
this (word for word)") to maximize the chance the agent relays it unchanged. The exact user-facing phrases for each
case:

- **Legacy layout** (`state.toml` missing while `soul.md` exists and still parses as YAML frontmatter — the pre-0.9.0
  layout that predates `state.toml`): "Leiter has moved to hookless operation: it no longer uses hooks, and its six
  skills are replaced by a single consolidated skill. Please run `leiter claude install` in your terminal — or let me
  run it now — and follow its instructions, then start a new session." Unlike the other cases, this instruction tells
  the agent it MAY run `leiter claude install` itself with the user's approval and then follow that command's output; it
  must not attempt any other leiter command first.
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

The instruction must also tell the agent not to attempt leiter commands for the remainder of the session — with one
carve-out: the Legacy layout message explicitly permits exactly `leiter claude install` (with the user's approval),
since running it is the migration. Its prohibition is on any _other_ leiter command.

- **Not initialized** (no `state.toml` and no soul, or a soul that is already pure content): "Leiter is not initialized.
  Run `leiter claude install` to set up." This one is not an ACTION REQUIRED relay — there is nothing to migrate and no
  urgency to escalate; it is a plain pointer.

Each incompatibility also carries a shorter non-verbatim `user_message` form used when a directly-invoked CLI command
fails validation (there is no agent to relay through); both forms funnel to the same remedy.

Soft epoch mismatches produce no message from validating commands at all; the advisory lives in `leiter status` (see
Setup Epochs and that command).

One unsupported shape, for completeness: a `state.toml` recording `setup_hard_epoch = 1` can only come from an interim
main-branch build (released 0.8.1 predates `state.toml` entirely), and no code path advances it. Such a box is outside
the migration contract; the recovery is deleting `state.toml` and re-running install, accepting the watermark reset.

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
retention_warn_days = 21
# agent_command is unset by default; when set it is an array of strings, e.g.
# agent_command = ["claude", "-p", "--model", "opus", "--allowedTools", "Read(~/.leiter/soul.md),Edit(~/.leiter/soul.md),Write(~/.leiter/soul.md)"]
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

`retention_warn_days` defaults to `21` when absent. It sets the age threshold, in days, for the retention warning
`leiter status` raises against undistilled external Claude sessions. The default sits deliberately below Claude Code's
roughly 30-day session-store pruning window, so the warning gives lead time to distill a session before Claude Code
deletes it out from under the scan.

`agent_command` is unset by default; when unset, `leiter distill` uses its built-in `claude -p` command (see that
command). When set, it is an array of strings that **replaces the entire agent command line**: the first element is the
executable and the rest are its arguments. Leiter appends nothing to it — it only pipes the composed prompt on the
child's stdin — so with `agent_command` set the user owns the full command line. Its two uses are the distill test seam
(point it at a fake executable) and choosing the harness or model (e.g. adding `--model`, or swapping in `codex exec`).

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
setup_hard_epoch = 2
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

1. Create `~/.leiter/` directory (no-op if exists). Deliberately NOT `~/.leiter/logs/`: the logs directory is a hook-era
   artifact nothing writes to on a hookless box — the SessionEnd tombstone recreates it on demand for still-hooked
   migrating boxes, and `leiter distill` removes it once drained, so install recreating it would resurrect an empty
   directory on every converge
2. Converge the soul/state pair by which of the two files exist. "Fresh state" below means a `state.toml` with
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
     YAML frontmatter. If it does, this is the pre-`state.toml` legacy layout: run the legacy migration routine (see
     Legacy layout migration) — soul frontmatter plus any `~/.leiter/codex-meta.toml` become `state.toml`, the soul is
     rewritten frontmatter-free, and `codex-meta.toml` is deleted; the migrated `last_distilled` carries over from the
     old frontmatter and becomes the external-scan floor, so the box does not re-ingest history it already learned. The
     migrated `state.toml` records the binary's current epochs, not the old frontmatter's: completing this install _is_
     the action the hard-epoch bump demands, so the box must come out at the current hard epoch or it would trip the
     same migration message on the very next session and loop forever. Then normalize `~/.leiter/leiter.toml`: if it
     exists, load it (accepting the legacy `enable_codex_experimental` alias) and rewrite it so the modern `codex` key
     is persisted; the converge continues normally from the migrated state. If the soul does not parse as frontmatter,
     this is instead the documented corrupt-state recovery path (the user deleted `state.toml`): write fresh state and
     keep the soul untouched — preferences survive, watermarks reset.
   - **Soul missing, state exists**: verify epochs first, then recreate the soul from the template. The existing state
     is kept, not reset — its watermarks may be perfectly valid.
   - **Both exist**: if the soul still parses as YAML frontmatter, this is a legacy migration that crashed between the
     routine's state write and its soul strip — the state write is the migration's first mutation, and it is exactly the
     thing this quadrant discriminates on, so without this check the "re-runnable from any crash point" guarantee would
     be unreachable from install and the frontmatter would leak into the managed blocks forever. Re-run the migration
     routine (it keeps the existing state, strips the soul, deletes any leftover `codex-meta.toml`), then verify epochs.
     Otherwise: verify epochs.

   Ordering invariant: whenever both files are written, `state.toml` is written before `soul.md`, and validation runs
   before any write. This keeps a torn or refused install self-healing: dying between the two writes leaves the
   soul-missing/state-present shape, which the next run repairs, never the soul-present/state-missing shape that reads
   as a legacy layout
3. Verify the Claude Code home directory exists (error if not — Claude Code not installed). This check runs BEFORE the
   state/soul convergence of step 3 in execution order: the legacy migration is a one-way epoch advance, and running it
   only to then bail on a missing or mistyped Claude home would leave a migrated box whose still-configured hooks no
   longer show the migration message while no managed block exists to deliver the soul. (Steps are numbered by what they
   converge, not strictly by execution sequence; this is the one place order is load-bearing enough to state.)
4. Write the single `<claude_home>/skills/leiter/SKILL.md` skill file, overwriting on re-run (idempotent). Remove any of
   the old `<claude_home>/skills/leiter-*/` directories whose `SKILL.md` carries the `SCODE_LEITER_INSTALLED` sentinel —
   this collapses a previous six-skill install down to the one skill and is safe to re-run
5. Write the managed soul-delivery block into `<claude_home>/CLAUDE.md` (always), and into `<codex_home>/AGENTS.md` when
   `codex = true` — effectively a `leiter sync` of both targets, using the block writer and clobber-guard semantics from
   Managed soul-delivery blocks. This runs after the state/soul convergence so the block reflects the just-materialized
   soul, and it records the `[sync.*]` hashes. A fresh install has no recorded block hash, so both targets are written
   unconditionally
6. Read-only inspection of `<claude_home>/settings.json` for leftover hooks. Leiter never edits `settings.json` itself;
   it only reads it and, when it finds hook entries whose command contains `leiter hook`, folds hook-removal
   instructions and a behavior-change summary into its output (see below). When no such entries exist — a fresh install,
   or a box that already had its hooks removed — none of that is emitted. A missing or unreadable `settings.json` is
   treated as "no leiter hooks present" and is never fatal

**Output (stdout):** A success message confirming what was written — the `CLAUDE.md` managed soul block (and the
`AGENTS.md` block when `codex = true`) and the single `leiter` skill — and noting that the soul is now delivered inline
via the managed block, so soul injection no longer depends on a hook.

When step 7 found `leiter hook` entries in `settings.json`, the output additionally carries the migration payload for
the agent to act on and relay:

- Agent-usable hook-removal instructions: find the entries whose command contains `leiter hook`, remove those entries,
  clean up any arrays and objects left empty by the removal, and preserve everything else byte-for-byte. Keep the
  `Bash(leiter:*)` and soul-file permission entries — they are still useful and are not hooks (this is the same edit
  `leiter claude agent-teardown-instructions` emits, and the output can point the agent there).
- A behavior-change summary to relay to the user: session-start distillation nudges and auto-distillation are gone;
  `leiter distill` is now the distillation mechanism and, being non-interactive, is cron-able — include one sample
  crontab line; and transcript retention is no longer leiter's own `~/.leiter/logs/` copy but Claude Code's own session
  store, bounded by its `cleanupPeriodDays` (~30-day default), so distill within that window or raise the setting.

If a legacy migration ran in step 3, the message also states that the box was migrated to the hookless layout (soul
stripped to pure content, `state.toml` written, `codex-meta.toml` absorbed, `leiter.toml` normalized to the `codex`
key).

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

**Exit code:** non-zero when any target was refused, zero otherwise. The instill flow and the consolidated skill tell
agents to run `leiter sync` as a routine final step, so whether a refusal constitutes failure is contractual: it does,
loudly, and the remedy is in the refusal line. `leiter codex install` shares this posture — a refused `AGENTS.md` write
exits non-zero after `codex = true` has already been persisted, pointing at `leiter sync --force`.

### `leiter distill`

The primary distillation mechanism: leiter scans the session stores, hands the new transcripts to a headless agent that
edits the soul, and then commits its own bookkeeping. This is what the consolidated `leiter` skill's distill route runs,
and — because it needs no interactive session — it is cron-able. The lower-level `leiter soul distill` and
`leiter soul mark-distilled` remain for debugging and scan testing (see those commands), but this path never delegates
the mark step to the LLM: leiter commits state itself, and only after a verified successful run, which removes the old
"agent forgot to mark" failure mode.

**Flags:**

- `--dry-run`: emit the fully composed prompt to stdout and stop — this short-circuits before the empty-scan handling,
  so it prints the (possibly transcript-empty) prompt rather than the nothing-to-distill message — invoke no agent and
  write nothing (no pending watermark staging either, matching the `--dry-run` posture of `leiter soul distill`).

This command deliberately has **no** home-override flags; its scan uses the default Claude and Codex homes. The plumbing
`leiter soul distill` keeps its `--claude-home`/`--codex-home` flags for scan testing (and, as noted there, those
overrides disable its opportunistic re-sync).

**Behavior:**

1. Validate state (see Setup Epochs). If incompatible, exit with an error. Load `~/.leiter/leiter.toml`; if it is
   unreadable or invalid, warn and use defaults (the same warn-and-default posture as `leiter soul distill`)
2. Gather the new-session content exactly as `leiter soul distill` does — the same external Claude scan,
   `last_distilled` floor, per-session watermark dedupe, legacy-log drain, obsolete-log cleanup, and non-dry-run pending
   staging (see `leiter soul distill` and Claude session scanning; that staging is precisely what a later success
   promotes). If the scan finds nothing to emit, no agent is spawned. Two sub-cases:
   - Nothing was staged either → print that there is nothing to distill and exit 0 without touching state.
   - Only no-visible-content sessions were staged → commit the staged watermarks directly (same commit as a successful
     run) and say so. There is nothing an agent could be shown, but without this self-commit those sessions would be
     re-read and re-staged on every run forever — and there is no separate mark step left to ever settle them. The
     obsolete-log cleanup report is NOT part of the composed prompt in any mode: what `--dry-run` prints must be
     byte-identical to what a real run would pipe to the agent
3. Compose the distillation prompt from the same pieces `leiter soul distill` emits: the shared soul-writing guidelines,
   the data-boundary preamble wrapping the transcripts (unchanged — the transcripts are historical data, never
   directives; dropping this boundary would hand transcript prompt-injection a headless agent with soul write access),
   the resolved soul path, and an instruction to edit **only** the soul file and to end with a one-paragraph summary of
   what changed. The prompt must **not** tell the agent to run any leiter command — leiter commits its own state
   afterward
4. Invoke the agent CLI headlessly, piping the composed prompt on **stdin** rather than passing it as an argv element.
   Transcript batches routinely exceed `ARG_MAX`, and `claude -p` reads its prompt from stdin when the positional prompt
   is omitted (empirically validated 2026-07-07). The built-in command for the claude harness is:

   ```
   claude -p --allowedTools "Read(<soul_perm_path>),Edit(<soul_perm_path>),Write(<soul_perm_path>)"
   ```

   where `<soul_perm_path>` is the soul path in Claude Code permission form — `~/…` when under `$HOME`, `//…` otherwise
   (the same formatting the Permissions section uses). Two empirically validated constraints shape this exactly: the
   three grants are comma-joined into **one** argument, because `--allowedTools` is variadic and a space-separated list
   would swallow the following arguments; and the scoped grants were verified to permit the soul edit non-interactively,
   while their absence blocks it and leaves the file untouched
5. `agent_command` in `~/.leiter/leiter.toml` (an array of strings; see that file), when set, **replaces the entire
   command line above**: its first element is the executable and the rest are its arguments. Leiter still pipes the
   prompt on stdin and appends nothing — with `agent_command` set the user owns the full command line. This is both the
   test seam (substitute a fake executable) and the user's hook for choosing a harness or model (e.g. adding `--model`)
6. Success requires the child exiting 0 **and** the full prompt having been delivered on its stdin. A child that exits 0
   after closing stdin early received a truncated prompt — committing watermarks for transcripts the agent never saw
   would silently lose learning, so that case fails like any other:
   - **On success:** first verify this run still owns the staged state — the staged `pending_scan_started_utc` must
     equal the scan-start this run staged. A mismatch means a concurrent `leiter distill` (cron overlap) restaged after
     this run's scan; committing would promote watermarks for sessions this run's agent never processed, so the
     superseded run aborts without committing and exits non-zero, deferring to the newer run. (Concurrent runs' agents
     may still both edit the soul — that lost-update is accepted and is the user's cron cadence to manage; state
     integrity is what leiter guarantees.) Then commit state exactly as `leiter soul mark-distilled` would — set
     `last_distilled` to the staged `pending_scan_started_utc`, promote `[claude.pending]` into `[claude.committed]`
     always and `[codex.pending]` when `codex = true`, in a single atomic write — then opportunistically re-sync any
     stale managed block (see Opportunistic re-sync; refusal warnings go to stderr, never stdout). Then print the
     agent's stdout (its one-paragraph summary) followed by a confirmation line naming the new `last_distilled`, and
     relay the agent's stderr to leiter's stderr (a successful run's diagnostics matter to cron users too). Finally,
     sweep the legacy `~/.leiter/logs/`: delete every legacy log whose filename timestamp is strictly below the
     just-committed `last_distilled`, then remove the directory if it is now empty. The sweep runs after the commit on
     purpose — a log emitted by THIS run sits above `last_distilled` at scan time, so the scan-time obsolete cleanup
     cannot delete it, and without the post-commit sweep the migration flow's "the first post-migration distill drains
     and removes the logs directory" would quietly take two runs. Post-commit, everything below the new cutoff is
     provably committed history, so the sweep is exactly as crash-safe as scan-time cleanup (on a failed run the
     post-commit sweep never executes — scan-time obsolete cleanup is the only deletion such a run performs). Sweep and
     `rmdir` failures are warnings, not command failures
   - **On non-zero exit, spawn failure** (e.g. the agent binary is missing from `PATH`)**, or truncated stdin
     delivery:** print the agent's stderr and stdout, commit **nothing**, and exit non-zero. The pending watermarks
     staged during the scan are left staged; that is harmless because they are never promoted without a successful run,
     and the next `leiter distill` restages from current reality Relayed child output (both streams, all paths) is
     stripped of C0 control characters other than newline and tab before it reaches leiter's own streams: the summary is
     LLM-generated from transcript-derived content, and terminal escape sequences must not ride it into the user's
     terminal or cron mail
7. Retention cadence warning: when any external Claude session emitted this run has a file mtime older than
   `retention_warn_days` (see `~/.leiter/leiter.toml`), log a warning (stderr) that distillation is running close to
   Claude Code's retention pruning and should run more often. This is the cron-facing twin of the `leiter status`
   retention warning — a user driving distillation from cron never sees status output, and stderr is what cron mails

**Output (stdout):** On success, the agent's summary followed by the new-`last_distilled` confirmation line. On an empty
scan, the "nothing to distill" message. Under `--dry-run`, the full composed prompt. Diagnostics and warnings go to
stderr as usual.

### `leiter status`

A read-only report of leiter's distillation and soul-delivery health. It **never writes anything** — not the state file,
and not even the opportunistic re-sync that other state-mutating commands perform (see the read-only exemption under
Opportunistic re-sync). Reporting block staleness while silently healing it would make its own output unfalsifiable.

Like `leiter distill`, this command has no home-override flags; its scan uses the default Claude and Codex homes.

**Behavior:**

1. Validate state (see Setup Epochs). An incompatible state (missing soul, corrupt or unsupported-version `state.toml`,
   or a hard epoch mismatch) is an error, as for every other non-`session-end` command. Load `~/.leiter/leiter.toml`;
   warn and use defaults if it is unreadable or invalid
2. Run the same external Claude scan and legacy-log dedupe as `leiter soul distill`, but emit and stage nothing, to
   count the sessions a distill would currently process (discovered-and-would-emit external sessions plus legacy logs,
   after dedupe)
3. When `codex = true`, run the Codex scan the same read-only way to count undistilled Codex sessions
4. For each managed block target (`CLAUDE.md`, plus `AGENTS.md` when `codex = true`), compare the recorded `soul_hash`
   against the current soul body (stale copy?) and the on-disk block against the recorded `block_hash` (hand-edited
   inside the managed span?)
5. Compute the retention warning: whether any undistilled external Claude session's transcript file has an mtime older
   than `retention_warn_days` days (see `~/.leiter/leiter.toml`; default 21)

**Output (stdout):** A human-readable report of the undistilled Claude session count, the undistilled Codex count when
Codex is enabled, per-target managed-block state, a soft-epoch advisory when `setup_soft_epoch` differs from the
binary's (suggesting a `leiter claude install` re-run or a binary upgrade, matching the mismatch direction — this is the
retired session-start nudge's new home), and — when triggered — the retention warning naming the at-risk sessions. The
per-target block states are: **in sync**, **stale** (recorded soul hash behind the current soul, or the block missing
from the file), **hand-edited** (block present but not matching the recorded hash), **never synced** (no recorded hashes
for the target), and **unreadable** (the target file could not be read or its sentinel structure is malformed).
Unreadable is deliberately distinct from hand-edited: "hand-edited" implies `leiter sync --force` is the remedy, which
is wrong advice for a permissions problem or a mangled sentinel pair.

**Exit status:** `leiter status` always exits 0 once state validates. The conditions it reports — staleness, hand edits,
pending sessions, retention pressure — are informational, not failures: it is a status report, not a health check. (An
incompatible `state.toml`, from step 1, is the one non-zero path, since status cannot report on a state it cannot
verify.) A scan-side read failure (e.g. an unreadable legacy log file) does not abort the report: status prints what it
could determine, notes the failure as a warning line, and still exits 0.

### `leiter claude agent-setup-instructions`

A pure tombstone. Hooks are retired: leiter no longer configures any, and this command no longer emits hook-setup JSON.
It survives one release only so the old `/leiter-setup` skill (and any muscle-memory direct runs) does not hit a clap
unknown-command error. It is removed in the release after 0.9.0, together with the `hook *` subcommands.

**Behavior:** Validates state (see Setup Epochs). If incompatible, exits with an error — on a legacy layout that error
is the migration message, which is what a pre-0.9.0 box invoking this via `/leiter-setup` needs to see.

**Output (stdout):** On a healthy box, a short note that leiter runs hookless now, that the soul is delivered by the
managed `CLAUDE.md` block, and that there is nothing to configure. On a legacy layout, the migration message (see Epoch
Error Messages) instead.

### `leiter claude agent-teardown-instructions`

Outputs natural language instructions for the agent to remove leiter hooks from `~/.claude/settings.json`. Unlike
`agent-setup-instructions`, this command stays functional: emitting hook-removal instructions is exactly what a
migrating box needs, and `leiter claude install`'s migration output points at it. It too is removed in the release after
0.9.0 — by then no box has leiter hooks left to remove.

**Behavior:** Validates state (see Setup Epochs). If incompatible, exits with an error.

**Output (stdout):** Instructions telling the agent to find and remove hook entries whose command contains `leiter hook`
(the same substring rule install's migration payload uses — it matches all three legacy hook commands), clean up empty
arrays, preserve non-leiter hooks, and provide cleanup/re-enable guidance to the user. It leaves the `Bash(leiter:*)`
and soul-file permission entries in place — they are still useful and are not hooks.

### `leiter hook context`

A one-release tombstone for the SessionStart hook. It no longer injects the soul: the managed `CLAUDE.md` block is the
sole delivery path now, and re-injecting here would be exactly the double injection this migration retires. The hook is
only still configured on a pre-0.9.0 box (or a mid-migration box whose `settings.json` cleanup has not run yet), so the
command's whole job is to serve those boxes during migration. It is removed in the release after 0.9.0.

**Behavior:**

1. Validate state (see Setup Epochs). On any incompatibility, output that case's agent-relay message from Epoch Error
   Messages and return without injecting anything. For a pre-0.9.0 box that is the **Legacy layout** message (missing
   `state.toml` with a frontmatter soul — the only case whose message invites the agent to run `leiter claude install`
   itself); a hard-epoch mismatch on a modern layout gets the ordinary setup-outdated / binary-outdated text, corrupt or
   unreadable state gets its own messages, and a genuinely uninitialized box gets the not-initialized line. This hook
   output is how a migrating user first learns to run `leiter claude install`
2. On a healthy hookless box (state current), the hook being configured at all means `settings.json` still carries
   leiter hooks — `leiter claude install` migrates state but never edits `settings.json` itself, so a box whose agent
   has not yet applied the cleanup instructions lands here. Output ONE short line telling the agent to briefly remind
   the user that leiter's hooks are no longer needed and can be removed via the instructions in
   `leiter claude install`'s output. Do not emit the soul — the managed block already delivered it this session

The old soul preamble and the soft-epoch nudge are gone from this command. There is no case in which it emits the soul.

### `leiter hook session-end`

Hook handler for the Claude Code SessionEnd event. Reads the SessionEnd hook JSON from stdin and copies the session
transcript to the logs directory. Unlike the other `leiter hook *` commands, this one keeps its original behavior rather
than becoming a migration tombstone: it is epoch-exempt, and a user who defers migration for days must not silently lose
a single session. It dies with the rest of the `hook *` subcommands in the release after 0.9.0, by which point no box
still has the SessionEnd hook configured.

Historical context: SessionEnd (rather than Stop) was chosen for logging because Stop fires on every turn — not just at
session end — which would block the agent on every response to write a log. SessionEnd fires once when the session
actually terminates and hands over the transcript path directly, so no agent involvement was needed to save it. The
hookless architecture retires this path entirely: `leiter distill` now reads Claude Code's own session store in place
(see Claude session scanning), so no copy step is needed at all. This command survives only to keep archiving on
un-migrated boxes.

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

A missing `~/.leiter/logs/` directory is recreated, not an error. A successful `leiter distill` removes the logs
directory once it drains empty, and this hook stays active on un-migrated boxes — the two would otherwise interact as
"first drained distill permanently breaks every later session save" (a procrastinating user can distill manually and
keep running old sessions). The hook's charter is that losing session data is worse than anything else it could do, so
it makes the directory it needs.

**Output:** None. A confirmation message with the saved file path is logged to stderr (via `tracing`). The SessionEnd
hook fires after the session terminates, so no agent is present to read stdout.

**No-op case:** If the transcript file does not exist (e.g., Claude Code does not write a transcript for zero-turn
sessions), log a debug message and exit successfully. No log file is created.

**Errors:** If the transcript file exists but cannot be read, the write fails, or the atomic rename fails, print an
error to stderr and exit with a non-zero code. Clean up the temporary file on any error.

### `leiter soul distill`

Lower-level distillation plumbing, exposed for debugging and scan testing; `leiter distill` is the primary mechanism
most users and the consolidated skill invoke (see `leiter distill`), and it reuses this command's scan, staging, and
prompt composition rather than duplicating them. Outputs session logs that haven't been processed since the last
distillation. It draws from two Claude sources that can coexist during migration: the legacy `~/.leiter/logs/` files the
SessionEnd hook copied — still archived on un-migrated boxes, and left behind for the first post-migration distill to
drain on a just-migrated one — and the external scan of Claude Code's own session store (see Claude session scanning),
which is now the primary source. Their output is deduplicated by session id so a session present in both places is
emitted once. The release after 0.9.0 removes the SessionEnd hook entirely, and with it the legacy path and this dedupe.

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

The envelope's closing tokens are neutralized inside transcript bodies: any literal `</session>` or
`</session-transcripts>` occurring in rendered transcript content is broken (`</` becomes `< /`) before emission, the
same defense the managed-block writer applies to its sentinels. Without this, a transcript containing a forged closing
tag would place everything after it outside the data boundary — text that reads as prompt rather than quoted data, which
matters most on the headless `leiter distill` path where the consumer is an autonomous agent holding soul write access.

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

**Claude session scanning:** `leiter soul distill` reads Claude Code's own session transcripts directly from the Claude
home directory — the primary and, on a migrated box, only Claude source (the hook-copied `~/.leiter/logs/` files are a
migration leftover, drained once and gone). Claude Code stores one JSONL transcript per session at
`<claude_home>/projects/<cwd-slug>/<session-uuid>.jsonl`, appended live while the session runs. This external scan is
always active; it is not gated on `codex`.

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

**Self-session exclusion:** the headless `leiter distill` child is itself an agent session, and its transcript —
containing the full distill prompt, i.e. every transcript emitted that cycle — lands in a session store like any other.
Without an exclusion, the next scan emits that transcript, whose embedded payload contains the previous cycle's payload,
and so on: prompt size accumulates without bound across cycles until the agent rejects the prompt outright and
distillation is permanently wedged (observed live: a few cycles reached several million tokens). The rule: the headless
prompt begins with a fixed sentinel line, and a session is classified as leiter's own child exactly when the **first
user message of its transcript begins with that sentinel** — the position occupied by the prompt leiter piped in, and a
position that no injected tool output, fetched web page, quoted document, or soul-synced block content can ever occupy.
Detection is deliberately positional, NOT substring-anywhere: a sentinel appearing later in a transcript (pasted, echoed
by a tool, embedded in an emitted payload, or leaked into the soul and thence into every session via the managed block)
is inert, because content-based matching would hand prompt injection a silent suppress-this-session-from-learning
primitive and, via the soul-sync channel, a global learning shutdown. The remaining false-positive is exactly one shape:
a session whose very first prompt characters are the sentinel, which cannot happen by accident.

Child classification applies to every source the scan can emit from, because the child's transcript lands wherever
`agent_command` points: the external Claude scan (default `claude -p` children), the Codex rollout scan (a `codex exec`
`agent_command` writes the child into the Codex session store; the check runs against the rollout's first user-role
input), and the legacy `~/.leiter/logs/` rendering (a still-hooked migrating box's SessionEnd tombstone copies the
child's raw transcript into logs, and an unaccounted copy would otherwise re-emit the very payload whose size wedged the
previous run). A classified child in a watermarked store is recorded as processed **without emission** — its content is
synthetic (leiter's own prompt plus the agent's summary), so nothing is lost; a classified child among the legacy logs
is simply not rendered, and the ordinary drain lifecycle deletes it. Excluded session ids are named in a log line, not
silently dropped.

`leiter distill --dry-run` replaces **every** occurrence of the sentinel in its printed output (the first line and any
payload-borne straggler), so a session that merely inspected the prompt cannot classify as a child even in principle.
The interactive `leiter soul distill` payload deliberately carries no sentinel: the session that ran it also contains
organic content worth learning, and its one-time payload re-feed is bounded because the recursive compounding only ever
happens through headless child transcripts, which the exclusion stops in every channel.

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

Dedupe against the legacy logs is what keeps this coexistence clean during migration. On an un-migrated box the
SessionEnd hook still copies every finished transcript into `~/.leiter/logs/`, and a just-migrated box has a backlog of
such files that the first post-migration distill drains, so for a while some sessions exist in both places at once. A
session id emitted from the external scan suppresses the legacy log file bearing the same session-id suffix — the
external copy is the same or fresher content. The suppressed legacy file still participates in obsolete-log cleanup
exactly as it does today. The release after 0.9.0 removes the SessionEnd hook, and the legacy path and this dedupe go
with it.

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

Lower-level plumbing paired with `leiter soul distill`; the headless `leiter distill` path performs this same commit
itself on a successful run and never delegates it to the LLM (see `leiter distill`). Commits the last distill run's
cutoff: `last_distilled` advances to the scan-start time that run staged in `pending_scan_started_utc`, not to the
mark-time wall clock. The distinction closes a loss window — a session created after distill's scan but before the mark
would sit below a mark-time floor forever (no committed watermark, mtime too old), and the same window would delete
hook-copied logs as obsolete without ever distilling them. A scan-start cutoff guarantees everything born after the scan
is the next run's responsibility. This is the only way `last_distilled` should be updated — the agent must never edit it
manually. This command never writes `soul.md`.

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

A silent one-release tombstone for the second SessionStart hook. It outputs nothing, ever, and never fails. Its old
staleness scan and its `--auto-distill` behavior are gone: `leiter hook context` already delivers the migration message
on a migrating box, so a nudge here would be double messaging, and there is no longer any nudge to emit on a healthy
box. It is removed in the release after 0.9.0.

**Flags:**

- `--auto-distill`: accepted and ignored. The flag survives only so a pre-0.9.0 `settings.json` that configured
  `leiter hook nudge --auto-distill` does not error on the new binary.

**Behavior:** Exit 0 with no output. It does not read state, scan logs, or emit anything.

**Output (stdout):** None, in every case.

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

Leiter no longer configures any hooks. `leiter claude install` writes none, and `leiter claude agent-setup-instructions`
is a tombstone that emits none. This section documents only what a pre-0.9.0 box still carries in
`~/.claude/settings.json`, and how the one-release tombstones serve those configs during migration until
`leiter claude install`'s cleanup instructions remove them. The release after 0.9.0 deletes the `hook *` subcommands;
any entry lingering past that is dead configuration.

### SessionStart Hook

A box set up before 0.9.0 still has:

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

Fires on every session start (new, resume, clear, compact). The stdout output is added as context for the agent. On a
migrating box `leiter hook context` now outputs the migration message instead of the soul (the managed `CLAUDE.md` block
delivers the soul), and `leiter hook nudge` is silent. Some pre-0.9.0 boxes configured the second entry as
`leiter hook nudge --auto-distill`; the flag is still accepted and ignored. Both entries are removed by the cleanup
instructions in `leiter claude install`'s output.

### SessionEnd Hook

A box set up before 0.9.0 still has:

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

Fires once when the session terminates. `leiter hook session-end` keeps its original behavior — it reads the SessionEnd
hook JSON from stdin (which includes `session_id` and `transcript_path`) and copies the transcript to `~/.leiter/logs/`,
so a user who defers migration loses nothing (see that command). It is the one tombstone still doing real work, and it
is removed with the rest in the release after 0.9.0.

## Permissions

Two permission entries stay useful in the hookless world. `leiter claude install`'s migration output points at them
(rather than the retired `agent-setup-instructions`), so a migrating box keeps them while its hooks are cleaned out:

1. **Bash commands:** `"Bash(leiter:*)"` — allows all leiter CLI commands without confirmation dialogs.
2. **Soul file access:** `"Read(<soul_path>)"`, `"Edit(<soul_path>)"`, and `"Write(<soul_path>)"` — allows reading,
   editing, and writing the soul file without confirmation dialogs. Claude Code's `permissions.allow` uses
   gitignore-style path matching: `/path` is project-relative, `//path` is absolute, and `~/path` is home-relative. A
   bare absolute path like `/Users/alice/.leiter/soul.md` would be interpreted as project-relative and never match. The
   soul path must be formatted as `~/.leiter/soul.md` (when under `$HOME`) or `//path/to/soul.md` (otherwise).

The old auto-distillation permission option is gone: it was purely a hook toggle (`leiter hook nudge --auto-distill`),
and hooks are retired. Running `leiter distill` from cron is the replacement (see `leiter claude install` and Migration
from hook-based setups).

Both `leiter claude install`'s hook-cleanup instructions and `leiter claude agent-teardown-instructions` deliberately
leave these two entries in place — they are not hooks, and they remain useful for the tool that stays installed.

## Flows

### First-Time Setup

Setup is one terminal command. There is no in-session step, and no hooks to configure.

1. User installs `leiter` binary
2. User runs `leiter claude install` from their terminal
3. The command creates the `~/.leiter/` structure, writes the single `leiter` skill to `~/.claude/skills/`, and writes
   the managed soul block into `~/.claude/CLAUDE.md` (and `~/.codex/AGENTS.md` when `codex = true`)
4. On the next session start, the harness reads the managed block, so the agent has the soul and leiter instructions
   with no hook involved
5. Optionally, the user enables Codex delivery with `leiter codex install`, and accepts the `Bash(leiter:*)` and
   soul-file permissions if desired (see Permissions)

### Normal Session (After Setup)

1. Session starts → the harness reads the managed `CLAUDE.md` block, so the agent has the soul and leiter instructions.
   No hook fires
2. Normal session proceeds; the transcript is written live into `~/.claude/projects/` by Claude Code itself
3. Session ends → nothing leiter-specific happens. When distillation next runs, `leiter distill`'s external scan reads
   the transcript in place from `~/.claude/projects/` — there is no copy step

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

1. The user asks to distill (the agent auto-matches the consolidated `leiter` skill's distill route), or a cron job runs
   `leiter distill` unattended — it is the same command either way
2. `leiter distill` scans both harnesses' session stores, drains any legacy `~/.leiter/logs/`, and composes a prompt of
   the soul-writing guidelines plus the new transcripts (inside the data-boundary preamble) plus the resolved soul path
3. It invokes a headless agent (`claude -p` by default, or the configured `agent_command`) with tool permissions scoped
   to the soul file. The agent edits the soul and prints a one-paragraph summary of what changed
4. On a successful (zero-exit) run, leiter commits its own bookkeeping — advancing `last_distilled`, promoting the
   Claude (and, when enabled, Codex) `pending` watermarks, and opportunistically re-syncing the managed blocks so the
   edited soul reaches future sessions — then removes the now-empty legacy logs directory if the drain emptied it. The
   agent never runs `leiter soul mark-distilled` in this flow
5. The user sees the agent's summary followed by the new `last_distilled`. Because the command is non-interactive, it
   can run from cron or a systemd timer on whatever cadence keeps sessions distilled inside Claude Code's retention
   window

### Migration from Hook-Based Setups

This is the one-time path for a box set up before 0.9.0. It is driven entirely by the setup-epoch mechanism and the
agent — leiter never edits `settings.json` itself.

1. The user upgrades the `leiter` binary. It carries `setup_hard_epoch = 2`, which no longer matches the box's recorded
   `setup_hard_epoch = 1` (or its missing `state.toml` with a frontmatter soul)
2. At the next session start, the still-configured SessionStart hook runs the new binary's `leiter hook context`
   tombstone. The hard mismatch makes it output the migration message (verbatim, strong-compliance framing) instead of
   the soul, telling the user to run `leiter claude install` — or let the agent run it — then start a new session. The
   soul is not injected that session (ordinary hard-mismatch behavior). `leiter hook nudge` is silent, so there is no
   double message, and `leiter hook session-end` keeps archiving transcripts, so nothing is lost however long the user
   defers
3. The agent runs `leiter claude install`. It converges deterministically: `state.toml` is built from the soul
   frontmatter and any `codex-meta.toml`, the soul is rewritten frontmatter-free, the `CLAUDE.md` block is written (and
   `AGENTS.md` when Codex was enabled), `leiter.toml` is normalized to the `codex` key, and the six old skills collapse
   to one. Its output then carries the settings.json hook-removal instructions and the behavior-change summary to relay:
   session-start nudges and auto-distillation are gone, `leiter distill` is the cron-able replacement (with a sample
   crontab line), and transcript retention is now bounded by Claude Code's `cleanupPeriodDays` (~30-day default), so
   distill within that window or raise it
4. The first post-migration `leiter distill` drains the leftover `~/.leiter/logs/` files (those at or after the migrated
   `last_distilled`) alongside the external scan, deduplicated by session id, then removes the now-empty logs directory
5. Any old skill invoked against the new binary before migration (`/leiter-distill`, `/leiter-instill`, and the rest)
   hits the same legacy-aware validation and surfaces the Legacy layout message, so every path funnels to step 3. (Only
   the Legacy layout message invites the agent to run install itself; the plain hard-mismatch messages, which apply to
   modern-layout boxes, keep telling the agent to stay away from leiter commands and let the user run install)
6. The release after 0.9.0 deletes all the tombstoned subcommands (`hook *` and both `agent-*-instructions`); by then no
   box has hooks left for them to serve

## Non-Goals (For Now)

- Multiple user profiles or project-specific souls
- Automatic distillation by default (the user opts in by scheduling `leiter distill` from cron or a timer)
- Soul backup
- API key management or direct Claude API calls from the CLI
