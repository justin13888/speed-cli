use serde::{Deserialize, Serialize};

pub use http::*;
pub use latency::*;
pub use simple::*;
mod http;
mod latency;
mod simple;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TestResult {
    Simple(ThroughputResult),
    Http(HttpTestResult),
}

impl From<ThroughputResult> for TestResult {
    fn from(result: ThroughputResult) -> Self {
        TestResult::Simple(result)
    }
}

impl From<HttpTestResult> for TestResult {
    fn from(result: HttpTestResult) -> Self {
        TestResult::Http(result)
    }
}
