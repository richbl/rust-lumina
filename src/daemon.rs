//! The long-running `run` command: apply the target brightness on an interval until interrupted
//!
//! The future version of me thinks we should do a real daemon (`systemd` service)

mod lookup;
mod session;
mod signals;

use anyhow::Result;
use lumina::config::Settings;
use session::{Session, process_tick, should_reload};
use signals::{Signals, Wake};
use std::path::Path;

/// Wait for either the ticker or a signal to wake the run loop
async fn wait_for_wake(session: &mut Session<'_>, signals: &mut Signals) -> Wake {
    tokio::select! {
        _ = session.ticker.tick() => Wake::Tick,
        w = signals.wait() => w,
    }
}

/// Run the main loop, applying the target brightness at the configured interval until
/// interrupted (SIGINT or SIGTERM). SIGHUP, or a changed config file, reloads the settings
pub(crate) async fn run(path: Option<&Path>, settings: Settings) -> Result<()> {
    let mut signals = Signals::install()?;
    let mut session = Session::new(path, settings);

    loop {
        let wake = wait_for_wake(&mut session, &mut signals).await;

        if matches!(wake, Wake::Stop) {
            break;
        }

        if should_reload(&session, &wake) {
            session.reload();
        }

        process_tick(&mut session).await;
    }

    outln!("Goodbye.");

    Ok(())
}
