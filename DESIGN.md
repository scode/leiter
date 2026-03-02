# Design Decisions

## Distillation uses two separate commands and a sub-agent

`leiter soul distill` outputs logs; `leiter soul mark-distilled` bumps the timestamp. They are intentionally separate so
that `mark-distilled` only runs after the agent has successfully processed the distill output — if the agent fails or
the session is interrupted mid-distillation, `last_distilled` is not advanced past unprocessed logs.

The context preamble instructs the agent to delegate distillation to a sub-agent. This serves two purposes: (1) avoid
polluting the main conversation context with potentially large session log output, and (2) the instruction to call
`mark-distilled` after the sub-agent completes is issued fresh in the main context, optimizing agent compliance with the
two-step protocol.

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

The leiter binary and its effects on agent configuration (soul file, hooks, skills) are installed independently. This
creates problem cases when they fall out of sync:

- **Binary upgraded, setup not re-run.** The new binary may expect hooks, commands, or soul fields that don't exist yet.
  Running commands against the old configuration could produce wrong output or silently corrupt state.
- **Binary downgraded or not yet upgraded.** The soul was stamped by a newer binary. The older binary may misinterpret
  newer fields or produce output incompatible with the soul's state.
- **Corrupt frontmatter.** If the soul's YAML frontmatter is unparseable, epochs cannot be verified and the binary
  cannot determine whether it is compatible. Proceeding would be guessing.

The epoch system detects all three cases. Every command except `session-end` calls a single shared validation function
(`validate_soul`) before doing any work. Hard epoch mismatches and corrupt frontmatter block the command entirely. Soft
epoch mismatches produce a nudge but allow the command to proceed.

The validation is a single shared function used by all commands. This eliminates the risk of individual commands
drifting out of sync on their validation logic.

`session-end` is intentionally exempt — it only copies transcript files to a known directory. Losing session data is
worse than any epoch-related risk, and the operation (appending files in a known location) is assumed safe across
versions.

`leiter claude install` additionally refuses to overwrite epochs when they differ from the binary's values. Since no
binary has been released with epochs other than 1, any soul with a different epoch must have been created by a future
binary. Overwriting would be a destructive downgrade. Future binaries that bump epochs will ship with their own
migration logic in `install`.
