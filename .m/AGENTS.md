# About This Directory

This `.m/` directory is managed by the [`m`](https://github.com/scode/m) tool — an agent instructions manager.

Instruction files (`AGENTS.md`, `CLAUDE.md`) in the parent directory are **generated** from the snippets in
`snippets.d/`. Do not edit them directly — your changes will be overwritten on the next `m build`.

To modify agent instructions, edit or add snippet files in `snippets.d/`, then run `m build` to regenerate the
instruction files.
