//! Main entry point for the `lumina` CLI tool
//!
//! This is a small utility that dims or brightens the device display based on the time of day and
//! the user's location. It can be run once to apply the current brightness, or it can run
//! continuously, adjusting brightness at a set interval

#[macro_use]
mod output;

mod cli;
mod commands;
mod daemon;
mod installer;

use anyhow::Result;

/// The CLI-mode main entry point is marked as `async` because some commands require asynchronous
/// operations (e.g., reading/writing brightness values)
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let cli = cli::parse();

    commands::dispatch(cli.cmd).await
}
