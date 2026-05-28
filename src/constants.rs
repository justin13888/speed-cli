/// Wire-compatibility version for the control handshake and every
/// per-protocol test framing. Bump on ANY incompatible change to a test
/// protocol or the manifest shape. Distinct from `CARGO_PKG_VERSION`
/// (binary semver) and `REPORT_SCHEMA_VERSION` (report serialization).
/// The client requires an exact match against the server's value and
/// aborts before running any test otherwise.
pub const PROTOCOL_VERSION: u32 = 1;

/// Default port for the control / handshake endpoint — the one port a
/// user normally picks. Every other listener binds an OS-assigned
/// ephemeral port and is discovered via the control manifest.
pub const DEFAULT_CONTROL_PORT: u16 = 9000;

/// Legacy per-protocol default ports. Used only as fallbacks when an
/// explicit `--<proto>-port` override is supplied; the normal path
/// binds ephemeral ports (`:0`) and advertises them in the manifest.
pub const DEFAULT_TCP_PORT: u16 = 5201;
pub const DEFAULT_UDP_PORT: u16 = 5201;
pub const DEFAULT_HTTP_PORT: u16 = 8080;
pub const DEFAULT_HTTPS_PORT: u16 = 8443;

pub const DEFAULT_TCP_PAYLOAD_SIZES: &[usize] = &[1024, 8192, 65536]; // 1KB, 8KB, 64KB
pub const DEFAULT_UDP_PAYLOAD_SIZES: &[usize] = &[1024, 8192, 65536]; // 1KB, 8KB, 64KB
pub const DEFAULT_HTTP_PAYLOAD_SIZES: &[usize] =
    &[1024 * 1024, 10 * 1024 * 1024, 100 * 1024 * 1024]; // 1MB, 10MB, 100MB
/// Maximum allowed upload size for HTTP requests.
pub const MAX_HTTP_UPLOAD_SIZE: usize = 100 * 1024 * 1024 * 1024; // 100GiB

pub const DEFAULT_CHUNK_SIZE: usize = 1024 * 1024; // 1MB
