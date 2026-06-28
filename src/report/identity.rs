//! Endpoint identity: who was on each end of the wire when this test ran.
//!
//! `LocalView` is what *this* process observed (its own identity, plus
//! the socket addresses it used). `RemoteView` is what the peer told us
//! about itself via the protocol-level handshake (added in a follow-up
//! phase) — until that ships, `RemoteView` is empty (`identity = None`).

use std::net::SocketAddr;

use serde::{Deserialize, Serialize};

use crate::build_info;

/// Self-description of one endpoint: build-time identity + host info.
///
/// The build-provenance fields below come from [`build_info`]. The
/// `profile` / `rustc` / `build_timestamp` fields were added after the
/// original three, so they are `#[serde(default)]` — a peer that
/// predates them simply leaves them `None` over the wire, keeping the
/// CBOR/JSON handshake forward-compatible.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerIdentity {
    /// `CARGO_PKG_VERSION` of the binary that was running.
    pub version: String,
    /// Git commit captured at build time. `None` when the build had
    /// no `.git` directory (e.g. tarball / crates.io builds).
    pub git_commit: Option<String>,
    /// Build-time dirty flag. `None` if the build environment couldn't
    /// determine it (no git, or `git status` failed).
    pub git_dirty: Option<bool>,
    /// Cargo build profile (`debug` / `release`). `None` when decoded
    /// from a peer that predates this field.
    #[serde(default)]
    pub profile: Option<String>,
    /// `rustc --version` banner of the build. `None` from older peers.
    #[serde(default)]
    pub rustc: Option<String>,
    /// RFC 3339 build timestamp (UTC). `None` from older peers.
    #[serde(default)]
    pub build_timestamp: Option<String>,
    pub hostname: Option<String>,
    pub os: String,
    pub arch: String,
}

impl PeerIdentity {
    /// Build a `PeerIdentity` describing the current process.
    pub fn local() -> Self {
        Self {
            version: build_info::VERSION.to_string(),
            git_commit: build_info::GIT_COMMIT.map(str::to_string),
            git_dirty: build_info::git_dirty(),
            profile: Some(build_info::PROFILE.to_string()),
            rustc: Some(build_info::RUSTC.to_string()),
            build_timestamp: Some(build_info::build_timestamp()),
            hostname: read_hostname(),
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
        }
    }
}

fn read_hostname() -> Option<String> {
    // /etc/hostname exists on most Linux distros, but not on macOS, where the
    // hostname lives in the system configuration store. $HOSTNAME is only
    // exported by interactive shells, so it is unreliable in a spawned process.
    // Fall back to the `hostname` command, which is present on both platforms.
    if let Ok(s) = std::fs::read_to_string("/etc/hostname") {
        let trimmed = s.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    if let Ok(s) = std::env::var("HOSTNAME") {
        if !s.is_empty() {
            return Some(s);
        }
    }
    std::process::Command::new("hostname")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// What this process observed about itself when it set up the test.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalView {
    pub identity: PeerIdentity,
    /// Socket address bound on this side (server: listener bind addr;
    /// client: kernel-assigned ephemeral). `None` if the socket layer
    /// didn't surface it.
    pub local_addr: Option<SocketAddr>,
    /// Socket address connected to / accepted from on the wire.
    pub remote_addr: Option<SocketAddr>,
}

impl LocalView {
    pub fn new() -> Self {
        Self {
            identity: PeerIdentity::local(),
            local_addr: None,
            remote_addr: None,
        }
    }
}

impl Default for LocalView {
    fn default() -> Self {
        Self::new()
    }
}

/// What the peer reported about itself, plus the peer's view of *us*.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RemoteView {
    /// Peer-supplied identity. `None` until the protocol-level
    /// handshake lands or when the peer didn't speak it.
    pub identity: Option<PeerIdentity>,
    /// What the server-side bound listener address was, as the server
    /// reported it.
    pub server_local_addr: Option<SocketAddr>,
    /// What the server saw as our peer address (post-NAT). Useful when
    /// the client is behind a translating middlebox.
    pub observed_client_addr: Option<SocketAddr>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Peers {
    pub client: LocalView,
    pub server: RemoteView,
}

impl Peers {
    pub fn local_only() -> Self {
        Self {
            client: LocalView::new(),
            server: RemoteView::default(),
        }
    }
}

impl Default for Peers {
    fn default() -> Self {
        Self::local_only()
    }
}

use std::fmt::{self, Display, Formatter};

impl Display for PeerIdentity {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "speed-cli {}", self.version)?;
        // Fold commit (+dirty) and build profile into one parenthetical,
        // e.g. `(abcdef123456+dirty, release)`. Either part may be absent.
        let mut tags: Vec<String> = Vec::new();
        if let Some(commit) = &self.git_commit {
            let short: String = commit.chars().take(12).collect();
            let dirty = match self.git_dirty {
                Some(true) => "+dirty",
                Some(false) => "",
                None => "?",
            };
            tags.push(format!("{short}{dirty}"));
        }
        if let Some(profile) = &self.profile {
            tags.push(profile.clone());
        }
        if !tags.is_empty() {
            write!(f, " ({})", tags.join(", "))?;
        }
        write!(
            f,
            " on {} {}/{}",
            self.hostname.as_deref().unwrap_or("?"),
            self.os,
            self.arch
        )
    }
}

impl Display for Peers {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        writeln!(f, "  Client: {}", self.client.identity)?;
        if let Some(addr) = &self.client.local_addr {
            writeln!(f, "    local:  {addr}")?;
        }
        if let Some(addr) = &self.client.remote_addr {
            writeln!(f, "    remote: {addr}")?;
        }
        match &self.server.identity {
            Some(id) => writeln!(f, "  Server: {id}")?,
            None => writeln!(f, "  Server: (no handshake)")?,
        }
        if let Some(addr) = &self.server.server_local_addr {
            writeln!(f, "    bound:    {addr}")?;
        }
        if let Some(addr) = &self.server.observed_client_addr {
            writeln!(f, "    sees us:  {addr}")?;
        }
        Ok(())
    }
}
