//! Socket construction helpers shared by every listener and client
//! connection: SO_REUSEADDR on server sockets (Unix), optional fixed
//! socket buffers, and user-facing hints when the kernel clamps a
//! buffer request.
//!
//! Policy notes, so call sites don't have to re-derive them:
//!
//! - **SO_REUSEADDR, Unix only.** Lets a restarted server rebind its
//!   fixed control/test ports while old connections linger in
//!   TIME_WAIT. Not set on Windows, where SO_REUSEADDR has different,
//!   hijack-prone semantics (and rebinding after close works without
//!   it).
//! - **SO_REUSEPORT: deliberately not set.** Its purpose is multi
//!   accept-loop scaling, which this tool doesn't need — and it would
//!   let a second server instance silently steal a fraction of
//!   connections on the same port, a correctness hazard for a
//!   measurement tool. See ROADMAP.md.
//! - **Fixed buffers only on request.** On Linux, setting SO_RCVBUF /
//!   SO_SNDBUF disables receive-buffer autotuning, which usually grows
//!   buffers beyond any fixed value on high-BDP paths. `None` therefore
//!   means "leave the kernel alone" and is the right default for TCP.
//!   Buffers are applied *before* bind/connect so accepted sockets
//!   inherit them and the SYN's window scaling is computed from them.

use std::io;
use std::net::SocketAddr;
use std::sync::Once;

use tokio::net::{TcpListener, TcpSocket, TcpStream};

/// Accept backlog for test listeners. Far above anything the tool
/// generates; the kernel clamps it to `net.core.somaxconn` anyway.
const LISTEN_BACKLOG: u32 = 1024;

fn new_socket_for(addr: SocketAddr) -> io::Result<TcpSocket> {
    match addr {
        SocketAddr::V4(_) => TcpSocket::new_v4(),
        SocketAddr::V6(_) => TcpSocket::new_v6(),
    }
}

fn apply_buffers(socket: &TcpSocket, socket_buffer: Option<usize>) {
    let Some(requested) = socket_buffer else {
        return;
    };
    let requested_u32 = u32::try_from(requested).unwrap_or(u32::MAX);
    if let Err(e) = socket.set_recv_buffer_size(requested_u32) {
        tracing::debug!("TCP set_recv_buffer_size({requested}) failed: {e}");
    }
    if let Err(e) = socket.set_send_buffer_size(requested_u32) {
        tracing::debug!("TCP set_send_buffer_size({requested}) failed: {e}");
    }
    let granted_recv = socket.recv_buffer_size().unwrap_or(0) as usize;
    let granted_send = socket.send_buffer_size().unwrap_or(0) as usize;
    warn_once_on_clamp("TCP socket buffers", requested, granted_recv, granted_send);
}

/// Bind a TCP listener with server-appropriate options: SO_REUSEADDR on
/// Unix and an optional fixed socket buffer (`None` = kernel
/// autotuning, the usual right answer for TCP).
pub fn bind_tcp_listener(
    addr: SocketAddr,
    socket_buffer: Option<usize>,
) -> io::Result<TcpListener> {
    let socket = new_socket_for(addr)?;
    #[cfg(unix)]
    socket.set_reuseaddr(true)?;
    apply_buffers(&socket, socket_buffer);
    socket.bind(addr)?;
    socket.listen(LISTEN_BACKLOG)
}

/// Same policy as [`bind_tcp_listener`] but yields a blocking std
/// listener — the axum-server HTTPS path wants one.
pub fn bind_std_tcp_listener(
    addr: SocketAddr,
    socket_buffer: Option<usize>,
) -> io::Result<std::net::TcpListener> {
    use socket2::{Domain, Protocol, Socket, Type};
    let socket = Socket::new(Domain::for_address(addr), Type::STREAM, Some(Protocol::TCP))?;
    #[cfg(unix)]
    socket.set_reuse_address(true)?;
    if let Some(requested) = socket_buffer {
        if let Err(e) = socket.set_recv_buffer_size(requested) {
            tracing::debug!("TCP set_recv_buffer_size({requested}) failed: {e}");
        }
        if let Err(e) = socket.set_send_buffer_size(requested) {
            tracing::debug!("TCP set_send_buffer_size({requested}) failed: {e}");
        }
        warn_once_on_clamp(
            "TCP socket buffers",
            requested,
            socket.recv_buffer_size().unwrap_or(0),
            socket.send_buffer_size().unwrap_or(0),
        );
    }
    socket.bind(&addr.into())?;
    socket.listen(LISTEN_BACKLOG as i32)?;
    Ok(socket.into())
}

/// Connect a TCP socket, applying an optional fixed buffer size before
/// the SYN (SO_RCVBUF must be set pre-connect to influence the
/// advertised window scaling). Tries every resolved address in order,
/// like `TcpStream::connect`.
pub async fn connect_tcp(addr: &str, socket_buffer: Option<usize>) -> io::Result<TcpStream> {
    let mut last_err = None;
    for sockaddr in tokio::net::lookup_host(addr).await? {
        let socket = new_socket_for(sockaddr)?;
        apply_buffers(&socket, socket_buffer);
        match socket.connect(sockaddr).await {
            Ok(stream) => return Ok(stream),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("no addresses resolved for {addr}"),
        )
    }))
}

/// Actionable message when the kernel granted less than the requested
/// socket buffer, or `None` when the request was honored. Pure, so it
/// is unit-testable; `granted_*` are raw `getsockopt` readbacks (on
/// Linux those report double the usable value for kernel bookkeeping,
/// which this accounts for).
pub fn buffer_clamp_hint(
    requested: usize,
    granted_recv: usize,
    granted_send: usize,
) -> Option<String> {
    let effective = |granted: usize| {
        if cfg!(target_os = "linux") {
            granted / 2
        } else {
            granted
        }
    };
    let recv = effective(granted_recv);
    let send = effective(granted_send);
    if recv >= requested && send >= requested {
        return None;
    }
    let granted_min = recv.min(send);
    let advice = if cfg!(target_os = "linux") {
        format!(
            "raise the kernel caps: sudo sysctl -w net.core.rmem_max={requested} net.core.wmem_max={requested}"
        )
    } else if cfg!(target_os = "macos") {
        // maxsockbuf bounds rcv+snd together; ask for both directions.
        format!(
            "raise the kernel cap: sudo sysctl -w kern.ipc.maxsockbuf={}",
            requested.saturating_mul(2)
        )
    } else {
        "the OS clamped the requested size".to_string()
    };
    Some(format!(
        "kernel granted {granted_min} B of the requested {requested} B socket buffer \
         (throughput may be limited on high-bandwidth paths); {advice}"
    ))
}

/// Emit the clamp hint as a warning, once per process. Tests run many
/// parallel streams; one warning is signal, thirty are noise.
pub fn warn_once_on_clamp(kind: &str, requested: usize, granted_recv: usize, granted_send: usize) {
    static ONCE: Once = Once::new();
    if let Some(hint) = buffer_clamp_hint(requested, granted_recv, granted_send) {
        ONCE.call_once(|| tracing::warn!("{kind}: {hint}"));
    }
}

#[cfg(test)]
mod tests {
    use super::buffer_clamp_hint;

    const MIB8: usize = 8 * 1024 * 1024;

    #[test]
    fn honored_request_yields_no_hint() {
        // Linux reports double the usable value; a 16 MiB readback of an
        // 8 MiB request is fully honored on every platform.
        assert!(buffer_clamp_hint(MIB8, 2 * MIB8, 2 * MIB8).is_none());
    }

    #[test]
    fn clamped_request_names_the_sysctl() {
        let hint = buffer_clamp_hint(MIB8, 416 * 1024, 416 * 1024)
            .expect("a clamped grant must produce a hint");
        assert!(hint.contains(&MIB8.to_string()));
        if cfg!(target_os = "linux") {
            assert!(hint.contains("net.core.rmem_max"));
        }
    }

    #[test]
    fn one_clamped_direction_is_enough_to_warn() {
        assert!(buffer_clamp_hint(MIB8, 2 * MIB8, 416 * 1024).is_some());
    }
}
