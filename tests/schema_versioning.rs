//! Schema-version sanity. With CBOR-only export and no backwards
//! compatibility, the current `REPORT_SCHEMA_VERSION` is 5 and reports
//! at any other version are rejected at import time. This test is a
//! tripwire: bump it deliberately whenever the schema changes.

use speed_cli::report::REPORT_SCHEMA_VERSION;

#[test]
fn current_schema_version_is_five() {
    assert_eq!(REPORT_SCHEMA_VERSION, 5);
}
