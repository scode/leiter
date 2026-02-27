# Contributing

## Commit Messages and PR Titles

All commit messages and PR titles must follow [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/).

Allowed types: `feat`, `fix`, `docs`, `doc`, `perf`, `refactor`, `style`, `test`, `chore`, `ci`, `revert`.

Scope is optional.

PR title enforcement is implemented in `.github/workflows/conventional-commit-pr-title.yml`.

## Changelog

The changelog is generated with [git-cliff](https://git-cliff.org/) from Conventional Commit messages and lives at
`CHANGELOG.md` in the repository root.

## Releasing

Ask the user what version to bump to. Read the current version from `Cargo.toml`, then offer three options showing the
resulting version for each:

- Bump bugfix (e.g. 0.1.0 → 0.1.1)
- Bump minor, reset bugfix (e.g. 0.1.1 → 0.2.0)
- Bump major, reset minor+bugfix (e.g. 0.2.0 → 1.0.0)

Then proceed:

1. Ensure you're on a fresh main with a clean working copy: `gt sync --all`, `gt checkout main`, then verify
   `git status` shows no uncommitted or untracked changes. Abort if dirty.
2. Set the version in `Cargo.toml`.
3. Refresh the lockfile: `cargo update --workspace`
4. Validate lockfile consistency: `cargo metadata --format-version 1 --locked > /dev/null`
5. Generate the changelog: `git-cliff --tag "v$VERSION" -o CHANGELOG.md`
6. Run `dprint fmt` to fix any formatting issues in the generated changelog.
7. Verify the changelog heading exists: `rg -n "^## \[$VERSION\]" CHANGELOG.md`
8. Create a release PR with commit message `chore: release $VERSION`. The PR must include `Cargo.toml`, `Cargo.lock`,
   and `CHANGELOG.md` (CHANGELOG.md will be untracked on first release — `gt add` it before committing).
9. **Stop and explicitly ask the user for confirmation before merging and tagging.** Do not silently wait — tell the
   user you are ready to merge and tag, and ask them to confirm.
10. Merge the PR: `gh pr merge <number> --squash`
11. Sync and checkout main: `gt sync --all`, `gt checkout main`.
12. Tag the merge commit and push: `git tag v$VERSION && git push origin v$VERSION`
13. Watch the Release workflow: `gh run watch <run-id>`. Confirm it succeeds (dist plan, release-plan tests, artifact
    builds, Homebrew formula publish).
