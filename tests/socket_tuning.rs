//! Socket-option contracts for the shared listener/connect helpers:
//! SO_REUSEADDR on Unix listeners, explicit buffer application, and the
//! default staying hands-off so kernel autotuning survives.

use eyre::Result;
use socket2::SockRef;
use speed_cli::utils::net::{bind_tcp_listener, connect_tcp};

fn ephemeral() -> std::net::SocketAddr {
    "127.0.0.1:0".parse().unwrap()
}

#[cfg(unix)]
#[tokio::test]
async fn tcp_listener_sets_reuseaddr() -> Result<()> {
    let listener = bind_tcp_listener(ephemeral(), None)?;
    let sock = SockRef::from(&listener);
    assert!(
        sock.reuse_address()?,
        "server listeners must set SO_REUSEADDR on Unix"
    );
    Ok(())
}

#[tokio::test]
async fn explicit_socket_buffer_is_applied() -> Result<()> {
    let requested = 1 << 20; // 1 MiB — modest enough to be granted anywhere.
    let listener = bind_tcp_listener(ephemeral(), Some(requested))?;
    let sock = SockRef::from(&listener);
    // Linux getsockopt reports double the usable value.
    let effective = |granted: usize| {
        if cfg!(target_os = "linux") {
            granted / 2
        } else {
            granted
        }
    };
    assert!(
        effective(sock.recv_buffer_size()?) >= requested / 2,
        "an explicit 1 MiB request should be at least half-granted"
    );
    Ok(())
}

/// `None` must leave the buffers exactly as a plain bind would — the
/// whole point of the default is not to disable kernel autotuning.
#[tokio::test]
async fn default_leaves_buffers_untouched() -> Result<()> {
    let ours = bind_tcp_listener(ephemeral(), None)?;
    let control = tokio::net::TcpListener::bind(ephemeral()).await?;
    assert_eq!(
        SockRef::from(&ours).recv_buffer_size()?,
        SockRef::from(&control).recv_buffer_size()?,
        "default bind must not touch SO_RCVBUF"
    );
    assert_eq!(
        SockRef::from(&ours).send_buffer_size()?,
        SockRef::from(&control).send_buffer_size()?,
        "default bind must not touch SO_SNDBUF"
    );
    Ok(())
}

#[tokio::test]
async fn connect_tcp_reaches_a_listener() -> Result<()> {
    let listener = bind_tcp_listener(ephemeral(), None)?;
    let addr = listener.local_addr()?;
    let accept = tokio::spawn(async move { listener.accept().await });

    let stream = connect_tcp(&addr.to_string(), Some(1 << 20)).await?;
    assert_eq!(stream.peer_addr()?, addr);
    accept.await??;
    Ok(())
}
