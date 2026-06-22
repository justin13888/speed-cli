# Contributing

## Development setup

This project uses [mise](https://mise.jdx.dev) to manage tooling and tasks, and
[hk](https://hk.jdx.dev) for git hooks. Rust itself is pinned in
`rust-toolchain.toml` (stable) and managed by rustup.

```bash
mise install      # install pinned tools (nextest, machete, typos, hk, pkl)
mise run setup    # install the git hooks (hk install)
```

## Common commands

All common commands are mise tasks (run `mise tasks` to list them):

| Command | What it does |
|---|---|
| `mise run build` | `cargo build --all-targets` |
| `mise run test` | `cargo nextest run` + doctests |
| `mise run fmt` | format all code |
| `mise run lint` | `cargo clippy --all-targets -- -D warnings` |
| `mise run lint-fix` | apply clippy autofixes |
| `mise run check` | fmt-check + lint + typos + machete (the static gate) |
| `mise run ci` | `check` + `test` (what CI runs) |
| `mise run bench` | criterion benchmarks |

## Git hooks

Installed by `mise run setup`:

- **pre-commit** auto-fixes: runs `cargo clippy --fix` then `cargo fmt` on
  staged files and re-stages them, so commits land clean.
- **commit-msg** enforces [Conventional Commits](https://www.conventionalcommits.org)
  (`feat:`, `fix:`, `refactor:`, `chore:`, `ci:`, `docs:`, `test:`, ...).
- **pre-push** verifies (no mutation): `cargo fmt --check`, `cargo clippy -D warnings`,
  `typos`, and the test suite. A failing check blocks the push.

Conventional Commits are required because releases are automated from them.

## Releases

Releases are automated by [release-plz](https://release-plz.dev):

1. Merging Conventional Commits to `master` opens/updates a **release PR** that
   bumps the version and updates `CHANGELOG.md`.
2. Merging that PR tags the commit and creates a **GitHub Release**, which
   triggers cross-compiled binary uploads.

### One-time setup (maintainers)

release-plz needs a token that can trigger downstream workflows (the default
`GITHUB_TOKEN` cannot). Create a **GitHub App** with `Contents: read & write`
and `Pull requests: read & write`, install it on the repo, and add two repo
secrets:

- `RELEASE_PLZ_APP_ID`
- `RELEASE_PLZ_APP_PRIVATE_KEY`

(A fine-grained PAT with the same permissions works as a fallback.)

## License

By contributing you agree your contributions are licensed under the project's
Apache-2.0 license.
