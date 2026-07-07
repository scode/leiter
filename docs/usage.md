# Usage

Once leiter is set up, your soul is delivered into every session automatically through the managed `~/.claude/CLAUDE.md`
block. This page covers the ways the soul gets updated, how distillation works, and how to check leiter's health.

All of the in-session actions below are driven by natural language. Leiter installs one consolidated `leiter` skill
whose description carries the trigger words (remember, learn, instill, always, never, distill, soul, upgrade), so Claude
routes matching requests to it and it runs the right `leiter` command for you. There are no slash commands to memorize.

## Teaching preferences (instill)

The fastest way to teach leiter is to tell the agent to "instill" a preference. Other trigger words like "remember",
"always", "never", and "learn" also work:

- "Instill that I prefer snake_case for Rust functions"
- "Never add emojis to commit messages"
- "Remember that I prefer explicit error handling over unwrap"
- "Always run clippy before considering work done"

The skill runs `leiter soul instill`, which hands the agent writing guidelines and tells it to edit `~/.leiter/soul.md`,
then run `leiter sync` so the managed block picks up the change. The preference takes effect immediately in the current
session and in all future sessions.

You can also instill broader patterns:

- "Remember that I prefer prose over bullet lists in documentation"
- "Learn that I use Graphite for branch management, not raw git"

The agent places each preference in the appropriate section of the soul file and resolves conflicts with existing
entries (newer observations replace older ones).

## Distillation

Distillation reads your recent session transcripts and folds in patterns you have not explicitly taught — things like a
consistent error-handling style, how you structure tests, or the kind of code review you tend to ask for. It is manual
by default; leiter never distills on its own unless you schedule it.

Trigger it two ways, both running the same `leiter distill` command:

- Say "distill" (or similar) in a Claude Code session. The skill runs `leiter distill`.
- Run `leiter distill` yourself from a shell.

`leiter distill` scans Claude Code's session store (and Codex's, when enabled), composes a prompt of the new transcripts
plus soul-writing guidelines, and hands it to a headless agent (`claude -p` by default, or your configured
`agent_command`) whose write access is scoped to the soul file (its read access is the harness default for its working
directory). The agent edits the soul and prints a one-paragraph summary of what changed. Leiter then commits its own
bookkeeping — advancing `last_distilled` and re-syncing the managed block — but only after a verified successful run.
You see the agent's summary followed by the new `last_distilled`.

NOTE: A session is only visible to distillation once Claude Code has written its transcript to disk. `leiter distill`
reads the transcript in place, so there is nothing to export first, but a session that is still in progress may not be
picked up until it has been written out.

### Running from cron

Because `leiter distill` is non-interactive, you can schedule it instead of triggering it by hand. A daily run keeps
your soul current without you thinking about it. For example, in your crontab:

```cron
0 3 * * * leiter distill
```

Distill on a cadence that keeps sessions inside Claude Code's retention window (its `cleanupPeriodDays`, ~30 days by
default) — once Claude Code prunes a session, it is gone before the scan can read it. When a run distills a session
whose transcript is already older than `retention_warn_days` (default 21), `leiter distill` logs a warning on stderr,
which is what cron mails you.

## Checking status

`leiter status` is a read-only report of leiter's health. It never writes anything. It reports:

- How many Claude sessions (and Codex sessions, when enabled) are waiting to be distilled.
- The state of each managed block (`CLAUDE.md`, and `AGENTS.md` when Codex is enabled): **in sync**, **stale** (the soul
  changed since the block was last written, or the block is missing), **hand-edited** (someone edited inside the managed
  span — rerun `leiter sync --force` to overwrite), **never synced**, or **unreadable** (a permissions problem or a
  mangled marker pair, which `--force` would not fix).
- A soft-epoch advisory when your state's soft epoch differs from the binary's — suggesting you re-run
  `leiter claude install` or upgrade the binary. This is where a recommended-upgrade notice lives now that there is no
  session-start nudge.
- A retention warning naming any undistilled Claude sessions whose transcripts are older than `retention_warn_days`.

`leiter status` exits 0 as long as validation passes (a readable soul, a valid state file, matching hard epochs) — the
things it reports are informational, not failures.

## Soul upgrades

When you update the leiter binary, the soul template may have changed (new sections, reorganized categories). Ask the
agent to "upgrade the leiter soul" (or it may tell you an upgrade is available). The skill runs `leiter soul upgrade`,
which hands the agent a changelog and the new template; the agent restructures your soul while preserving all learned
preferences, then runs `leiter soul mark-upgraded` to record the new version in `state.toml`. If the agent restructures
the soul but the version does not advance, that is harmless — the next upgrade attempt just re-emits the instructions.

## Viewing the soul

Ask to "show my soul" and the skill runs `leiter soul show`. The agent displays the learned preferences verbatim in a
fenced code block, with no hidden metadata stripped out — the soul is pure preferences content.

## The soul file

The soul lives at `~/.leiter/soul.md`. It is a plain markdown file you can read and edit directly — there is nothing
magic about it, and the agent edits it with the same tools it uses for any other file.

CLI-managed metadata lives in `~/.leiter/state.toml` instead: epochs, `soul_version`, `last_distilled`, the
managed-block sync hashes, and distillation watermarks. Do not edit `state.toml` by hand.

If you edit the soul directly — reorganizing, removing entries, adding things by hand — run `leiter sync` afterward so
the managed block is brought back in line. (The skill does this for you after any edit it makes on your behalf.) A block
you hand-edited between the markers is not silently overwritten; `leiter sync` warns and leaves it, and you rerun with
`--force` to accept the re-materialized copy.

`last_distilled` is the cutoff that decides which sessions a distill treats as new. It is advanced by `leiter distill`
after a verified successful run — set to the time that run's scan started, not the wall-clock mark time, so any session
created after the scan is the next run's responsibility. The lower-level `leiter soul mark-distilled` plumbing advances
it the same way. The agent must never edit it by hand.
