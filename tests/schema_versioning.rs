//! Schema-version sanity. With CBOR-only export and no backwards
//! compatibility, the current `REPORT_SCHEMA_VERSION` is 9 and reports
//! at any other version are rejected at import time. This test is a
//! tripwire: bump it deliberately whenever the schema changes.

use speed_cli::report::REPORT_SCHEMA_VERSION;

#[test]
fn current_schema_version_is_nine() {
    assert_eq!(REPORT_SCHEMA_VERSION, 9);
}
