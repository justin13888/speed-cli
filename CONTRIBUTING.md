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

### Homebrew tap

Releases auto-update the formula in
[`justin13888/homebrew-tap`](https://github.com/justin13888/homebrew-tap) via the
`bump-homebrew` job in `release-binaries.yml`. Two one-time steps are required:

1. **Seed the formula.** The tap starts empty; commit an initial
   `Formula/speed-cli.rb` once. The auto-bump assumes a *source-build* formula
   (the kind `dawidd6/action-homebrew-bump-formula` understands), e.g.:

   ```ruby
   class SpeedCli < Formula
     desc "Comprehensive multi-protocol network performance testing CLI"
     homepage "https://github.com/justin13888/speed-cli"
     url "https://github.com/justin13888/speed-cli/archive/refs/tags/v1.0.0.tar.gz"
     sha256 "<sha256-of-the-tarball>"
     license "Apache-2.0"
     depends_on "rust" => :build

     def install
       system "cargo", "install", *std_cargo_args
     end

     test do
       assert_match "speed-cli", shell_output("#{bin}/speed-cli --version")
     end
   end
   ```

2. **Add the token.** Create a repo secret `HOMEBREW_TAP_TOKEN` — a fine-grained
   PAT with `Contents: read & write` on the tap repo. Until it is set, the
   `bump-homebrew` job no-ops (stays green).

After that, every release bumps the formula's `url`/`sha256` automatically.

## License

By contributing you agree your contributions are licensed under the project's
Apache-2.0 license.
