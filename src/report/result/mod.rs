use serde::{Deserialize, Serialize};

pub use http::*;
pub use simple::*;
mod http;
mod simple;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TestResult {
    Simple(SimpleTestResult),
    Http(HttpTestResult),
}

impl From<SimpleTestResult> for TestResult {
    fn from(result: SimpleTestResult) -> Self {
        TestResult::Simple(result)
    }
}

impl From<HttpTestResult> for TestResult {
    fn from(result: HttpTestResult) -> Self {
        TestResult::Http(result)
    }
}
