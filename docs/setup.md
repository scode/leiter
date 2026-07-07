# Setup

## Prerequisites

- [Claude Code](https://docs.anthropic.com/en/docs/claude-code) installed and working (`~/.claude/` directory exists)
- macOS or Linux

## Install

```sh
brew install scode/dist-tap/leiter
```

Or from source (requires Rust — install via [rustup.rs](https://rustup.rs/)):

```sh
cargo install --path .
```

## Configure

Setup is a single terminal command:

```sh
leiter claude install
```

That is the whole thing. There is no in-session step and no hooks to configure. `leiter claude install` creates
`~/.leiter/` (your soul file and state), writes the one `leiter` skill into `~/.claude/skills/`, and writes a managed
block into `~/.claude/CLAUDE.md`. Claude Code already reads `CLAUDE.md` at session start, so that block is how your soul
reaches every new session — no hook required.

Re-running `leiter claude install` is safe and idempotent; it rewrites the skill and the managed block from the current
soul.

### Granting permissions (optional)

To keep leiter from prompting on every command, you can allow it in `~/.claude/settings.json`:

- `Bash(leiter:*)` — lets Claude run leiter CLI commands without a confirmation dialog.
- `Read`/`Edit`/`Write` on `~/.leiter/soul.md` — lets Claude read and edit the soul without prompting when instilling or
  distilling.

These are optional and you can add them by hand. See [leiter_command_permissions.md](leiter_command_permissions.md) for
what granting `Bash(leiter:*)` signs you up for. On a box migrating from a pre-0.9.0 setup, `leiter claude install`'s
output tells the agent to keep any such entries it already had, alongside its hook-removal instructions.

## Enabling Codex

Leiter can deliver your soul to [Codex](https://developers.openai.com/codex) as well and distill Codex sessions into the
same soul. Enable it with:

```sh
leiter codex install
```

This flips on Codex support in `~/.leiter/leiter.toml` and writes the managed block into `~/.codex/AGENTS.md` (which
Codex reads at session start, the same way Claude Code reads `CLAUDE.md`). Turn it back off with
`leiter codex uninstall`, which removes the `AGENTS.md` block and disables Codex support. See [codex.md](codex.md) for
details.

## Configuration

Settings live in `~/.leiter/leiter.toml`. The file is optional; every key has a default.

- `codex` (default `false`) — whether Codex support is active. Set by `leiter codex install`/`uninstall`; you rarely
  touch it directly. The older name `enable_codex_experimental` is still accepted when reading, and is rewritten to
  `codex` the next time leiter saves the file.
- `retention_warn_days` (default `21`) — how old, in days, an undistilled Claude session may get before `leiter status`
  and `leiter distill` warn about it. The default sits below Claude Code's roughly 30-day session-store pruning window,
  so the warning gives you lead time to distill a session before Claude Code deletes it out from under the scan.
- `agent_command` (unset by default) — the command line `leiter distill` runs to do the soul-editing. When unset,
  distill uses its built-in `claude -p` invocation, granting the agent write access to the soul file only. When set, it
  is an array of strings that **replaces the entire command line**: the first element is the executable, the rest are
  its arguments, and leiter appends nothing — it only pipes the composed prompt on the child's stdin. Use it to pick a
  model or a different harness, e.g.
  `agent_command = ["claude", "-p", "--model", "opus", "--allowedTools",
  "Read(~/.leiter/soul.md),Edit(~/.leiter/soul.md),Write(~/.leiter/soul.md)"]`,
  or to swap in `codex exec`.

## Upgrading

When you upgrade the leiter binary, a new version may need a one-time migration (for example, moving to a new
integration model). Leiter tracks this with a pair of setup epochs and tells you what to do:

- **Recommended (soft epoch mismatch):** the new version benefits from re-running install but does not require it. There
  is no session-start nudge anymore — the advisory shows up in `leiter status`, which suggests re-running install or
  upgrading the binary. Sessions keep working.
- **Required (hard epoch mismatch):** the new version needs install re-run before it works correctly. Leiter refuses to
  inject the soul and commands error out until you re-run install.

In both cases the fix is the same:

```sh
leiter claude install
```

Re-running install migrates a behind soft epoch forward and performs any required migration. The `1 → 2` hard-epoch bump
in 0.9.0 is the hookless migration itself — see below.

### Migrating from a pre-0.9.0 (hook-based) setup

Older leiter used Claude Code hooks: a SessionStart hook injected the soul and a SessionEnd hook copied transcripts into
`~/.leiter/logs/`. The hookless architecture retires both. You do not have to do anything special to be told about it —
when you upgrade the binary, your still-configured hooks detect the epoch mismatch on the next session start and print a
message telling you to run `leiter claude install`. You can also just run it directly.

`leiter claude install` performs the migration: it converts your old layout (soul frontmatter plus any
`codex-meta.toml`) into the current `state.toml`, strips the soul to pure markdown, writes the managed `CLAUDE.md`
block, collapses the old six `leiter-*` skills down to the one `leiter` skill, and normalizes `leiter.toml`. Its output
then carries instructions for the agent to remove the leftover hook entries from `~/.claude/settings.json` — leiter
never edits that file itself — plus a summary of what changed: distillation is now manual (`leiter distill`, cron-able)
rather than nudged or automatic, and transcript retention is now bounded by Claude Code's own `cleanupPeriodDays`
(~30-day default) rather than a leiter copy, so distill within that window.

Nothing is lost if you defer. Until the hooks are removed, the SessionEnd hook keeps archiving transcripts into
`~/.leiter/logs/`, and the first `leiter distill` after migration drains those leftover files (then removes the empty
directory) alongside its normal scan.

## Verifying it works

Start a new Claude Code session after setup and ask the agent what leiter is, or tell it to "remember that I prefer tabs
over spaces" and check that `~/.leiter/soul.md` was updated. If the soul is loaded, the agent knows about leiter and
your preferences from the managed block.

## File layout

After setup, leiter's files live in two places:

```
~/.leiter/
├── leiter.toml         # Settings (optional; created on first config write)
├── soul.md             # Your learned preferences (the "soul"), pure markdown, no metadata
└── state.toml          # Leiter-managed epochs, soul_version, last_distilled, sync hashes, and
                        #   distillation watermarks; the agent must never edit this

~/.claude/
├── CLAUDE.md           # Carries the managed soul block (between SCODE_LEITER_BEGIN/END markers)
└── skills/
    └── leiter/         # The one consolidated leiter skill
```

When Codex is enabled, `~/.codex/AGENTS.md` also carries the managed soul block.

A `~/.leiter/logs/` directory only exists on a box migrated from a pre-0.9.0 setup, holding transcripts the old
SessionEnd hook copied. The first `leiter distill` after migration drains those files and removes the directory. A fresh
hookless install never creates it — distillation reads Claude Code's session store in place.
