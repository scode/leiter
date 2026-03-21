# Usage

Once leiter is set up, session context injection and logging happen automatically. This page covers the three ways the
soul gets updated and how to work with the soul file directly.

## Teaching preferences (instill)

The fastest way to teach leiter is to tell the agent to "instill" a preference. Other trigger words like "remember",
"always", "never", and "learn" also work:

- "Instill that I prefer snake_case for Rust functions"
- "Never add emojis to commit messages"
- "Remember that I prefer explicit error handling over unwrap"
- "Always run clippy before considering work done"

The agent invokes the `/leiter-instill` skill, which provides writing guidelines and tells the agent to edit
`~/.leiter/soul.md`. The preference takes effect immediately in the current session and all future sessions.

You can also instill broader patterns:

- "Remember that I prefer prose over bullet lists in documentation"
- "Learn that I use Graphite for branch management, not raw git"
- "Always squash into a single commit per feature branch"

The agent places each preference in the appropriate section of the soul file and resolves conflicts with existing
entries (newer observations replace older ones).

## Distillation

Distillation processes accumulated session transcripts and extracts patterns you have not explicitly taught. It catches
things like: you consistently prefer a certain error handling style, you always structure tests a particular way, or you
tend to ask for specific kinds of code review.

### Manual distillation

Run `/leiter-distill` in a Claude Code session. This spawns a sub-agent to read through your recent transcripts, in a
separate context to keep raw transcript data out of your main session. You can also just say "distill" or similar
natural language — the agent auto-matches the skill.

### Automatic distillation

If you opted into auto-distillation during `/leiter-setup`, the agent automatically runs distillation at session start
whenever undistilled logs are older than 4 hours. It will briefly let you know it's doing so.

### Nudges

Without auto-distillation, leiter nudges you after the first turn when undistilled logs are older than 24 hours. The
agent mentions that logs are available for distillation and suggests running it. You can do it then or ignore it.

## Soul upgrades

When you update the leiter binary, the soul template may have changed (new sections, reorganized categories, etc.). The
agent tells you when an upgrade is available — just follow its suggestion. You can also trigger it manually by running
`/leiter-soul-upgrade` (or saying "upgrade the leiter soul" in natural language). The agent runs `leiter soul upgrade`,
gets the migration instructions, and restructures your soul while preserving all learned preferences.

## Viewing the soul

Run `/leiter-soul` to see the current contents of your soul file. The agent displays the learned preferences verbatim,
without the internal frontmatter.

## The soul file

The soul lives at `~/.leiter/soul.md`. It is a markdown file you can read and edit directly — there is nothing magic
about it. The agent edits it with the same tools it uses for any other file.

The file has YAML frontmatter (between the `---` delimiters at the top) followed by markdown content containing your
learned preferences. The frontmatter is managed by the CLI — do not edit it, as corrupting it will break leiter. The
body below the frontmatter is yours — the agent writes to it through instill and distillation, and you can also edit it
directly if you want to reorganize, remove entries, or add things by hand. Changes take effect on the next session.
