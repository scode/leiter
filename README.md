# Leiter

A self-training system for [Claude Code](https://docs.anthropic.com/en/docs/claude-code). Leiter automatically learns
your preferences, coding practices, and workflow patterns across sessions by logging activity and distilling it into a
persistent "soul" that shapes future agent behavior.

## Install

```sh
cargo install --path .
```

## Setup

In a Claude Code session, say "set up leiter". The agent will run `leiter agent-setup`, which initializes `~/.leiter/`
and outputs instructions for the agent to configure Claude Code hooks in `~/.claude/settings.json`.

Once hooks are configured, leiter is active on every future session.

## Usage

Leiter works through Claude Code hooks — no manual CLI interaction is needed during normal use.

- **Learning preferences:** Tell the agent "remember to always use snake_case" (or similar). It edits
  `~/.leiter/soul.md` directly.
- **Session logging:** Happens automatically when a session ends (via the Stop hook).
- **Distillation:** Say "distill" to have the agent process recent session logs and update the soul.
- **Soul upgrade:** After updating the leiter binary, say "upgrade the leiter soul" to migrate to the latest template.
