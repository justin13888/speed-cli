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
| `mise run check` | fmt-check + lint + typos + machete + no-default-features check (the static gate) |
| `mise run ci` | `check` + `test` (what CI runs) |
| `mise run bench` | criterion benchmarks |
| `mise run bench-loopback` | sustained loopback suite run for A/B comparisons ([docs/PROFILING.md](docs/PROFILING.md)) |
| `mise run flamegraph-server` / `flamegraph-client` | profile a loopback run with cargo-flamegraph |

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
2. Merging that PR publishes the crate to **crates.io**, tags the commit, creates
   a **GitHub Release**, and — in the *same* workflow run — cross-compiles
   binaries and bumps the Homebrew tap (`release-binaries.yml`).

Everything authenticates with the built-in `GITHUB_TOKEN` and crates.io OIDC —
**no long-lived publish token and no GitHub App** are required.

### One-time setup (maintainers)

**crates.io Trusted Publishing.** Publishing uses
[Trusted Publishing](https://crates.io/docs/trusted-publishing): the `release`
job has `id-token: write`, and release-plz exchanges a GitHub OIDC token for a
short-lived crates.io token at publish time. There is **no `CARGO_REGISTRY_TOKEN`
secret**. Configure it once:

1. A crate can't be *created* through Trusted Publishing, so the first version is
   published manually — already done for `1.0.0` (`cargo publish` with a token in
   `~/.cargo/credentials.toml`).
2. On crates.io, open the crate's **Settings → Trusted Publishing → Add a new
   GitHub publisher** and enter:
   - **Repository owner:** `justin13888`
   - **Repository name:** `speed-cli`
   - **Workflow filename:** `release-plz.yml`
   - **Environment:** *(leave blank)*

   Every subsequent release then publishes with no stored token.

**Let Actions open the release PR.** release-plz opens the PR with the default
`GITHUB_TOKEN`, so enable **Settings → Actions → General → Workflow permissions →
"Allow GitHub Actions to create and approve pull requests."** (You do *not* need
to switch the default token to read/write — each job requests the scopes it needs
via an explicit `permissions:` block.)

> A PR opened by `GITHUB_TOKEN` does not itself start a `pull_request` CI run
> (GitHub blocks token-triggered cascades). The release PR is a mechanical
> version/changelog bump off code that already passed CI on `master`, so it's
> safe to merge as-is; push an empty commit to it if you want a CI run. For the
> same reason the on-release binary/Homebrew build is invoked *inside* the
> release workflow run (see `release-plz.yml`) rather than via `release:
> published`, so it fires without a PAT or App token.

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
