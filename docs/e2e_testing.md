# E2E Testing

> **WARNING:** These tests invoke `claude -p --dangerously-skip-permissions` on the remote host, giving Claude
> unrestricted shell access. The test harness and Claude may make arbitrary changes to the remote account — including
> files, shell environment, installed packages, and Claude Code configuration. **Only run against a dedicated,
> disposable user account with no access to sensitive systems, credentials, or data.**

Leiter's unit, CLI, and integration tests verify the CLI in isolation. The E2E tests go further: they deploy leiter to a
remote host and exercise the full lifecycle through real `claude -p` invocations. This catches breakage at integration
seams — hook firing, skill matching, soul injection, session logging, and the instill/distill flows — that isolated
tests cannot reach.

These tests are inherently flaky because several steps depend on an LLM interpreting prompts and taking the right
actions. They are developed on a best-effort basis and refined as failures are observed during manual testing runs.

## Prerequisites

You need a remote host (or VM) with:

- **SSH access** with non-interactive auth (key-based). The test harness uses `ssh -o ConnectTimeout=10`.
- **Node.js/npm** available on the remote host.
- `~/.local/bin` should be on `PATH` (the harness adds it to `~/.profile` if missing).

The harness installs Claude Code via npm if not already present and probes whether it's authenticated. If not, it
prompts you to press Enter and then launches `claude` on the remote host via `ssh -t` so you can complete the login
flow. After you exit, it re-probes before continuing.

## Setting up a dedicated test user

If you have root access to the remote box, set the host and then run the script below to create a `leiter-e2e` user and
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

`--nocapture` is important — the suite prints step progress and diagnostics during the multi-minute run.

## Environment variables

- `LEITER_E2E_DEST` (required): SSH destination, e.g. `testuser@192.168.1.100`. This is passed directly to `ssh` and
  `scp`.
- `LEITER_E2E_TARGET` (optional): Rust target triple for cross-compilation. If omitted, the harness auto-detects from
  `uname -sm` on the remote host:
  - `Linux x86_64` → `x86_64-unknown-linux-musl`
  - `Linux aarch64` → `aarch64-unknown-linux-musl`
  - `Darwin x86_64` → `x86_64-apple-darwin`
  - `Darwin arm64` → `aarch64-apple-darwin`

## What the suite does

The tests run as a single ordered sequence inside one `#[test]` function. Each step builds on prior state.

**Setup** (runs first, before all steps):

1. Cross-compiles (or natively compiles) leiter for the remote target
2. Installs Claude Code via npm if not already present
3. Probes Claude auth — if not authenticated, prompts you and launches `claude` via `ssh -t` for login
4. Copies the binary to `~/.local/bin/leiter` on the remote host
5. Cleans all prior leiter state (`~/.leiter/`, skill files, hooks in `settings.json`)
6. Runs `leiter claude install`

**Test steps:**

1. **Install verification** — Checks that `soul.md` has the expected frontmatter fields, `logs/` exists, and all 4 skill
   files contain the `SCODE_LEITER_INSTALLED` sentinel.
2. **Agent-driven setup** — Prompts Claude to run `/leiter-setup` and accept all optional features. Verifies
   `settings.json` contains the expected hooks and permissions.
3. **Soul injection** — Asks Claude what leiter is. If the SessionStart hook works, the agent knows about leiter from
   the injected soul.
4. **Session logging** — Runs a trivial prompt, waits briefly, and checks that the log file count increased (SessionEnd
   hook fired).
5. **Instill** — Tells Claude to remember a preference. Verifies the soul file was updated with the preference text.
6. **Distill** — Asks Claude to distill session logs. Verifies `last_distilled` timestamp advanced.
7. **Soul upgrade** — Manually downgrades `soul_version` to 1 via `sed`, then asks Claude to upgrade. Verifies the
   version is restored.

## Reset between runs

Re-running the suite is safe. The setup phase cleans all leiter state before each run:

- Deletes `~/.leiter/` (soul and logs)
- Removes `~/.claude/skills/leiter-*`
- Strips leiter hooks and permissions from `~/.claude/settings.json`
- Runs a fresh `leiter claude install`

Claude Code auth (`~/.claude/credentials.json` or equivalent) is preserved.

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
`musl-cross --with-aarch64`).

When the remote host matches your local platform (e.g., both are `aarch64-apple-darwin`), no extra toolchains are
needed.

You can also override auto-detection by setting `LEITER_E2E_TARGET` to an explicit target triple.

## Troubleshooting

- **SSH auth failures**: Ensure the remote host accepts key-based auth for the user in `LEITER_E2E_DEST`. Test with
  `ssh $LEITER_E2E_DEST 'echo ok'`.
- **Claude hangs or times out**: The harness wraps `claude -p` in `timeout 180`. If Claude hangs (e.g., waiting for
  interactive input), it'll be killed after 3 minutes. Check that `--dangerously-skip-permissions` is working and that
  Claude is authenticated on the remote.
- **Cross-compilation failures**: If `cargo build --target` fails with "can't find crate for `core`", see the
  Cross-compilation section above for one-time setup.
- **Step 2 failures (agent-driven setup)**: This step depends on Claude correctly interpreting the `/leiter-setup` skill
  and modifying `settings.json`. If it fails, check the claude stdout/stderr in the test output — Claude may have hit an
  error or needed more turns.
- **Step 4 failures (session logging)**: The SessionEnd hook fires asynchronously. If the log count doesn't increase,
  try increasing the sleep duration or checking that the hook is actually configured (look at `settings.json`).
