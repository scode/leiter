# CLI Restructuring Plan

**Goal:** Restructure the flat command namespace into three nested subcommand groups:

| Old command              | New command                  |
| ------------------------ | ---------------------------- |
| `leiter context`         | `leiter hook context`        |
| `leiter nudge`           | `leiter hook nudge`          |
| `leiter session-end`     | `leiter hook session-end`    |
| `leiter agent-setup`     | `leiter setup install`       |
| `leiter agent-uninstall` | `leiter setup uninstall`     |
| `leiter instill <text>`  | `leiter soul instill <text>` |
| `leiter distill`         | `leiter soul distill`        |
| `leiter soul-upgrade`    | `leiter soul upgrade`        |

**Process:** One command per step. Each step produces a single PR. After each PR is created, STOP and wait for user
review before continuing.

**Tracking:** As each checkbox item is completed, mark it `[x]`. When an entire step is finished (PR created), also
append **(DONE)** to the step's header line.

**Per-step checklist:**

1. Move the clap subcommand definition into its new group (creating the group if first command in it)
2. Update dispatch in `main.rs`
3. Update all template strings/constants that reference the old command name
4. Update all unit and integration tests
5. Update SPEC.md references
6. Run `dprint fmt`, `cargo fmt`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test`
7. Audit test coverage: grep for all references to the moved command across src/ and tests/, verify every code path that
   produces or checks the command string has a test. Add missing tests if found
8. Run `pre-pr-review-swarm`, address feedback
9. Create a new graphite PR via `scode-graphite` skill
10. **STOP** — wait for user

---

## Step 1: Move `context` → `hook context` **(creates Hook group)** **(DONE)**

- [x] Add `Hook` subcommand group to clap with `HookCommand` enum containing `Context`
- [x] Update dispatch in `main.rs` to route `Hook(HookCommand::Context)` → `commands::context::run`
- [x] Remove old `Context` variant from top-level `Command` enum
- [x] Update template strings: `"leiter context"` → `"leiter hook context"` in `CONTEXT_PREAMBLE`,
      `AGENT_SETUP_INSTRUCTIONS`, `AGENT_UNINSTALL_INSTRUCTIONS`, and any detection strings
- [x] Update all tests referencing `"leiter context"` (templates tests, agent_setup tests, agent_uninstall tests,
      context tests, integration tests)
- [x] Update SPEC.md: all occurrences of `leiter context` → `leiter hook context`
- [x] Run checks (`dprint fmt`, `cargo fmt`, `cargo clippy`, `cargo test`)
- [x] Audit test coverage: grep for all remaining references to the old and new command string across src/ and tests/;
      verify every code path that produces or checks the string is tested. Add missing tests if found
- [x] Run `pre-pr-review-swarm`, address feedback
- [x] Create PR via `scode-graphite`, STOP

## Step 2: Move `nudge` → `hook nudge`

- [ ] Add `Nudge` variant to `HookCommand` enum
- [ ] Update dispatch in `main.rs`
- [ ] Remove old `Nudge` variant from top-level `Command` enum
- [ ] Update template strings: `"leiter nudge"` → `"leiter hook nudge"` in `AGENT_SETUP_INSTRUCTIONS`,
      `AGENT_UNINSTALL_INSTRUCTIONS`, and any detection strings
- [ ] Update all tests referencing `"leiter nudge"`
- [ ] Update SPEC.md: all occurrences of `leiter nudge` → `leiter hook nudge`
- [ ] Run checks (`dprint fmt`, `cargo fmt`, `cargo clippy`, `cargo test`)
- [ ] Audit test coverage: grep for all remaining references to the old and new command string across src/ and tests/;
      verify every code path that produces or checks the string is tested. Add missing tests if found
- [ ] Run `pre-pr-review-swarm`, address feedback
- [ ] Create PR via `scode-graphite`, STOP

## Step 3: Move `session-end` → `hook session-end`

- [ ] Add `SessionEnd` variant to `HookCommand` enum
- [ ] Update dispatch in `main.rs`
- [ ] Remove old `SessionEnd` variant from top-level `Command` enum
- [ ] Update template strings: `"leiter session-end"` → `"leiter hook session-end"` in `AGENT_SETUP_INSTRUCTIONS`,
      `AGENT_UNINSTALL_INSTRUCTIONS`, `CONTEXT_PREAMBLE`, and detection strings
- [ ] Update all tests referencing `"leiter session-end"`
- [ ] Update SPEC.md: all occurrences of `leiter session-end` → `leiter hook session-end`
- [ ] Run checks (`dprint fmt`, `cargo fmt`, `cargo clippy`, `cargo test`)
- [ ] Audit test coverage: grep for all remaining references to the old and new command string across src/ and tests/;
      verify every code path that produces or checks the string is tested. Add missing tests if found
- [ ] Run `pre-pr-review-swarm`, address feedback
- [ ] Create PR via `scode-graphite`, STOP

## Step 4: Move `agent-setup` → `setup install` **(creates Setup group)**

- [ ] Add `Setup` subcommand group to clap with `SetupCommand` enum containing `Install`
- [ ] Update dispatch in `main.rs` to route `Setup(SetupCommand::Install)` → `commands::agent_setup::run`
- [ ] Remove old `AgentSetup` variant from top-level `Command` enum
- [ ] Update template strings: `"leiter agent-setup"` → `"leiter setup install"` in `CONTEXT_PREAMBLE`,
      `AGENT_UNINSTALL_INSTRUCTIONS`, context.rs error/nudge messages, and agent_setup.rs error output
- [ ] Update all tests referencing `"leiter agent-setup"`
- [ ] Update SPEC.md: all occurrences of `leiter agent-setup` → `leiter setup install`, rename the section heading from
      `leiter agent-setup` to `leiter setup install`
- [ ] Run checks (`dprint fmt`, `cargo fmt`, `cargo clippy`, `cargo test`)
- [ ] Audit test coverage: grep for all remaining references to the old and new command string across src/ and tests/;
      verify every code path that produces or checks the string is tested. Add missing tests if found
- [ ] Run `pre-pr-review-swarm`, address feedback
- [ ] Create PR via `scode-graphite`, STOP

## Step 5: Move `agent-uninstall` → `setup uninstall`

- [ ] Add `Uninstall` variant to `SetupCommand` enum
- [ ] Update dispatch in `main.rs`
- [ ] Remove old `AgentUninstall` variant from top-level `Command` enum
- [ ] Update template strings: any reference to `"leiter agent-uninstall"` (if present)
- [ ] Update all tests referencing `"leiter agent-uninstall"`
- [ ] Update SPEC.md: all occurrences of `leiter agent-uninstall` → `leiter setup uninstall`, rename the section heading
- [ ] Run checks (`dprint fmt`, `cargo fmt`, `cargo clippy`, `cargo test`)
- [ ] Audit test coverage: grep for all remaining references to the old and new command string across src/ and tests/;
      verify every code path that produces or checks the string is tested. Add missing tests if found
- [ ] Run `pre-pr-review-swarm`, address feedback
- [ ] Create PR via `scode-graphite`, STOP

## Step 6: Move `instill` → `soul instill` **(creates Soul group)**

- [ ] Add `Soul` subcommand group to clap with `SoulCommand` enum containing `Instill { text: String }`
- [ ] Update dispatch in `main.rs` to route `Soul(SoulCommand::Instill { text })` → `commands::instill::run`
- [ ] Remove old `Instill` variant from top-level `Command` enum
- [ ] Update template strings: `"leiter instill"` → `"leiter soul instill"` in `CONTEXT_PREAMBLE`
- [ ] Update all tests referencing `"leiter instill"`
- [ ] Update SPEC.md: all occurrences of `leiter instill` → `leiter soul instill`, rename section heading
- [ ] Run checks (`dprint fmt`, `cargo fmt`, `cargo clippy`, `cargo test`)
- [ ] Audit test coverage: grep for all remaining references to the old and new command string across src/ and tests/;
      verify every code path that produces or checks the string is tested. Add missing tests if found
- [ ] Run `pre-pr-review-swarm`, address feedback
- [ ] Create PR via `scode-graphite`, STOP

## Step 7: Move `distill` → `soul distill`

- [ ] Add `Distill { dry_run: bool }` variant to `SoulCommand` enum
- [ ] Update dispatch in `main.rs`
- [ ] Remove old `Distill` variant from top-level `Command` enum
- [ ] Update template strings: `"leiter distill"` → `"leiter soul distill"` in `CONTEXT_PREAMBLE`
- [ ] Update all tests referencing `"leiter distill"`
- [ ] Update SPEC.md: all occurrences of `leiter distill` → `leiter soul distill`, rename section heading
- [ ] Run checks (`dprint fmt`, `cargo fmt`, `cargo clippy`, `cargo test`)
- [ ] Audit test coverage: grep for all remaining references to the old and new command string across src/ and tests/;
      verify every code path that produces or checks the string is tested. Add missing tests if found
- [ ] Run `pre-pr-review-swarm`, address feedback
- [ ] Create PR via `scode-graphite`, STOP

## Step 8: Move `soul-upgrade` → `soul upgrade`

- [ ] Add `Upgrade` variant to `SoulCommand` enum
- [ ] Update dispatch in `main.rs`
- [ ] Remove old `SoulUpgrade` variant from top-level `Command` enum
- [ ] Update template strings: `"leiter soul-upgrade"` → `"leiter soul upgrade"` in `CONTEXT_PREAMBLE`
- [ ] Update all tests referencing `"leiter soul-upgrade"`
- [ ] Update SPEC.md: all occurrences of `leiter soul-upgrade` → `leiter soul upgrade`, rename section heading
- [ ] Run checks (`dprint fmt`, `cargo fmt`, `cargo clippy`, `cargo test`)
- [ ] Audit test coverage: grep for all remaining references to the old and new command string across src/ and tests/;
      verify every code path that produces or checks the string is tested. Add missing tests if found
- [ ] Run `pre-pr-review-swarm`, address feedback
- [ ] Create PR via `scode-graphite`, STOP

## Step 9: Cleanup

- [ ] Verify `Command` enum is empty (or contains only the three group variants) — remove if appropriate
- [ ] Hide `Hook` subcommand group from `--help` output (clap `hide = true` or similar) since hook commands are not
      user-facing
- [ ] Review top-level `--help` output for clarity
- [ ] Final full test run
- [ ] Audit test coverage: verify CLI integration tests cover the new nested subcommand routing (e.g.,
      `leiter hook context`, `leiter setup install`, `leiter soul distill` all parse correctly). Add missing tests if
      found
- [ ] Run `pre-pr-review-swarm`, address feedback
- [ ] Create PR via `scode-graphite`, STOP
