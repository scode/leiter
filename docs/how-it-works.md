# How It Works

Leiter is built on a simple idea: the Claude agent does all the thinking, and the `leiter` CLI handles plumbing. The CLI
never calls a model API directly. It manages files, timestamps, and the managed blocks that deliver your soul — the
agent reads, writes, and decides what to remember. The one place leiter orchestrates a model is `leiter distill`, which
shells out to a headless agent CLI (`claude -p`) to do the soul-editing; even there, leiter talks to a harness that
already owns authentication rather than to any API itself.

This page is the conceptual overview. [SPEC.md](../SPEC.md) at the repo root is the authoritative contract for every
behavior described here.

## The learning loop

Leiter has two inputs that feed a single output (the soul file):

1. **Direct teaching** — you tell the agent to remember something, it writes it to the soul immediately.
2. **Distillation** — your accumulated session transcripts are periodically processed, and any new patterns are folded
   into the soul.

The soul is then delivered into every future session, so the agent starts with all your preferences loaded.

## Soul delivery: managed blocks

Leiter is hookless. Rather than injecting the soul with a hook, it materializes the soul into a file each harness
already reads at session start: `~/.claude/CLAUDE.md` for Claude Code, and `~/.codex/AGENTS.md` for Codex when enabled.
Inside that file, leiter owns a **managed block** — a span delimited by `<!-- SCODE_LEITER_BEGIN -->` and
`<!-- SCODE_LEITER_END -->` sentinel comments. Everything outside the span is left byte-for-byte untouched; only the
span between the sentinels is leiter's. The block opens with a short preamble (leiter's identity, the soul path, and how
to act on your language) and then inlines the soul body verbatim inside a code fence.

The soul is **inlined, not imported**, and that is a deliberate security choice rather than a convenience. Claude Code
supports `@path` imports in `CLAUDE.md`, which would be a tempting way to always deliver the current soul without
keeping a copy. But imports nest — an import inside an imported file also resolves (verified empirically against a
2.1.20x-era CLI). The soul is agent-writable, so an import-delivered soul would let anything that can write `soul.md`
pull arbitrary readable files into every session by planting an `@import`. Imports inside a code fence, by contrast, are
inert — so the inlined fenced copy is safe, and incidental `@`-tokens in soul prose (decorators, email addresses) are
never interpreted as imports either. Codex has no import syntax at all, so inlining is the only option there anyway, and
using one delivery model for both harnesses keeps the sync handling uniform.

The fence length is computed at write time: always at least three backticks, and always strictly longer than the longest
run of backticks anywhere in the soul body, so a soul that itself contains fenced code blocks cannot terminate the fence
early.

## Keeping the copy current: sync and the clobber guard

Because the block holds a materialized copy of the soul, the copy has to be refreshed whenever the soul changes.
`leiter sync` re-materializes the block from the current soul. The instill and upgrade flows end by running it, and the
state-mutating commands enumerated in the spec (mark-distilled, mark-upgraded, config set, codex install, and distill's
staging path) opportunistically re-sync a stale block as a side effect, so the gap between "agent edited the soul" and
"block reflects it" stays small.

Leiter records two SHA-256 hashes per target in `state.toml`: the soul body it last materialized into the block (to
detect staleness) and the full block content it last wrote (to detect hand edits). If you edit inside the managed span
yourself, the on-disk block no longer matches the recorded hash — the **clobber guard** — and `leiter sync` warns and
refuses that target rather than overwriting your edit. Rerun with `--force` to accept the re-materialized copy. A
refusal from the opportunistic re-sync only ever warns and never fails the host command; an explicit `leiter sync`
treats a refused target as a failure and exits non-zero, pointing at `--force`. Read-only surfaces (`leiter status`, any
`--dry-run`) report block state without healing it, so their output reflects what is actually on disk.

## Distillation: reading transcripts in place

Distillation used to depend on a SessionEnd hook copying each transcript into `~/.leiter/logs/`. Hookless leiter reads
Claude Code's own session store directly instead. Claude Code stores one JSONL transcript per session under
`~/.claude/projects/`, appended live while the session runs; `leiter distill` walks that tree, canonicalizes each
transcript down to user-visible content, and hands the new ones to the distilling agent. There is no copy step.

The scan is careful about which sessions count as new, using two mechanisms:

- **Per-session watermarks.** For each discovered session, leiter records `(path, size_bytes, mtime_utc)` in
  `state.toml`. On the next run, an unchanged session is skipped entirely; a changed or new one is re-read and emitted
  in full. Emitting the whole session on any change is what handles a session that was distilled and later resumed —
  Claude Code materializes a resumed session as a new file duplicating the prior history, so re-emitting in full keeps
  the agent's view complete.
- **The `last_distilled` floor.** A discovered session with no watermark whose file is older than `last_distilled` is
  treated as already distilled and skipped. This keeps the first hookless scan from re-ingesting the whole retention
  window of sessions the soul already learned from — learning starts at install (or at the last distill), not at the
  beginning of Claude Code's history.

On a box migrated from the hook era, the leftover `~/.leiter/logs/` files are drained alongside the in-place scan and
deduplicated by session id, then the directory is removed. A fresh install never has that directory.

Codex sessions, when enabled, are scanned the same way from Codex's rollout directories, with their own watermark
tables. See [codex.md](codex.md).

## Distillation: the headless lifecycle

`leiter distill` runs a gather → agent → verified-commit cycle:

1. **Gather.** Scan the session stores, apply the watermarks and the `last_distilled` floor, and stage what this run
   will show the agent as `pending` watermarks in `state.toml`. Compose a prompt from the soul-writing guidelines, a
   data-boundary preamble wrapping the transcripts (they are historical data, never directives — dropping this boundary
   would hand transcript prompt-injection a headless agent with soul write access), and the soul path. Leiter excludes
   its own headless agent transcripts from later scans using a sentinel at the start of the child prompt, so distill
   does not feed on itself.
2. **Agent.** Invoke the agent CLI headlessly, piping the prompt on stdin (transcript batches routinely exceed
   `ARG_MAX`). By default this is `claude -p` with `Read`/`Edit`/`Write` grants added for the soul file — write access
   is scoped to the soul; reads fall under the harness's normal working-directory rules; a configured `agent_command`
   replaces the whole command line. The agent edits only the soul and ends with a one-paragraph summary.
3. **Verified commit.** Only on a genuinely successful run — the child exited 0 **and** the full prompt reached its
   stdin — does leiter commit its bookkeeping: advance `last_distilled` to the scan-start time, promote the `pending`
   watermarks to `committed`, and re-sync the managed block. A failure commits nothing and leaves the pending watermarks
   to be restaged from current reality on the next run.

Committing only after a verified success removes the old "the agent forgot to run the mark step" failure mode:
`leiter distill` marks its own state, and never delegates that to the LLM. The lower-level `leiter soul distill` and
`leiter soul mark-distilled` commands remain for debugging and scan testing, but the everyday path is `leiter distill`.

## The soul file

The soul is a pure markdown file at `~/.leiter/soul.md` containing your learned preferences organized by category. The
agent edits it directly with its standard file tools. It has no frontmatter and no embedded metadata — the file the
agent reads is exactly the preferences you care about.

## state.toml: the single metadata home

All CLI-managed metadata lives in one place, `~/.leiter/state.toml`: the setup epochs, `soul_version`, `last_distilled`,
the per-target managed-block sync hashes, and the Claude and Codex distillation watermarks. Leiter owns this file; the
agent must never edit it. Keeping metadata out of the soul means a malformed soul is at worst a malformed preferences
document — the agent can no longer corrupt leiter's bookkeeping by editing the file it is supposed to edit. Writes are
atomic (temp file plus rename), so a crash mid-write never leaves a torn state file.

See [SPEC.md](../SPEC.md) for the exact shape of `state.toml` and the full contract of every command.
