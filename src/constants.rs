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
