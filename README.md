# Leiter

A self-training system for [Claude Code](https://docs.anthropic.com/en/docs/claude-code). Leiter automatically learns
your preferences, coding practices, and workflow patterns across sessions by logging activity and distilling it into a
persistent "soul" that shapes future agent behavior.

## Install

```sh
cargo install --path .
```

## Setup

In a Claude Code session, paste the following prompt:

```
Run the shell command `leiter agent-setup` and follow the instructions it outputs on stdout.
```

This initializes `~/.leiter/` and configures Claude Code hooks. Once done, leiter is active on every future session.

## Usage

Session context injection and session logging happen automatically via hooks. The soul itself is updated in two ways:

- **Learning preferences:** Tell the agent "remember to always use snake_case" (or similar). It edits the soul directly.
- **Distillation:** Periodically say "distill" to have the agent process accumulated session logs and update the soul.
  Without this, logs accumulate but don't feed back into the soul.
- **Soul upgrade:** After updating the leiter binary, say "upgrade the leiter soul" to migrate to the latest template.
