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
2. Merging that PR publishes the crate to **crates.io**, tags the commit, and
   creates a **GitHub Release** — which in turn triggers cross-compiled binary
   uploads and the Homebrew tap bump (`release-binaries.yml`).

### One-time setup (maintainers)

**crates.io publishing.** Add a repo **secret** `CARGO_REGISTRY_TOKEN` — a
crates.io API token with the publish scope, from
<https://crates.io/settings/tokens>. The `release` job's `cargo publish` step
needs it; without it the release fails at publish time.

**Triggering downstream workflows (recommended).** A release PR opened — or a
release tagged — with the default `GITHUB_TOKEN` will *not* trigger other
workflows (GitHub blocks that to prevent loops), so the release PR's CI and the
on-release binary/Homebrew build wouldn't run. To fix that, create a **GitHub
App** with `Contents: read & write` and `Pull requests: read & write`, install
it on the repo, then add:

- a repo **variable** `RELEASE_PLZ_APP_ID` — the app's ID. It's a *variable*,
  not a secret, because the workflow gates on it in an `if:` and those can't read
  secrets; the ID isn't sensitive.
- a repo **secret** `RELEASE_PLZ_APP_PRIVATE_KEY` — the app's private key.

When the variable is unset the workflow falls back to `GITHUB_TOKEN`, so releases
still cut — they just won't auto-trigger the downstream jobs. (A fine-grained PAT
with the same permissions works in place of the App.)

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
     depends_on "cmake" => :build # aws-lc-rs (TLS provider) builds aws-lc-sys via CMake
     depends_on "rust" => :build

     def install
       # speed-cli enables reqwest's HTTP/3, which reqwest gates behind this cfg.
       # The repo's .cargo/config.toml sets it, but `cargo install` ignores that
       # file, so set it explicitly here.
       ENV["RUSTFLAGS"] = "--cfg reqwest_unstable"
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
