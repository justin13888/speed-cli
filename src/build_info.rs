//! Build-time provenance — the single source of truth for *which*
//! binary this is: crate version, git commit and dirty state, build
//! profile, rustc toolchain, and build timestamp.
//!
//! Every value is captured at compile time. `build.rs` runs git and
//! `rustc --version`, stamps the build time, and exports the results as
//! `cargo:rustc-env` variables; this module reads them back through
//! `env!` / `option_env!`. Everything that reports the version —
//! the `--version` / `-V` flag, the report metadata, and the
//! peer-identity handshake — routes through here so there is exactly
//! one place that answers "what build am I?".

use std::fmt::{self, Display, Formatter};
use std::sync::LazyLock;

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

/// Crate version (`CARGO_PKG_VERSION`).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Full git commit hash captured at build time, or `None` when the
/// build tree had no `.git` (crates.io / tarball builds).
pub const GIT_COMMIT: Option<&str> = option_env!("GIT_COMMIT");

/// Cargo build profile: `"debug"` or `"release"` (`"unknown"` if the
/// build script couldn't read it).
pub const PROFILE: &str = env!("BUILD_PROFILE");

/// `rustc --version` banner of the compiler that built this binary.
pub const RUSTC: &str = env!("BUILD_RUSTC");

/// Build-time dirty flag as the raw `{true,false,unknown}` string.
const GIT_DIRTY: Option<&str> = option_env!("GIT_DIRTY");

/// Build timestamp as Unix seconds (see `build.rs`).
const BUILD_UNIX_TIME: &str = env!("BUILD_UNIX_TIME");

/// Whether the working tree had uncommitted changes at build time.
/// `None` when git state couldn't be determined (no `.git`, or the
/// `git status` probe failed).
pub fn git_dirty() -> Option<bool> {
    match GIT_DIRTY {
        Some("true") => Some(true),
        Some("false") => Some(false),
        _ => None,
    }
}

/// Build timestamp as a UTC `DateTime`, decoded from the compile-time
/// Unix-seconds stamp. Falls back to the epoch if the stamp is somehow
/// unparseable (it never should be).
pub fn build_time() -> DateTime<Utc> {
    BUILD_UNIX_TIME
        .parse::<i64>()
        .ok()
        .and_then(|secs| DateTime::from_timestamp(secs, 0))
        .unwrap_or_else(|| DateTime::from_timestamp(0, 0).expect("epoch is a valid timestamp"))
}

/// RFC 3339 build timestamp, e.g. `2026-06-25T12:00:00Z`.
pub fn build_timestamp() -> String {
    build_time().to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// Serializable snapshot of build provenance, embedded in reports so a
/// saved report records exactly which binary produced it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildInfo {
    /// Crate version (`CARGO_PKG_VERSION`).
    pub version: String,
    /// Full git commit hash, or `None` for non-git builds.
    pub git_commit: Option<String>,
    /// Working-tree dirty flag at build time; `None` if undetermined.
    pub git_dirty: Option<bool>,
    /// Cargo build profile (`debug` / `release`).
    pub profile: String,
    /// `rustc --version` banner.
    pub rustc: String,
    /// RFC 3339 build timestamp (UTC).
    pub build_timestamp: String,
}

impl BuildInfo {
    /// Provenance of the currently-running binary.
    pub fn current() -> Self {
        Self {
            version: VERSION.to_string(),
            git_commit: GIT_COMMIT.map(str::to_string),
            git_dirty: git_dirty(),
            profile: PROFILE.to_string(),
            rustc: RUSTC.to_string(),
            build_timestamp: build_timestamp(),
        }
    }

    /// Short commit (first 12 chars) plus a `+dirty` / `?` suffix, or
    /// `None` when there was no commit.
    pub fn commit_short(&self) -> Option<String> {
        self.git_commit.as_ref().map(|commit| {
            let short: String = commit.chars().take(12).collect();
            let suffix = match self.git_dirty {
                Some(true) => "+dirty",
                Some(false) => "",
                None => "?",
            };
            format!("{short}{suffix}")
        })
    }

    /// One-line summary: `0.1.0 (abcdef123456+dirty, release)`. The
    /// commit segment is omitted for non-git builds.
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if let Some(commit) = self.commit_short() {
            parts.push(commit);
        }
        parts.push(self.profile.clone());
        format!("{} ({})", self.version, parts.join(", "))
    }
}

impl Default for BuildInfo {
    fn default() -> Self {
        Self::current()
    }
}

impl Display for BuildInfo {
    /// Multi-line, label-aligned form used by `--version` and report
    /// output. The first line is just the version so callers (clap)
    /// can prefix it with the binary name.
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let commit = match &self.git_commit {
            Some(c) => {
                let dirty = match self.git_dirty {
                    Some(true) => " (dirty)",
                    Some(false) => "",
                    None => " (dirty: unknown)",
                };
                format!("{c}{dirty}")
            }
            None => "unknown".to_string(),
        };
        writeln!(f, "{}", self.version)?;
        writeln!(f, "commit:  {commit}")?;
        writeln!(f, "profile: {}", self.profile)?;
        writeln!(f, "rustc:   {}", self.rustc)?;
        write!(f, "built:   {}", self.build_timestamp)
    }
}

/// Multi-line version string for clap's `-V` / `--version`. clap prints
/// `speed-cli ` immediately before this, so the leading version line
/// completes the `speed-cli <version>` banner and the remaining lines
/// detail the build.
///
/// Held in a `LazyLock` so it can be handed to clap as a `&'static str`
/// (clap's `version(...)` requires a static lifetime).
pub static LONG_VERSION: LazyLock<String> = LazyLock::new(|| BuildInfo::current().to_string());
