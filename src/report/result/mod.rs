use serde::{Deserialize, Serialize};
use std::fmt::{self, Display, Formatter};

pub use latency::*;
pub use network::*;
pub use throughput::*;

mod latency;
mod network;
mod throughput;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TestResult {
    Simple(ThroughputResult),
    Network(NetworkTestResult),
}

impl From<ThroughputResult> for TestResult {
    fn from(result: ThroughputResult) -> Self {
        TestResult::Simple(result)
    }
}

impl From<NetworkTestResult> for TestResult {
    fn from(result: NetworkTestResult) -> Self {
        TestResult::Network(result)
    }
}

impl Display for TestResult {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            TestResult::Simple(result) => write!(f, "{result}"),
            TestResult::Network(result) => write!(f, "{result}"),
        }
    }
}
