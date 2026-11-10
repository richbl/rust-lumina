//! Process signal handling for the run loop

use anyhow::{Context, Result};
use tokio::signal::unix::{Signal, SignalKind, signal};

/// What woke the run loop
pub(super) enum Wake {
    /// The interval elapsed
    Tick,
    /// SIGHUP arrived; re-read the config
    Reload,
    /// SIGINT or SIGTERM arrived; shut down
    Stop,
}

/// The process signals the run loop reacts to
pub(super) struct Signals {
    /// SIGINT: stop
    int: Signal,
    /// SIGTERM: stop
    term: Signal,
    /// SIGHUP: reload
    hup: Signal,
}

impl Signals {
    /// Register every handler up front so a signal during the first tick isn't lost to the
    /// default action
    pub(super) fn install() -> Result<Self> {
        let on =
            |kind, name| signal(kind).with_context(|| format!("failed to install {name} handler"));

        Ok(Self {
            int: on(SignalKind::interrupt(), "SIGINT")?,
            term: on(SignalKind::terminate(), "SIGTERM")?,
            hup: on(SignalKind::hangup(), "SIGHUP")?,
        })
    }

    /// Wait for the next signal: SIGHUP asks for a reload, SIGINT and SIGTERM stop the loop
    pub(super) async fn wait(&mut self) -> Wake {
        tokio::select! {
            _ = self.int.recv() => Wake::Stop,
            _ = self.term.recv() => Wake::Stop,
            _ = self.hup.recv() => Wake::Reload,
        }
    }
}
