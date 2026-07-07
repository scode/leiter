# Design Decisions

## Distillation separates scanning from committing

`leiter soul distill` outputs the new transcripts and stages the watermarks it showed the agent;
`leiter soul mark-distilled` commits them and advances the cutoff. They are intentionally separate so that the commit
only lands after the agent has successfully processed the distill output — if the agent fails or is interrupted
mid-distillation, `last_distilled` and the watermarks are not advanced past unprocessed sessions.

The everyday entry point is `leiter distill`, which drives both halves in one headless process: it scans, invokes the
agent CLI (`claude -p` by default, or the configured `agent_command`) with soul-scoped write grants, and then commits
its own state — but only after a verified successful run (the child exited 0 and received the full prompt on stdin).
Because leiter owns the commit rather than instructing the agent to run `mark-distilled`, the old "the agent forgot to
mark" failure mode is gone. The lower-level `leiter soul distill`/`mark-distilled` pair survives underneath it for
debugging and scan testing, and `leiter distill` reuses that machinery rather than duplicating it.

## Distill output uses data-boundary framing to reduce prompt injection risk

Session transcripts contain assistant messages that may look like instructions ("I'll run `cargo test`", "Let me update
the config"). When the distilling agent reads this output, it could misinterpret historical assistant utterances as
directives for itself to follow.

The distill output mitigates this with two layered defenses:

- An explicit preamble between the soul-writing guidelines and the transcript data, instructing the agent that
  everything that follows is historical data — not instructions to execute.
- XML-like boundary tags (`<session-transcripts>`, `<session file="...">`) wrapping the transcript content. These give
  the model an unambiguous structural signal about what is data vs. what is instruction.

Neither defense is a hard security boundary. A sufficiently instruction-like passage inside the tags can still influence
behavior. The value is in layering: explicit framing + structural markup + the `[user]:`/`[assistant]:` prefixes
together make it significantly harder for transcript content to be misread as directives.

## Epoch system guards all commands against binary/configuration drift

The leiter binary and its effects on agent integration (soul file, managed blocks, skills) are installed independently.
This creates problem cases when they fall out of sync:

- **Binary upgraded, setup not re-run.** The new binary may expect an integration model, commands, or state fields that
  the box does not have yet. Running commands against the old layout could produce wrong output or silently corrupt
  state. The 0.9.0 hookless migration is exactly this case: the hard-epoch `1 → 2` bump routes a pre-0.9.0 box to re-run
  `leiter claude install`.
- **Binary downgraded or not yet upgraded.** The state was stamped by a newer binary. The older binary may misinterpret
  newer fields or produce output incompatible with the state.
- **Corrupt or unreadable state.** If `state.toml` cannot be parsed (or records an unsupported schema version), epochs
  cannot be verified and the binary cannot determine whether it is compatible. Proceeding would be guessing.

The epoch system detects these cases. Every command except `session-end` (and the inert `hook nudge` tombstone) calls a
single shared validation function before doing any work. Hard epoch mismatches and corrupt state block the command
entirely. Soft epoch mismatches are tolerated in either direction: the command proceeds, and the mismatch surfaces as an
advisory line in `leiter status`. (There is no session-start channel to nudge through anymore — hooks are retired — so
status is where a user checks leiter's health.)

The validation is a single shared function used by all commands. This eliminates the risk of individual commands
drifting out of sync on their validation logic.

`session-end` is a migration tombstone and is intentionally exempt — it only copies transcript files to a known
directory on un-migrated boxes. Losing session data is worse than any epoch-related risk, and the operation (appending
files in a known location) is assumed safe across versions.

`leiter claude install` additionally refuses to overwrite epochs when the state is _ahead_ of the binary — that would be
a destructive downgrade — while migrating a behind soft epoch forward on re-run. The hard-epoch bump that a migration
requires is stamped by the migration routine itself, so a migrated box comes out at the current hard epoch rather than
re-tripping the migration message on its next session.
