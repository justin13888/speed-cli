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

impl LinuxNetEnv {
    /// Actionable tuning suggestions derived from the captured sysctls.
    /// Shown in the environment section of reports and logged at server
    /// startup, so "why isn't this saturating the link?" comes with an
    /// answer attached.
    pub fn tuning_hints(&self) -> Vec<String> {
        // The buffer size the UDP tests (and quinn internally) ask for.
        let wanted = crate::performance::udp::SOCKET_BUFFER_BYTES as u64;
        let mut hints = Vec::new();
        if let Some(r) = self.rmem_max
            && r < wanted
        {
            hints.push(format!(
                "net.core.rmem_max ({r} B) caps receive buffers below the {wanted} B the \
                 UDP/QUIC tests request; raise it: sudo sysctl -w net.core.rmem_max={wanted}"
            ));
        }
        if let Some(w) = self.wmem_max
            && w < wanted
        {
            hints.push(format!(
                "net.core.wmem_max ({w} B) caps send buffers below the {wanted} B the \
                 UDP/QUIC tests request; raise it: sudo sysctl -w net.core.wmem_max={wanted}"
            ));
        }
        if let Some(cc) = &self.tcp_congestion_control
            && cc != "bbr"
        {
            hints.push(format!(
                "TCP tests use the OS congestion controller ({cc}); QUIC tests can opt \
                 into BBR with --congestion bbr"
            ));
        }
        hints
    }
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
                    tcp_congestion_control: read_str("/proc/sys/net/ipv4/tcp_congestion_control"),
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
    // /etc/hostname exists on most Linux distros, but not on macOS, where the
    // hostname lives in the system configuration store. $HOSTNAME is only
    // exported by interactive shells, so it is unreliable in a spawned process.
    // Fall back to the `hostname` command, which is present on both platforms.
    if let Some(h) = read_str("/etc/hostname") {
        return Some(h);
    }
    if let Ok(h) = std::env::var("HOSTNAME")
        && !h.is_empty()
    {
        return Some(h);
    }
    hostname_cmd()
}

/// Query the `hostname` binary. Works on macOS and Linux without unsafe libc
/// calls (we `forbid(unsafe_code)`).
fn hostname_cmd() -> Option<String> {
    std::process::Command::new("hostname")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
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
            for hint in linux.tuning_hints() {
                writeln!(f, "  hint:   {hint}")?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::LinuxNetEnv;

    #[test]
    fn tuning_hints_flag_low_rmem_max() {
        let env = LinuxNetEnv {
            tcp_congestion_control: Some("bbr".to_string()),
            rmem_max: Some(212_992),
            wmem_max: Some(64 * 1024 * 1024),
            netdev_max_backlog: Some(1000),
        };
        let hints = env.tuning_hints();
        assert_eq!(hints.len(), 1);
        assert!(hints[0].contains("net.core.rmem_max"));
        assert!(hints[0].contains("sysctl"));
    }

    #[test]
    fn tuning_hints_quiet_when_tuned() {
        let env = LinuxNetEnv {
            tcp_congestion_control: Some("bbr".to_string()),
            rmem_max: Some(64 * 1024 * 1024),
            wmem_max: Some(64 * 1024 * 1024),
            netdev_max_backlog: Some(1000),
        };
        assert!(env.tuning_hints().is_empty());
    }

    #[test]
    fn tuning_hints_mention_quic_bbr_option() {
        let env = LinuxNetEnv {
            tcp_congestion_control: Some("cubic".to_string()),
            rmem_max: None,
            wmem_max: None,
            netdev_max_backlog: None,
        };
        let hints = env.tuning_hints();
        assert_eq!(hints.len(), 1);
        assert!(hints[0].contains("--congestion bbr"));
    }
}
