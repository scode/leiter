# Leiter Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Implement the `leiter` Rust CLI tool per SPEC.md — a self-training system for Claude Code.

**Architecture:** Thin Rust CLI with clap derive API. All agent intelligence lives outside the binary; the CLI handles structured storage, timestamp management, and context injection. State lives under `~/.leiter/`.

**Tech Stack:** Rust, clap (derive), tracing/tracing-subscriber, thiserror, anyhow, serde/serde_yaml/serde_json, chrono, tempfile

**Process notes:**
- Build bottom-up: low-level modules first, commands on top.
- Each step includes exhaustive tests. Run tests before and after implementation.
- **Docstrings:** Every public type, function, and module should have a doc comment explaining what it does and why it exists. Don't just restate the name — explain the purpose, invariants, or non-obvious design decisions. Internal/private items get doc comments when the intent isn't obvious from context.
- At the end of each step, invoke the `pre-pr-review-swarm` skill (a Claude Code skill, not a binary) with instructions to review uncommitted changes. Address all feedback.
- After completing a step and addressing review feedback, **STOP and wait for the user to continue**. Do not proceed to the next step until explicitly asked.
- If during implementation we discover open gaps in the spec or problems to fix later, append them as known gaps at the bottom of SPEC.md.
- **Tracking:** Mark checkboxes `[x]` in this file as each item is completed. When a step is fully done, also mark its header line with **(DONE)**.

---

## Step 1: Project scaffolding and CLI skeleton **(DONE)**

- [x] `cargo init` with binary target
- [x] Add dependencies to Cargo.toml: clap (derive), tracing, tracing-subscriber, thiserror, anyhow, serde (derive), serde_yaml, serde_json, chrono (serde feature), tempfile
- [x] Add dev-dependency: assert_cmd, predicates, tempfile
- [x] Create `src/main.rs` with clap top-level CLI struct:
  - Global flags: `-v` (DEBUG), `-vv` (TRACE), `-q` (WARN), `-qq` (ERROR), `--log-level=<LEVEL>`
  - `--log-level` takes precedence over `-v`/`-q`
  - Subcommands as empty stubs: `agent-setup`, `context`, `log`, `distill`, `stop-hook`, `soul-upgrade`
- [x] Initialize tracing-subscriber from the resolved log level, output to stderr
- [x] Tests:
  - CLI parses each subcommand without error
  - `-v` sets DEBUG, `-vv` sets TRACE, `-q` sets WARN, `-qq` sets ERROR
  - `--log-level=TRACE` overrides `-q`
  - `--log-level=WARN` overrides `-v`
  - Unknown subcommand errors
  - `leiter log` requires `--session-id`
- [x] Invoke `pre-pr-review-swarm` skill to review uncommitted changes; address feedback
- [x] **STOP** — wait for user before proceeding to next step

## Step 2: State paths and error types **(DONE)**

- [x] Create `src/paths.rs`:
  - `leiter_dir()` → `~/.leiter/`
  - `soul_path()` → `~/.leiter/soul.md`
  - `logs_dir()` → `~/.leiter/logs/`
  - All return `PathBuf`. Use `dirs::home_dir()` (add `dirs` crate) or `$HOME` env var.
- [x] Create `src/errors.rs` with `thiserror` error types:
  - `SoulNotFound` — soul.md does not exist
  - `FrontmatterParse` — invalid YAML frontmatter
  - `LogsDirNotFound` — logs directory does not exist
  - `HomeNotFound` — cannot determine home directory
- [x] Wire modules into `main.rs`
- [x] Tests:
  - Path functions return expected suffixes (`ends_with`)
  - Error types display human-readable messages
  - Error types implement `std::error::Error`
- [x] Invoke `pre-pr-review-swarm` skill to review uncommitted changes; address feedback
- [x] **STOP** — wait for user before proceeding to next step

## Step 3: Frontmatter parsing **(DONE)**

- [x] Create `src/frontmatter.rs`:
  - `SoulFrontmatter` struct: `last_distilled: DateTime<Utc>`, `soul_version: u32`
  - `parse_soul(content: &str) -> Result<(SoulFrontmatter, &str)>` — extracts frontmatter and body from `---`-delimited YAML block
  - `serialize_soul(frontmatter: &SoulFrontmatter, body: &str) -> String` — reassembles the full document
- [x] Tests:
  - Parse valid frontmatter with both fields
  - Parse returns correct body (content after closing `---`)
  - Round-trip: serialize then parse produces identical values
  - Error on missing `---` delimiters
  - Error on missing `last_distilled` field
  - Error on missing `soul_version` field
  - Error on invalid YAML
  - Error on empty input
  - Handles body with its own `---` (e.g., markdown horizontal rules) without breaking
  - Preserves body whitespace exactly
- [x] Invoke `pre-pr-review-swarm` skill to review uncommitted changes; address feedback
- [x] **STOP** — wait for user before proceeding to next step

## Step 4: Log filename parsing and generation **(DONE)**

- [x] Create `src/log_filename.rs`:
  - `generate_log_filename(timestamp: DateTime<Utc>, session_id: &str) -> String` — produces `YYYYMMDDTHHMMSSZ-<session_id>.md`
  - `parse_log_filename(filename: &str) -> Result<(DateTime<Utc>, String)>` — extracts timestamp and session_id from filename
- [x] Tests:
  - Generate produces correct format
  - Parse extracts correct timestamp and session_id
  - Round-trip: generate then parse
  - Parse rejects invalid filenames (no `.md`, bad timestamp, missing session_id)
  - Session IDs with hyphens work
  - Filenames sort lexicographically in chronological order
- [x] Invoke `pre-pr-review-swarm` skill to review uncommitted changes; address feedback
- [x] **STOP** — wait for user before proceeding to next step

## Step 5: Soul template and agent-facing text constants **(DONE)**

- [x] Create `src/templates.rs`:
  - `SOUL_TEMPLATE_VERSION: u32` — current version (start at 1)
  - `SOUL_TEMPLATE: &str` — the initial soul template content (~1 page, section headings for communication style, coding preferences, workflow patterns, tool preferences, etc.)
  - `SOUL_TEMPLATE_CHANGELOG: &[(u32, &str)]` — version changelog entries (just v1 for now)
  - `CONTEXT_PREAMBLE: &str` — the preamble for `leiter context` covering identity, soul file location, when to edit soul, session logging, distillation command, soul upgrade command (all per spec)
  - `STOP_HOOK_PROMPT_TEMPLATE: &str` — the stop hook blocking reason template (with `{session_id}` placeholder)
  - `AGENT_SETUP_INSTRUCTIONS: &str` — the instructions output by `leiter agent-setup` (with the exact hook JSON from the spec)
- [x] Tests:
  - Soul template contains expected section headings
  - Soul template version is > 0
  - Changelog has entry for current version
  - Context preamble contains required literal strings: `~/.leiter/soul.md`, `leiter distill`, `leiter soul-upgrade`
  - Stop hook prompt template contains `{session_id}` placeholder
  - Agent setup instructions contain `leiter context` and `leiter stop-hook`
  - Agent setup instructions contain the exact hook JSON structure from the spec
- [x] Invoke `pre-pr-review-swarm` skill to review uncommitted changes; address feedback
- [x] **STOP** — wait for user before proceeding to next step

## Step 6: `leiter agent-setup` command **(DONE)**

- [x] Create `src/commands/mod.rs` and `src/commands/agent_setup.rs`
- [x] Implement `agent_setup()`:
  1. Create `~/.leiter/` (no-op if exists)
  2. Create `~/.leiter/logs/` (no-op if exists)
  3. If `~/.leiter/soul.md` does not exist: write soul template with frontmatter (`last_distilled: 1970-01-01T00:00:00Z`, `soul_version: <current>`)
  4. If `~/.leiter/soul.md` exists: skip (do not overwrite)
  5. Print agent setup instructions to stdout
  6. If any step fails, output instructions telling the agent to relay the error
- [x] Wire into CLI dispatch in `main.rs`
- [x] Tests (use a temp dir override for `~/.leiter/` — accept a base path parameter or use an env var override for testing):
  - Fresh setup creates all directories and soul.md
  - Soul.md contains expected frontmatter (last_distilled = epoch, soul_version = current)
  - Soul.md body matches template
  - Running twice does not overwrite existing soul.md
  - Running twice still creates missing directories
  - Output contains agent setup instructions
  - Instructions mention `leiter context` and `leiter stop-hook`
- [x] Invoke `pre-pr-review-swarm` skill to review uncommitted changes; address feedback
- [x] **STOP** — wait for user before proceeding to next step

## Step 7: `leiter context` command **(DONE)**

- [x] Create `src/commands/context.rs`
- [x] Implement `context()`:
  1. If `~/.leiter/soul.md` does not exist: output message suggesting `leiter agent-setup`, exit 0
  2. If exists: output preamble, then full contents of soul.md
- [x] Wire into CLI dispatch
- [x] Tests:
  - With existing soul: output starts with preamble, followed by soul content
  - Without soul: output contains "not initialized" and "leiter agent-setup"
  - Preamble contains all required elements (identity, soul path, edit instructions, logging, distill command, upgrade command)
  - Soul content is reproduced verbatim
- [x] Invoke `pre-pr-review-swarm` skill to review uncommitted changes; address feedback
- [x] **STOP** — wait for user before proceeding to next step

## Step 8: `leiter log --session-id <id>` command **(DONE)**

- [x] Create `src/commands/log.rs`
- [x] Implement `log_session(session_id: &str)`:
  1. Read all of stdin to a string
  2. Write to a temp file in the same filesystem as `~/.leiter/logs/` (use `tempfile::NamedTempFile::new_in`)
  3. Capture current UTC timestamp (after stdin read)
  4. Generate filename, atomically rename temp file to final path
  5. Print confirmation with path to stdout
  6. On error: clean up temp file, print error to stderr, exit non-zero
- [x] Wire into CLI dispatch
- [x] Tests:
  - Successful log creates file with correct name format
  - File contains exact stdin content
  - Confirmation message includes the file path
  - Missing logs directory → error exit
  - Session ID appears in filename
  - Timestamp in filename reflects post-stdin time (not pre-stdin)
- [x] Invoke `pre-pr-review-swarm` skill to review uncommitted changes; address feedback
- [x] **STOP** — wait for user before proceeding to next step

## Step 9: `leiter distill` command **(DONE)**

- [x] Create `src/commands/distill.rs`
- [x] Implement `distill()`:
  1. Read and parse `~/.leiter/soul.md` frontmatter for `last_distilled`
  2. Scan `~/.leiter/logs/` for `.md` files
  3. Parse each filename's timestamp; keep those `>= last_distilled`
  4. Sort chronologically
  5. Output each file's content preceded by a header with the filename
  6. If no matching logs: output "no new session logs to process"
- [x] Wire into CLI dispatch
- [x] Tests:
  - No logs at all → "no new session logs" message
  - All logs older than last_distilled → "no new session logs"
  - Log with timestamp == last_distilled is included (inclusive >=)
  - Log with timestamp > last_distilled is included
  - Multiple logs output in chronological order
  - Each log section has filename header
  - Log content reproduced verbatim
  - Unparseable filenames in logs dir are silently skipped
  - Missing soul.md → error
- [x] Invoke `pre-pr-review-swarm` skill to review uncommitted changes; address feedback
- [x] **STOP** — wait for user before proceeding to next step

## Step 10: `leiter stop-hook` command **(DONE)**

- [x] Create `src/commands/stop_hook.rs`
- [x] Implement `stop_hook()`:
  1. Read JSON from stdin
  2. Deserialize to extract `session_id` (string) and `stop_hook_active` (bool)
  3. If `stop_hook_active` is false: output `{"decision":"block","reason":"..."}` with session logging prompt including the session_id
  4. If `stop_hook_active` is true: output nothing (or `{"decision":"allow"}`), exit 0
- [x] Wire into CLI dispatch
- [x] Tests:
  - `stop_hook_active: false` → block decision JSON
  - Block reason contains the session_id
  - Block reason contains `leiter log --session-id`
  - `stop_hook_active: true` → allow (no stdout or allow JSON)
  - Extra fields in input JSON are ignored
  - Missing `session_id` → error
  - Missing `stop_hook_active` → error
  - Invalid JSON → error
- [x] Invoke `pre-pr-review-swarm` skill to review uncommitted changes; address feedback
- [x] **STOP** — wait for user before proceeding to next step

## Step 11: `leiter soul-upgrade` command **(DONE)**

- [x] Create `src/commands/soul_upgrade.rs`
- [x] Implement `soul_upgrade()`:
  1. Read and parse `~/.leiter/soul.md` frontmatter for `soul_version`
  2. Compare with `SOUL_TEMPLATE_VERSION`
  3. If equal: output "soul is up to date"
  4. If outdated: output changelog for versions between user's and current, the full current template, and instructions for the agent to migrate
- [x] Wire into CLI dispatch
- [x] Tests:
  - Soul version == current → up-to-date message
  - Soul version < current → upgrade output includes changelog, template, and instructions
  - Upgrade output includes `soul_version` update instruction
  - Missing soul.md → error
- [x] Invoke `pre-pr-review-swarm` skill to review uncommitted changes; address feedback
- [x] **STOP** — wait for user before proceeding to next step

## Step 12: Integration tests **(DONE)**

- [x] Create `tests/integration.rs` (or `tests/` directory with multiple files)
- [x] Full flow: agent-setup → context → verify soul injected
- [x] Full flow: agent-setup → log a session → distill → verify log appears
- [x] Full flow: distill with last_distilled = epoch → all logs appear
- [x] Full flow: stop-hook with `stop_hook_active: false` → block → log → stop-hook with `stop_hook_active: true` → allow
- [x] Full flow: agent-setup twice → soul not overwritten, no errors
- [x] Verify all stdout/stderr separation (contractual output on stdout, tracing on stderr)
- [x] Invoke `pre-pr-review-swarm` skill to review all uncommitted changes; address feedback
- [x] **STOP** — wait for user before proceeding to next step

## Final

- [ ] Run full test suite one final time
- [ ] Invoke `pre-pr-review-swarm` skill for a final review of the complete codebase
- [ ] Address any remaining feedback
- [ ] Stop and present the work to the human for review and follow-ups
