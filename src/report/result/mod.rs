use serde::{Deserialize, Serialize};
use std::fmt::{self, Display, Formatter};

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

impl Display for TestResult {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            TestResult::Simple(result) => write!(f, "{result}"),
            TestResult::Http(result) => write!(f, "{result}"),
        }
    }
}
