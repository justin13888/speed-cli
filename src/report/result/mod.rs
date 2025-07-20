use serde::{Deserialize, Serialize};
use std::fmt::{self, Display, Formatter};

pub use http::*;
pub use latency::*;
pub use tcp::*;
pub use throughput::*;

mod http;
mod latency;
mod tcp;
mod throughput;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TestResult {
    Simple(ThroughputResult),
    Http(HttpTestResult),
    Tcp(TcpTestResult),
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

impl From<TcpTestResult> for TestResult {
    fn from(result: TcpTestResult) -> Self {
        TestResult::Tcp(result)
    }
}

impl Display for TestResult {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            TestResult::Simple(result) => write!(f, "{result}"),
            TestResult::Http(result) => write!(f, "{result}"),
            TestResult::Tcp(result) => write!(f, "{result}"),
        }
    }
}
