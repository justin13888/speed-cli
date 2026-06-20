//! Progress-bar construction and global enable/disable gating.

use std::sync::OnceLock;
use std::time::Duration;

use indicatif::{ProgressBar, ProgressStyle};

/// Whether progress bars are shown. Set once at startup; defaults to enabled
/// so library/test use still renders (indicatif itself hides on a non-TTY).
static ENABLED: OnceLock<bool> = OnceLock::new();

/// Globally enable or disable progress bars. Called once at startup — e.g.
/// `--quiet` disables them. When disabled, [`create_progress_bar`] returns a
/// hidden bar so callers need no special-casing.
pub fn set_enabled(enabled: bool) {
    let _ = ENABLED.set(enabled);
}

fn enabled() -> bool {
    *ENABLED.get().unwrap_or(&true)
}

/// Test phase a progress bar represents, used for styling.
#[derive(Clone, Copy)]
pub enum ProgressBarType {
    Latency,
    Download,
    Upload,
}

/// Create a styled progress bar for `test_type` spanning `duration`. Returns a
/// hidden bar when progress is disabled.
pub fn create_progress_bar(test_type: ProgressBarType, duration: Duration) -> ProgressBar {
    if !enabled() {
        return ProgressBar::hidden();
    }

    let progress_bar = ProgressBar::new(duration.as_secs());

    let (color, test_name) = match test_type {
        ProgressBarType::Latency => ("yellow/blue", "latency"),
        ProgressBarType::Download => ("cyan/blue", "download"),
        ProgressBarType::Upload => ("green/blue", "upload"),
    };

    progress_bar.set_style(
        ProgressStyle::default_bar()
            .template(&format!(
                "[{{elapsed_precise}}] {{bar:40.{color}}} {{pos}}s/{{len}}s {{msg}}"
            ))
            .unwrap()
            .progress_chars("##-"),
    );
    progress_bar.set_message(format!("{}...", test_name.to_title_case()));
    progress_bar
}

trait ToTitleCase {
    fn to_title_case(&self) -> String;
}

impl ToTitleCase for str {
    fn to_title_case(&self) -> String {
        let mut chars = self.chars();
        match chars.next() {
            None => String::new(),
            Some(first) => first.to_uppercase().chain(chars).collect(),
        }
    }
}
