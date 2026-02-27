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

1. Set the version in `Cargo.toml`.
2. Refresh the lockfile: `cargo update --workspace`
3. Validate lockfile consistency: `cargo metadata --format-version 1 --locked > /dev/null`
4. Generate the changelog entry:
   ```sh
   VERSION=X.Y.Z
   git-cliff --tag "v$VERSION" -o CHANGELOG.md
   ```
5. Verify the changelog heading exists: `rg -n "^## \[$VERSION\]" CHANGELOG.md`
6. Commit `Cargo.toml`, `Cargo.lock`, and `CHANGELOG.md` in the release PR.
7. Merge the PR, tag the merge commit as `vX.Y.Z`, and push the tag.
8. Confirm the GitHub Actions release pipeline succeeds (dist plan, release-plan tests, artifact builds, Homebrew
   formula publish).
9. Verify: `brew install scode/dist-tap/leiter`
