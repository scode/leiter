# Leiter

A self-training system for [Claude Code](https://docs.anthropic.com/en/docs/claude-code). Leiter logs your Claude Code
sessions and distills them into a persistent "soul" — a file of learned preferences, coding practices, and workflow
patterns that gets injected into future sessions to shape agent behavior.

## Install

```sh
brew install scode/dist-tap/leiter
```

Or from source:

```sh
cargo install --path .
```

## Setup

Run `leiter claude install` in your terminal. This creates `~/.leiter/` and installs skill files into
`~/.claude/skills/`. Then start a Claude Code session and run `/leiter-setup` to configure the hooks in
`~/.claude/settings.json`.

On your next session start, leiter is active.

## Usage

Session context injection and logging happen automatically via hooks. The soul gets updated in three ways:

**Learning preferences.** Tell the agent something like "remember to always use snake_case" and it invokes the
`/leiter-instill` skill to update the soul immediately.

**Distillation.** Periodically say "distill" to have the agent invoke `/leiter-distill`, which processes accumulated
session logs and folds any new patterns into the soul. If undistilled logs are older than 24 hours, the agent nudges you
at session start. During `/leiter-setup` you can opt into automatic background distillation instead (4-hour threshold,
runs silently).

**Soul upgrade.** After updating the leiter binary, say "upgrade the leiter soul" to migrate to the latest template.
