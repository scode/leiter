# Experimental Codex Support

Codex transcript distillation is currently experimental and disabled by default.

Enable it with:

```sh
leiter config set enable_codex_experimental true
```

Disable it again with:

```sh
leiter config set enable_codex_experimental false
```

Leiter stores this setting in `~/.leiter/leiter.toml`:

```toml
enable_codex_experimental = false
```

## Current behavior

When `enable_codex_experimental = true`:

- `leiter soul distill` still distills Claude logs as usual.
- It also best-effort scans Codex rollout logs under `~/.codex/sessions/**/*.jsonl` and
  `~/.codex/archived_sessions/**/*.jsonl`.
- It never reads Codex SQLite state.
- It never writes, renames, deletes, or otherwise mutates anything under `~/.codex`.
- It only sends a Codex session to the LLM when that session's rollout file changed since the last successful
  `leiter soul mark-distilled`.
- If a session changed, leiter re-reads the whole rollout file and sends the full canonicalized session again.

When `enable_codex_experimental = false`:

- `leiter soul distill` does not read Codex logs or Codex metadata at all.
- `leiter soul mark-distilled` does not read or update Codex metadata at all.

## Watermark metadata

When the experiment is enabled, leiter stores Codex watermarks in `~/.leiter/codex-meta.toml`.

A watermark is leiter's remembered snapshot of a Codex session file at the moment that session was last successfully
marked distilled. On a later `leiter soul distill` run, leiter recomputes the snapshot for each discovered Codex session
and compares it to the committed watermark:

- if the watermark is unchanged, leiter skips that session entirely and does not send it to the LLM again
- if the watermark changed, leiter re-reads the whole session file and sends the full canonicalized session again

The dedupe watermark is this session-level file state:

- `path`
- `size_bytes`
- `mtime_utc`

`codex-meta.toml` uses its own staged/committed watermark maps:

- `pending` is what the most recent `leiter soul distill` run observed
- `committed` is what the most recent successful `leiter soul mark-distilled` accepted

Leiter also records:

- `session_timestamp_utc`
- `latest_event_timestamp_utc`

Those timestamps are for ordering and observability. They are not the primary dedupe rule.

Known gap: Claude distillation state still lives in soul frontmatter while Codex distillation state lives in
`codex-meta.toml`.
