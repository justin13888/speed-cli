//! Capture a snapshot of the local environment so reports written on
//! one machine can be meaningfully compared against reports from
//! another. The fields here are deliberately a small, evergreen set;
//! anything that needs richer ethtool-style detail belongs in a
//! dedicated probe later.

use serde::{Deserialize, Serialize};

/// Static-ish description of the host that ran the test.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Environment {
    pub hostname: Option<String>,
    pub os: String,
    pub arch: String,
    /// Kernel / OS version string when we can read it cheaply (Linux:
    /// `uname -r`-equivalent via `/proc/sys/kernel/osrelease`; other
    /// platforms: `None` for now to avoid pulling in a heavyweight
    /// dependency).
    pub kernel: Option<String>,
    pub cpu_count: usize,
    /// Linux only. Reading these is cheap and they are the single
    /// most common cause of "why isn't this saturating the link?"
    /// confusion in throughput tests.
    pub linux: Option<LinuxNetEnv>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LinuxNetEnv {
    /// `/proc/sys/net/ipv4/tcp_congestion_control` (e.g. "cubic", "bbr").
    pub tcp_congestion_control: Option<String>,
    /// `/proc/sys/net/core/rmem_max` in bytes.
    pub rmem_max: Option<u64>,
    /// `/proc/sys/net/core/wmem_max` in bytes.
    pub wmem_max: Option<u64>,
    /// `/proc/sys/net/core/netdev_max_backlog`.
    pub netdev_max_backlog: Option<u64>,
}

impl Environment {
    /// Capture what we can, swallowing per-field errors so a missing
    /// `/proc` entry doesn't tank the report.
    pub fn capture() -> Self {
        Self {
            hostname: read_hostname(),
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            kernel: read_kernel(),
            cpu_count: num_cpus::get(),
            linux: if cfg!(target_os = "linux") {
                Some(LinuxNetEnv {
                    tcp_congestion_control: read_str(
                        "/proc/sys/net/ipv4/tcp_congestion_control",
                    ),
                    rmem_max: read_u64("/proc/sys/net/core/rmem_max"),
                    wmem_max: read_u64("/proc/sys/net/core/wmem_max"),
                    netdev_max_backlog: read_u64("/proc/sys/net/core/netdev_max_backlog"),
                })
            } else {
                None
            },
        }
    }
}

fn read_hostname() -> Option<String> {
    // /etc/hostname is the most portable way without pulling in libc.
    // Falls back to $HOSTNAME, then None.
    if let Some(h) = read_str("/etc/hostname") {
        return Some(h);
    }
    std::env::var("HOSTNAME").ok()
}

fn read_kernel() -> Option<String> {
    if cfg!(target_os = "linux") {
        read_str("/proc/sys/kernel/osrelease")
    } else {
        None
    }
}

fn read_str(path: &str) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn read_u64(path: &str) -> Option<u64> {
    read_str(path).and_then(|s| s.parse().ok())
}

use std::fmt::{self, Display, Formatter};

impl Display for Environment {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        writeln!(f, "  Host:   {}", self.hostname.as_deref().unwrap_or("?"))?;
        writeln!(f, "  OS:     {} ({})", self.os, self.arch)?;
        if let Some(k) = &self.kernel {
            writeln!(f, "  Kernel: {k}")?;
        }
        writeln!(f, "  CPUs:   {}", self.cpu_count)?;
        if let Some(linux) = &self.linux {
            if let Some(cc) = &linux.tcp_congestion_control {
                writeln!(f, "  TCP cc: {cc}")?;
            }
            if let (Some(r), Some(w)) = (linux.rmem_max, linux.wmem_max) {
                writeln!(f, "  rmem_max / wmem_max: {r} / {w}")?;
            }
            if let Some(b) = linux.netdev_max_backlog {
                writeln!(f, "  netdev_max_backlog: {b}")?;
            }
        }
        Ok(())
    }
}
