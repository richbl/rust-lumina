//! Run-loop state between ticks: config reloading, location lookup, and applying brightness

use super::lookup::LocationTracker;
use super::signals::Wake;
use crate::output::{StepLogger, fmt_loc, ts};
use anyhow::{Context, Result};
use chrono::Local;
use lumina::config::{self, INTERVAL_MIN_SECONDS, Settings};
use lumina::engine::Applier;
use lumina::location::{self, Location, Source};
use lumina::plan;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime};
use tokio::time::{self, Interval, MissedTickBehavior};

/// A ticker that delays rather than bursts after a missed tick
fn make_ticker(seconds: u64) -> Interval {
    let mut t = time::interval(Duration::from_secs(seconds.max(INTERVAL_MIN_SECONDS)));
    t.set_missed_tick_behavior(MissedTickBehavior::Delay);
    t
}

/// Everything the run loop session remembers between ticks
pub(super) struct Session<'a> {
    /// Config file path; `None` means the default location
    path: Option<&'a Path>,
    /// Config file mtime when last read, used to spot edits
    stamp: Option<SystemTime>,
    /// Settings exactly as loaded from the config file
    loaded: Settings,
    /// Location lookup state, used when the config has no location
    tracker: LocationTracker,
    /// Tracks what was last applied and decides the next step
    applier: Applier,
    /// Logs each step, suppressing repeated no-op status lines
    logger: StepLogger,
    /// Paces the run loop at the configured interval
    pub(super) ticker: Interval,
}

/// Methods for the run loop session
impl<'a> Session<'a> {
    /// A fresh session: nothing applied, no lookup done, config mtime recorded
    pub(super) fn new(path: Option<&'a Path>, loaded: Settings) -> Self {
        Self {
            path,
            stamp: config::modified(path),
            ticker: make_ticker(loaded.interval_seconds),
            applier: Applier::default().respecting_manual_changes(loaded.respect_manual_changes),
            loaded,
            tracker: LocationTracker::new(Instant::now()),
            logger: StepLogger::default(),
        }
    }

    /// The loaded settings with any detected location filled in
    fn effective(&self) -> Settings {
        let mut s = self.loaded.clone();
        s.location = s.location.or(self.tracker.detected());
        s
    }

    /// `true` when the schedule needs a location that the config doesn't supply
    fn wants_location(&self) -> bool {
        self.loaded.location.is_none() && self.loaded.needs_location()
    }

    /// `true` when a lookup should run now: the schedule needs a location and the tracker says
    /// one is due
    fn lookup_due(&self) -> bool {
        self.wants_location() && self.tracker.is_due(Instant::now())
    }

    /// `true` if the config file's mtime differs from the one last read
    fn config_changed(&self) -> bool {
        config::modified(self.path) != self.stamp
    }

    /// Re-read the config. A bad file keeps the current settings. A real change starts a fresh
    /// applier (the next step is an initial write, which also ends a manual pause); a change to
    /// the interval alone keeps it
    pub(super) fn reload(&mut self) {
        self.stamp = config::modified(self.path);

        let new = match config::load(self.path) {
            Ok(s) => s,
            Err(e) => {
                errln!("{}: reload failed, keeping current settings: {e:#}", ts());
                return;
            }
        };

        if new == self.loaded {
            return;
        }

        outln!("{}: [reload]: settings changed", ts());
        let same_but_interval = Settings {
            interval_seconds: self.loaded.interval_seconds,
            ..new.clone()
        } == self.loaded;

        if new.interval_seconds != self.loaded.interval_seconds {
            self.ticker = make_ticker(new.interval_seconds);
        }

        if !same_but_interval {
            self.applier = Applier::default().respecting_manual_changes(new.respect_manual_changes);
            self.logger.reset();
        }

        self.loaded = new;
    }

    /// Record a successful lookup, logging it when the location or its source changed
    fn found(&mut self, loc: Location, src: Source) {
        let from_cache = src == Source::Cache;
        if self.tracker.record_found(loc, from_cache, Instant::now()) {
            outln!("{}: [location]: {} ({})", ts(), fmt_loc(&loc), src.label());
        }
    }

    /// Record a failed lookup and report the backoff before the next one
    fn lookup_failed(&mut self, e: &anyhow::Error) {
        let wait = self.tracker.record_failure(Instant::now());
        errln!("{}: {e:#}\n  retrying in {}s", ts(), wait.as_secs());
    }

    /// Run a location lookup. Returns `false` when this tick should be skipped because the
    /// schedule needs a location and none is known yet
    async fn ensure_location(&mut self) -> bool {
        if self.lookup_due() {
            match location::resolve_async().await {
                Ok((loc, src)) => self.found(loc, src),
                Err(e) => self.lookup_failed(&e),
            }
        }

        !self.wants_location() || self.tracker.detected().is_some()
    }

    /// One iteration of the run loop: ensure a location, compute the target, apply and log it
    async fn tick(&mut self) -> Result<()> {
        if !self.ensure_location().await {
            return Ok(());
        }
        let target =
            plan::brightness_at(&self.effective(), &Local::now()).context("schedule error")?;
        let step = self.applier.step(target, false).await?;

        self.logger.log(&step, target);
        self.applier.record(&step);

        Ok(())
    }
}

/// Whether this wake-up should re-read the config: always on SIGHUP, on a tick only if the
/// file changed
pub(super) fn should_reload(session: &Session<'_>, wake: &Wake) -> bool {
    match wake {
        Wake::Stop => false,
        Wake::Reload => true,
        Wake::Tick => session.config_changed(),
    }
}

/// Process one tick of the run loop, logging any errors
pub(super) async fn process_tick(session: &mut Session<'_>) {
    if let Err(e) = session.tick().await {
        errln!("{}: {e:#}", ts());
    }
}
