# Design Decisions

## Distillation uses two separate commands and a sub-agent

`leiter soul distill` outputs logs; `leiter soul mark-distilled` bumps the timestamp. They are intentionally separate so
that `mark-distilled` only runs after the agent has successfully processed the distill output — if the agent fails or
the session is interrupted mid-distillation, `last_distilled` is not advanced past unprocessed logs.

The context preamble instructs the agent to delegate distillation to a sub-agent. This serves two purposes: (1) avoid
polluting the main conversation context with potentially large session log output, and (2) the instruction to call
`mark-distilled` after the sub-agent completes is issued fresh in the main context, optimizing agent compliance with the
two-step protocol.
