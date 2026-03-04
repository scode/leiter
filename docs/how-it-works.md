# How It Works

Leiter is built on a simple idea: the Claude agent does all the thinking, and the `leiter` CLI handles plumbing. The CLI
never calls the Claude API. It manages files, timestamps, and context injection — the agent reads, writes, and decides
what to remember.

## The learning loop

Leiter has three inputs that feed into a single output (the soul file):

1. **Direct teaching** — you tell the agent to remember something, it writes it to the soul immediately.
2. **Session logging** — every session transcript is saved automatically.
3. **Distillation** — accumulated transcripts are periodically processed, and any new patterns are extracted and added
   to the soul.

The soul file is then injected into every future session at startup, so the agent starts with all your preferences
already loaded.

## Session lifecycle

Here is what happens during a typical session, from start to finish:

```
┌──────────────────────────────────────────────────────────────┐
│                       Claude Code Session                    │
│                                                              │
│  SessionStart hook ──► leiter hook context ──► soul + agent  │
│                        leiter hook nudge        instructions │
│                                                injected      │
│                                                              │
│  ... normal session ...                                      │
│                                                              │
│  /leiter-instill (or "instill X") ──► /leiter-instill skill   │
│                           ──► agent edits soul.md            │
│                                                              │
│  /leiter-distill (or "distill") ──► /leiter-distill skill     │
│                        ──► sub-agent processes logs           │
│                        ──► sub-agent edits soul.md           │
│                        ──► agent marks distillation timestamp │
│                                                              │
│  SessionEnd hook ──► leiter hook session-end                 │
│                      ──► copies transcript to logs/          │
└──────────────────────────────────────────────────────────────┘
```

### Session start

Two hooks fire when a session starts (new, resumed, cleared, or compacted):

1. `leiter hook context` reads `~/.leiter/soul.md` and outputs the full soul content plus a short preamble explaining
   how to interact with leiter. This becomes part of the agent's context — it sees your preferences as if they were
   system instructions.

2. `leiter hook nudge` checks whether any session logs are overdue for distillation. If undistilled logs are older than
   24 hours, it tells the agent to suggest running distillation. If you opted into auto-distillation during setup, it
   instead uses a 4-hour threshold and tells the agent to run distillation silently in the background.

The soul content is injected inline (not as a file path) so that it survives context compaction in long sessions. Even
after earlier messages are compressed, your preferences remain available.

### During the session

The session proceeds normally. When you tell the agent to "remember" something (or use similar language like "always",
"never", "learn"), it invokes the `/leiter-instill` skill, which provides writing guidelines and tells the agent to edit
the soul file directly.

### Session end

When the session terminates, the SessionEnd hook fires and `leiter hook session-end` copies the transcript to
`~/.leiter/logs/`. The transcript is a JSONL file containing the full conversation. This happens silently — no agent
involvement is needed.

## The soul file

The soul is a markdown file at `~/.leiter/soul.md` with YAML frontmatter for metadata (timestamps, version numbers) and
a body containing your learned preferences organized by category. The agent edits this file directly using its standard
file editing tools. The CLI creates the initial soul during install and may update frontmatter during upgrades, but the
body — your actual preferences — is only ever written by the agent.

See [usage.md](usage.md) for details on the soul file format and how to customize it.

## Distillation

Distillation is the process of extracting patterns from session transcripts and instilling them into the soul. It runs
in a sub-agent (a separate context window) to keep the raw transcript data out of your main session.

The sub-agent runs `leiter soul distill`, which outputs all session logs recorded since the last distillation. The
sub-agent reads through them, identifies new preferences or patterns not already in the soul, and edits the soul file.
After the sub-agent finishes, the main agent runs `leiter soul mark-distilled` to record the timestamp and prevent the
same logs from being reprocessed.

Log cleanup is tied to the distillation timestamp. The `last_distilled` timestamp is only advanced after the sub-agent
successfully finishes — if distillation fails partway through, the timestamp stays put and the same logs are reprocessed
next time. As a consequence, logs processed in a given distillation run persist on disk until the _next_ successful
distillation, at which point they fall behind the updated timestamp and are cleaned up.
