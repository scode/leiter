# Should I grant permission to run leiter:\* commands?

Granting Claude permission to run `leiter:*` commands without a confirmation prompt is optional. You can add the entry
to `~/.claude/settings.json` by hand; on a box migrating from a pre-0.9.0 setup, `leiter claude install`'s output tells
the agent to keep the entry if it already exists, alongside the hook-removal instructions. This page covers what that
means, why you'd want it, and what you're signing up for.

## What this permission does

Claude Code has a permission system that prompts you before running shell commands. By default, every time Claude needs
to run a `leiter` CLI command, you'll get a confirmation dialog. Granting the `Bash(leiter:*)` permission tells Claude
Code to allow any command matching `leiter *` to run without prompting.

In practice, leiter needs to run CLI commands during normal operation — distilling session logs (`leiter distill`),
recording preferences (`leiter soul instill ...`), and a few others. Without this permission, you'll be prompted to
approve each of these individually, which gets tedious fast.

## Why you'd want to grant it

Leiter is designed to work quietly in the background. The whole point is that it learns without you having to think
about it. If you're getting prompted every time it needs to run a command, that defeats the purpose — you end up
babysitting the tool that's supposed to be saving you from babysitting.

Granting this permission makes leiter feel invisible, which is the intended experience.

## What you're signing up for

The `leiter:*` pattern matches any shell command that starts with `leiter`. This means Claude can run any leiter
subcommand without asking. The leiter commands are designed so that their behavior is predictable and deterministic —
they do a fixed set of things based on the command given, and cannot be tricked into doing something unexpected through
creative command-line arguments. This is intentional: the goal is that you can confidently allow all leiter commands
while having a good understanding of what they'll do, provided you trust the tool itself.

Here's what the leiter CLI can actually do:

- **Read and write your soul file** (`leiter distill`, `leiter soul instill`, `leiter soul upgrade`) — this is leiter's
  core function, so it's expected.
- **Read session transcripts** (`leiter distill` reads Claude Code transcripts in place under `<claude_home>/projects/`,
  Codex rollouts under `<codex_home>/` when Codex is enabled, and — only on a box migrated from a pre-0.9.0 setup — any
  leftover copies under `~/.leiter/logs/`) — again, core function.
- **Mark sessions as distilled** (`leiter distill`, or the lower-level `leiter soul mark-distilled`) — updates
  `last_distilled` and the distillation watermarks in `~/.leiter/state.toml`.
- **Manage Claude Code integration** (`leiter claude install`, `leiter claude uninstall`, `leiter sync`,
  `leiter codex install`/`uninstall`) — these write the managed soul block into `~/.claude/CLAUDE.md` (and
  `~/.codex/AGENTS.md` when Codex is enabled) and the `leiter` skill under `~/.claude/skills/`. They could in principle
  be invoked by the agent, though in normal use you run install/uninstall yourself from the terminal.
- **Change leiter configuration** (`leiter config set`) — sets persistent config values.
- (The exact set of commands may change in future versions and is not guaranteed to remain fixed. However, leiter
  commands are guaranteed to never become arbitrarily flexible — they will always do a bounded, predictable set of
  things that cannot be expanded through creative use of command-line arguments or environment variables.)

The leiter binary does not make network requests. Most commands are local-only file operations, but `leiter distill`
does run an agent CLI: by default `claude -p` with `Read`, `Edit`, and `Write` permissions scoped to the soul file, or
the exact `agent_command` configured in `~/.leiter/leiter.toml` if you set one. Leiter itself reads and writes a bounded
set of known files: leiter state under `~/.leiter/`, Claude Code plugin files, Claude Code session transcripts under
`<claude_home>/projects/`, and (when enabled) Codex rollout transcripts under `<codex_home>/`.

One risk vector worth mentioning: leiter respects a `LEITER_HOME` environment variable that overrides the default
`~/.leiter` state directory. The agent could, accidentally or through a prompt injection attack, set this variable
before running a leiter command, causing leiter to treat an arbitrary directory as its state directory. This would
affect where it reads and writes the soul file, session logs, and configuration.

The main operational risk is that Claude could, in theory, invoke `leiter claude uninstall` or `leiter config set`
without you asking. In practice this doesn't happen because the agent has no reason to, but the permission would allow
it. If that bothers you, skip this option and approve commands individually.

NOTE: The leiter commands are not 100% guaranteed to be hermetically sealed. The binary depends on typical Rust
libraries (logging, CLI parsing, file I/O, etc.) whose every line of code has not been individually inspected and fully
understood. Do not treat leiter commands as absolutely guaranteed to be safe under all possible circumstances. That
said, the surface area of exposure is extremely limited — leiter does simple, predictable file operations on a small set
of paths. A realistic exploit would require a highly targeted prompt injection attack or similar; it is not the kind of
thing that happens by accident.
