# Leiter

A self-training system for [Claude Code](https://docs.anthropic.com/en/docs/claude-code). Leiter automatically learns
your preferences, coding practices, and workflow patterns across sessions by logging activity and distilling it into a
persistent "soul" that shapes future agent behavior.

## Install

```sh
brew install scode/dist-tap/leiter
```

Or from source:

```sh
cargo install --path .
```

## Setup

1. Run `leiter claude install` in your terminal. This creates `~/.leiter/` and installs skill files into
   `~/.claude/skills/`.
2. Start a Claude Code session and run `/leiter-setup`. This configures the hooks in `~/.claude/settings.json`.

On next session start, leiter is active.

## Usage

Session context injection and session logging happen automatically via hooks. The soul itself is updated in two ways:

- **Learning preferences:** Tell the agent "remember to always use snake_case" (or similar). It invokes the
  `/leiter-instill` skill to update the soul.
- **Distillation:** Periodically say "distill" to have the agent invoke `/leiter-distill`, which processes accumulated
  session logs and updates the soul. By default, if undistilled logs are older than 24 hours the agent will nudge you at
  session start. During `/leiter-setup` you can opt into automatic background distillation instead (4-hour threshold,
  runs silently).
- **Soul upgrade:** After updating the leiter binary, say "upgrade the leiter soul" to migrate to the latest template.
