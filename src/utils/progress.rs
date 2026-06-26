use colored::*;
use std::future::Future;
use std::time::Duration;
use tokio::time::{Instant, interval};

/// A wrapper that shows a count up indicator around an async function
/// The indicator updates every second and shows elapsed time
pub async fn with_progress_counter<F, T>(message: &str, future: F) -> T
where
    F: Future<Output = T>,
{
    let start_time = Instant::now();
    let mut interval = interval(Duration::from_secs(1));

    // Pin the future so we can poll it
    tokio::pin!(future);

    // Print initial message
    print!("{} ", message.yellow().bold());
    std::io::Write::flush(&mut std::io::stdout()).unwrap();

    let mut seconds = 0u64;

    loop {
        tokio::select! {
            // Check if the main future is complete
            result = &mut future => {
                // Clear the current line and print completion message
                print!("\r{} {} ({}s)\n",
                    "✓".green().bold(),
                    message.green(),
                    seconds
                );
                std::io::Write::flush(&mut std::io::stdout()).unwrap();
                return result;
            }

            // Update the progress counter every second
            _ = interval.tick() => {
                seconds = start_time.elapsed().as_secs();
                print!("\r{} {} ({}s)",
                    "⏳".blue(),
                    message.yellow().bold(),
                    seconds
                );
                std::io::Write::flush(&mut std::io::stdout()).unwrap();
            }
        }
    }
}
