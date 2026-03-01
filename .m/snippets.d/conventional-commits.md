# Conventional Commits

Use [Conventional Commits](https://www.conventionalcommits.org/) style for PR titles (e.g. `feat: add session replay`,
`fix: handle empty log dir`, `chore: bump dependencies`). This is required for git-cliff changelog generation.

Type must reflect user-visible behavior, not implementation activity. If a change affects the CLI interface or behavior
(command names, flags/options, arguments, output contract, exit codes, documented usage), use `feat`, `fix`, or `perf`
(add `!` when breaking). Do not classify those as `refactor`.

Use `refactor`, `style`, `test`, `chore`, `ci`, `docs`, and `doc` only when behavior is not user-visible.

The description after the type prefix must state the user-visible effect, not the implementation mechanism. For example,
prefer `fix: avoid agent running a wrong leiter binary` over
`fix: clarify that skills must invoke the installed leiter
binary` — the former says what changes for the user, the
latter describes an internal detail.
