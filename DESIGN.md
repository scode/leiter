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
