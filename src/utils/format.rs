use humansize::{BINARY, ToF64, Unsigned, format_size};

/// Format bytes using binary prefixes (KiB, MiB, etc.)
pub fn format_bytes(bytes: impl ToF64 + Unsigned) -> String {
    format_size(bytes, BINARY)
}

/// Format values for network throughput from Mbps
pub fn format_throughput(mbps: f64) -> String {
    if mbps >= 1000.0 {
        format!("{:.2} Gbps", mbps / 1000.0)
    } else {
        format!("{mbps:.2} Mbps")
    }
}
