# E2E Testing

> **WARNING:** These tests invoke `claude -p --model opus --dangerously-skip-permissions` on the remote host, giving
> Claude unrestricted shell access. The test harness and Claude may make arbitrary changes to the remote account —
> files, shell environment, installed packages, Claude Code configuration. **Only run against a dedicated, disposable
> user account with no access to sensitive systems, credentials, or data.**

Leiter's unit, CLI, and integration tests verify the CLI in isolation. The E2E tests go further: they deploy leiter to a
remote host and exercise the hookless lifecycle through real `claude -p --model opus` invocations. This catches breakage
at integration seams — managed-block soul delivery, skill matching, Claude Code transcript scanning, and the instill,
distill, sync, status, epoch, tombstone, and legacy-migration flows — that isolated tests cannot reach.

These tests are inherently flaky because several steps depend on an LLM interpreting prompts and taking the right
actions. The deterministic assertions are intentionally strict around the LLM-dependent steps so failures usually show
which side broke: leiter state, managed blocks, or Claude's routing.

## Prerequisites

You need a remote host (or VM) with SSH key-based auth (the harness uses `ssh -o ConnectTimeout=10`), Node.js/npm
available, and `~/.local/bin` on `PATH` (the harness adds it to `~/.profile` if missing).

The harness installs Claude Code via npm if not already present and probes whether it's authenticated. If not, it
prompts you to press Enter and then launches `claude --model opus` on the remote host via `ssh -t` so you can complete
the login flow. After you exit, it re-probes before continuing.

## Setting up a dedicated test user

If you have root access to the remote box, set the host and run the script below to create a `leiter-e2e` user and
authorize your local SSH key:

```bash
E2E_HOST=192.168.1.100
```

```bash
ssh root@$E2E_HOST 'useradd -m -s /bin/bash leiter-e2e && mkdir -p ~leiter-e2e/.ssh && chmod 700 ~leiter-e2e/.ssh' \
  && scp ~/.ssh/id_ed25519.pub root@$E2E_HOST:~leiter-e2e/.ssh/authorized_keys \
  && ssh root@$E2E_HOST 'chown -R leiter-e2e:leiter-e2e ~leiter-e2e/.ssh && chmod 600 ~leiter-e2e/.ssh/authorized_keys'
```

Replace `id_ed25519.pub` with your key if it differs. After this, you still need to install Node.js on the remote host.
Claude Code installation and authentication are handled by the test harness.

## Running the tests

```bash
LEITER_E2E_DEST=leiter-e2e@192.168.1.100 cargo test --features e2e e2e -- --nocapture
```

The suite is compiled only when the `e2e` cargo feature is enabled. `--nocapture` is important — the suite prints step
progress and diagnostics during the multi-minute run.

Every Claude invocation made by the harness uses `--model opus`, including auth probes and the `leiter distill`
`agent_command` written during the suite. This is intentional: E2E testing must not consume whatever model happens to be
the Claude Code default on the remote host.

## Environment variables

`LEITER_E2E_DEST` (required) is the SSH destination, e.g. `testuser@192.168.1.100`, passed directly to `ssh` and `scp`.

`LEITER_E2E_TARGET` (optional) is a Rust target triple for cross-compilation. If omitted, the harness auto-detects from
`uname -sm` on the remote host: `Linux x86_64` maps to `x86_64-unknown-linux-musl`, `Linux aarch64` to
`aarch64-unknown-linux-musl`, `Darwin x86_64` to `x86_64-apple-darwin`, and `Darwin arm64` to `aarch64-apple-darwin`.

## What the suite does

The tests run as a single ordered sequence inside one `#[test]` function. Each step builds on prior state.

**Setup** runs first: cross-compile (or natively compile) leiter for the remote target, install Claude Code via npm if
needed, probe Claude auth (prompting you to log in via `ssh -t` if not authenticated), copy the binary to
`~/.local/bin/leiter`, clean prior leiter state (`~/.leiter/`, old and new leiter skill files, and leiter hooks in
`settings.json`), and run `leiter claude install`.

**Test steps**, in order:

1. **Install verification** — checks for pure `soul.md`, `state.toml`, no fresh `logs/`, exactly one `skills/leiter/`
   directory, the managed `CLAUDE.md` block, and no clean-setup leiter hooks in `settings.json`.
2. **Soul delivery through the block** — writes a marker into the soul, runs `leiter sync`, then asks a real Claude
   session to quote the marker from startup context.
3. **Instill** — asks Claude to remember an exact marker through the consolidated skill, then verifies `soul.md` and the
   synced `CLAUDE.md` block contain it, while Codex remains off and has no managed `AGENTS.md` block.
4. **Headless distill** — writes an `agent_command` using `claude -p --model opus --allowedTools ...`, runs
   `leiter distill`, and verifies `last_distilled` and `[claude.committed]` advanced from the external
   `~/.claude/projects` scan. It also verifies the distill agent did not drop the instilled soul marker.
5. **Status** — verifies `leiter status` exits 0, reports a low undistilled Claude count, and says the `CLAUDE.md` block
   is in sync.
6. **Clobber guard** — hand-edits inside the managed block, verifies `leiter sync` refuses with `--force` guidance
   without deleting the hand edit, then verifies `leiter sync --force` heals it.
7. **Soul upgrade** — downgrades `soul_version`, asks Claude to run the upgrade flow, and verifies
   `leiter soul mark-upgraded` restored the current version in `state.toml`.
8. **Hard epoch mismatch** — writes a higher hard epoch and verifies validating leiter commands fail with the
   binary-outdated message.
9. **SessionEnd tombstone exemption** — directly invokes `leiter hook session-end` under a hard-epoch mismatch and
   verifies it recreates `logs/` and archives the transcript.
10. **Soft epoch advisory** — writes a behind soft epoch and verifies `leiter status` reports the non-blocking setup
    advisory.
11. **Legacy migration** — constructs the old frontmatter soul, `codex-meta.toml`, legacy config key, old leiter skill,
    legacy logs, and mixed hook settings; runs `leiter claude install`; and verifies state migration, soul stripping,
    Codex watermark absorption, config normalization, managed blocks, hook-removal instructions, byte-unchanged
    `settings.json`, old-skill cleanup, and legacy-log drain. This step backs up any existing remote
    `~/.claude/settings.json` before writing the hook fixture, restores it after the step succeeds, and removes the
    fixture-created Codex managed block.
12. **Uninstall/reinstall convergence** — verifies `leiter claude uninstall` removes the managed `CLAUDE.md` block and
    skill while preserving other content and `~/.leiter/`, verifies `leiter codex uninstall` removes only the
    `AGENTS.md` block and persists `codex = false`, then verifies `leiter claude install` converges back.
13. **Retention warning** — creates controlled Claude transcripts, backdates one undistilled file beyond
    `retention_warn_days`, and verifies `leiter status` names the at-risk session while fresh transcripts do not warn.
14. **Staleness and opportunistic heal** — edits `soul.md` directly, verifies `leiter status` reports `CLAUDE.md` stale
    without repairing it, then runs a participating mutating command and verifies the block is back in sync.
15. **Error paths** — corrupts `state.toml` and verifies `leiter status` fails with the corrupt-state recovery message,
    then points `agent_command` at a nonexistent binary and verifies `leiter distill` fails without advancing
    `last_distilled`.

## Not covered (manual battery scenarios)

- **Full Codex session flow** — requires authenticated Codex plus a real Codex session. Keep this in the
  `lore/hookless-migration.md` battery instead of making the remote suite depend on another interactive auth surface.
- **Procrastinating-user multi-session flows** — depend on an old still-hooked binary and human delay across sessions.
  The deterministic pieces are covered here; the end-to-end behavior belongs in the manual battery.
- **In-flight/resumed session semantics** — require observing how Claude Code resumes or continues an existing session
  after migration. That is LLM- and product-behavior observational, not a stable CLI contract.
- **Prompt-injection posture** — needs adversarial transcript review and model behavior observation. The suite verifies
  the data-boundary plumbing, while the broader posture stays in the manual battery in `lore/hookless-migration.md`.

## Reset between runs

Re-running the suite is safe for a disposable account. The setup phase deletes `~/.leiter/`, removes both the current
`~/.claude/skills/leiter/` skill and old `~/.claude/skills/leiter-*` skills, strips leiter hooks and permissions from
`~/.claude/settings.json`, snapshots the stripped settings to a private `~/.leiter-e2e-settings.*` file long enough to
verify install did not edit it, and runs a fresh `leiter claude install`. Claude Code auth (`~/.claude/credentials.json`
or equivalent) is preserved.

Step 11 deliberately writes a legacy `~/.claude/settings.json` containing a live `SessionStart` hook command
(`leiter hook context`) and a decoy hook, creates a legacy `~/.leiter/logs/` directory, creates an old
`~/.claude/skills/leiter-distill/` skill, writes `~/.leiter/codex-meta.toml`, enables the legacy Codex config alias, and
materializes a managed `~/.codex/AGENTS.md` block during migration. On the success path it restores the prior
`settings.json` (or removes the fixture file if there was none), drains and removes the legacy logs directory, verifies
the old skill was removed, and removes the fixture-created Codex managed block with `leiter codex uninstall`.

## Cross-compilation (macOS to Linux)

The most common setup is developing on macOS and running E2E tests against a Linux remote. The harness auto-detects the
remote as `x86_64-unknown-linux-musl` (or `aarch64-unknown-linux-musl`), but you need the Rust target and a musl
cross-linker installed locally.

One-time setup:

```bash
rustup target add x86_64-unknown-linux-musl
brew install filosottile/musl-cross/musl-cross
```

Then configure the linker for cargo (add to `~/.cargo/config.toml`):

```toml
[target.x86_64-unknown-linux-musl]
linker = "x86_64-linux-musl-gcc"
```

After that, `cargo test --features e2e e2e` will cross-compile automatically.

For `aarch64` Linux remotes, substitute `aarch64-unknown-linux-musl` and `aarch64-linux-musl-gcc` (install
`musl-cross --with-aarch64`). When the remote host matches your local platform (e.g., both `aarch64-apple-darwin`), no
extra toolchains are needed. You can also override auto-detection by setting `LEITER_E2E_TARGET` explicitly.

## Troubleshooting

**SSH auth failures.** Ensure the remote host accepts key-based auth for the user in `LEITER_E2E_DEST`. Test with
`ssh $LEITER_E2E_DEST 'echo ok'`.

**Claude hangs or times out.** The harness wraps `claude -p --model opus` in `timeout 180`. If Claude hangs (e.g.,
waiting for interactive input), it gets killed after 3 minutes. Check that `--dangerously-skip-permissions` is working
and that Claude is authenticated on the remote.

**Cross-compilation failures.** If `cargo build --target` fails with "can't find crate for `core`", see the
cross-compilation section above for one-time setup.

**Soul-delivery failures.** The suite no longer uses SessionStart hooks. If Claude cannot quote the marker, inspect
`~/.claude/CLAUDE.md` on the remote host and the `leiter sync` output first.

**Instill or upgrade failures.** These depend on Claude matching the consolidated `leiter` skill and following its CLI
instructions. Check the Claude stdout/stderr in the test output before changing deterministic assertions.

**Distill failures.** The suite writes `agent_command` explicitly so the headless agent uses `--model opus` and only
gets soul-scoped file grants. If distill fails, check both leiter stderr and Claude stderr; a permission issue here
usually means the `--allowedTools` grant string no longer matches Claude Code's permission syntax.
