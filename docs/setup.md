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

Run `leiter claude install` in your terminal. This creates the `~/.leiter/` directory (containing your soul file and log
storage) and installs skill files into `~/.claude/skills/`.

```sh
leiter claude install
```

Then start a Claude Code session and run `/leiter-setup`. The agent reads the setup instructions and configures hooks in
`~/.claude/settings.json`:

- A **SessionStart** hook that injects your soul into every session
- A **SessionEnd** hook that saves session transcripts for later distillation

The agent also offers optional permission entries (see below) that get added to the same `settings.json`. The agent
shows you all proposed changes and asks you to approve them.

### Optional permissions

After configuring hooks, the agent offers three optional features. You can accept any combination or none:

1. **Bash commands** — allows leiter CLI commands to run without confirmation dialogs. Without this, Claude asks
   permission every time it runs a `leiter` command.
2. **Soul file access** — allows reading and editing `~/.leiter/soul.md` without confirmation. Without this, Claude asks
   permission when instilling preferences or during distillation.
3. **Auto-distillation** — runs distillation silently in the background at the end of the first turn when stale logs
   exist (4-hour threshold), instead of nudging you to do it manually. See [usage.md](usage.md) for more on
   distillation.

## Re-running setup after upgrades

When you upgrade the leiter binary, the new version may need updated configuration. Leiter detects this automatically
through an epoch system and tells you what to do:

- **Recommended changes** (soft epoch mismatch): The new version introduces changes that benefit from re-running setup,
  but are not strictly required. The agent suggests re-running and the session continues normally.
- **Required changes** (hard epoch mismatch): The new version introduces changes that require re-running setup. Leiter
  disables itself (the soul is not injected and commands refuse to run) until you do.

In either case, the fix is the same:

```sh
leiter claude install
```

Then run `/leiter-setup` again in a Claude Code session. The agent handles the upgrade — it detects the existing hooks
and updates them rather than duplicating them.

## Verifying it works

Start a new Claude Code session after setup. If leiter is active, the agent has your soul loaded — it knows your
preferences from previous sessions. You can verify by telling it to "remember that I prefer tabs over spaces" and
checking that `~/.leiter/soul.md` gets updated.

## File layout

After setup, leiter's files live in two places:

```
~/.leiter/
├── soul.md              # Your learned preferences (the "soul")
└── logs/
    └── *.jsonl          # Session transcripts

~/.claude/skills/
├── leiter-setup/        # Hook configuration skill
├── leiter-distill/      # Distillation skill
├── leiter-instill/      # Preference recording skill
└── leiter-teardown/     # Hook removal skill
```
