# Codex support

Leiter can deliver your soul to [Codex](https://developers.openai.com/codex) and distill Codex sessions into the same
soul that Claude Code uses. It is off by default and gated behind a single config key.

## Enabling and disabling

```sh
leiter codex install
```

This sets `codex = true` in `~/.leiter/leiter.toml` and writes the managed soul block into `~/.codex/AGENTS.md` — the
file Codex reads at session start, the same role `~/.claude/CLAUDE.md` plays for Claude Code. The block uses the same
`<!-- SCODE_LEITER_BEGIN -->`/`<!-- SCODE_LEITER_END -->` sentinels and the same inlined-fenced-copy delivery model, so
everything in [how-it-works.md](how-it-works.md) about managed blocks and sync applies to `AGENTS.md` too.

```sh
leiter codex uninstall
```

This removes the `AGENTS.md` block and sets `codex = false`. It works regardless of the current config value, so you can
still clean up the block if you set `codex = false` by hand first.

The config key is `codex`. The older name `enable_codex_experimental` is still accepted when reading `leiter.toml`, and
is rewritten to `codex` the next time leiter saves the file — an existing config that used the experimental name keeps
working and is migrated silently.

## What enabling Codex changes

When `codex = true`, `leiter distill` (and the lower-level `leiter soul distill`) does everything it does for Claude,
plus:

- Best-effort scans Codex rollout logs under `~/.codex/sessions/**/*.jsonl` and `~/.codex/archived_sessions/**/*.jsonl`.
- Sends a Codex session to the distilling agent only when that session's rollout file changed since the last successful
  distill; if it changed, leiter re-reads the whole rollout file and sends the full canonicalized session again.

Leiter never reads Codex's SQLite state, and distillation never writes, renames, or deletes anything under `~/.codex/` —
the one thing leiter writes there is the managed `AGENTS.md` block, via `leiter codex install` and `leiter sync`. A
missing, malformed, or unexpected Codex directory never fails the distill command.

When `codex = false`, distillation does not read Codex logs or consult the Codex watermark tables, and leaves those
tables in `state.toml` unchanged.

## Watermark metadata

Codex distillation watermarks live under `[codex.*]` in `~/.leiter/state.toml`, alongside the `[claude.*]` tables that
serve the Claude scan. A watermark is leiter's remembered snapshot of a session's rollout file at the moment it was last
successfully distilled. On each run leiter recomputes the snapshot for every discovered Codex session and compares:

- unchanged watermark → the session is skipped and not sent to the agent again;
- changed watermark → leiter re-reads the whole file and sends the full canonicalized session again.

The dedupe watermark is this per-session file state:

- `path`
- `size_bytes`
- `mtime_utc`

Each table carries staged and committed maps: `pending` is what the most recent distill observed, and `committed` is
what the most recent verified-successful commit accepted. Leiter also records `session_timestamp_utc` and
`latest_event_timestamp_utc` for ordering and observability; those are not the primary dedupe rule.
