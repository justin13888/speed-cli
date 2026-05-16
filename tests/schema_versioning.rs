//! Schema-version sanity. With CBOR-only export and no backwards
//! compatibility, the current `REPORT_SCHEMA_VERSION` is 4 and reports
//! at any other version are rejected at import time.

use speed_cli::report::REPORT_SCHEMA_VERSION;

#[test]
fn current_schema_version_is_four() {
    assert_eq!(REPORT_SCHEMA_VERSION, 4);
}
