//! The [`App`] model and [`InitData`] struct for the GUI component

use crate::display::{fmt_hm, fmt_readings, solar_parts, time_valid};
use crate::messages::{Cmd, Msg};
use crate::work::{Mode, do_work};
use chrono::{DateTime, Local, NaiveTime};
use lumina::config::{self, INTERVAL_MAX_SECONDS, INTERVAL_MIN_SECONDS};
use lumina::engine::{Applier, Step};
use lumina::form::Form;
use lumina::location::{self, Location, Source};
use lumina::plan;
use lumina::schedule::target_brightness;
use relm4::prelude::*;
use std::path::PathBuf;

/// Round a spin-button value to a brightness percent, clamped to 0-100
fn spin_u8(value: f64) -> u8 {
    let value = value.clamp(0.0, 100.0);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    {
        value.round() as u8
    }
}

/// Round a spin-button value to a whole number, clamped to `min..=max`
fn spin_u64(value: f64, min: u64, max: u64) -> u64 {
    #[allow(clippy::cast_precision_loss)]
    let min = min as f64;

    #[allow(clippy::cast_precision_loss)]
    let max = max as f64;

    let value = value.clamp(min, max);

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    {
        value.round() as u64
    }
}

/// Ticks (seconds) between full recomputes of derived state
pub(crate) const RECOMPUTE_EVERY_TICKS: u64 = 60;

/// Ticks (seconds) between brightness reads while auto-adjust is off
pub(crate) const READ_EVERY_TICKS: u64 = 2;

/// The model for the GUI component: a snapshot of the current state of the application
pub(crate) struct App {
    /// Current editable form state mirroring the config file
    pub(crate) form: Form,
    /// Path to the TOML config file; `None` falls back to the library default
    pub(crate) config_path: Option<PathBuf>,
    /// `true` while saving is held back (a failed load, no edit yet)
    pub(crate) hold_save: bool,
    /// `true` once the on-disk config has been checked (and backed up if invalid) this session
    pub(crate) config_checked: bool,
    /// Location returned by the last successful IP-based detection
    pub(crate) detected: Option<Location>,
    /// Current detection lifecycle: in-flight vs. resolved vs. failed
    pub(crate) detection: DetectionState,
    /// Auto-adjust loop state and scheduling bookkeeping
    pub(crate) automation: AutoAdjustState,
    /// Total number of 1-second ticks since startup; used for periodic reads
    pub(crate) ticks: u64,
    /// Remembers the last applied brightness so redundant writes are skipped
    pub(crate) applier: Applier,
    /// Background work bookkeeping: one job at a time, plus applier generation
    pub(crate) job: WorkState,
    /// Human-readable string of the most recent brightness readings
    pub(crate) current_text: String,
    /// UI-only schedule state
    pub(crate) schedule: ScheduleState,
    /// Target brightness for the current moment, if computable
    pub(crate) target: Option<u8>,
    /// Today's true solar `(sunrise, sunset)`, independent of custom times
    pub(crate) solar: Option<(NaiveTime, NaiveTime)>,
    /// Subtitle for the follow-the-sun switch (times, polar note, or "—")
    pub(crate) solar_text: String,
    /// Validation state for the currently edited settings
    pub(crate) validation: ValidationState,
}

/// Bookkeeping that keeps background work from overlapping or recording stale results
#[derive(Default)]
pub(crate) struct WorkState {
    /// Bumped whenever the applier is reset or replaced; results from an older generation are
    /// displayed but not recorded
    pub(crate) generation: u64,
    /// `true` while a work task is running
    pub(crate) in_flight: bool,
    /// A request that arrived while a task was running; runs when it finishes
    pub(crate) pending: Option<Mode>,
}

/// State describing the current IP-location detection lifecycle
#[derive(Default)]
pub(crate) struct DetectionState {
    /// `true` while an IP-location request is in flight
    pub(crate) in_flight: bool,
    /// Error message from the last failed detection attempt
    pub(crate) error: Option<String>,
    /// Where the current `detected` location came from
    pub(crate) source: Option<Source>,
}

/// State for the automatic brightness adjustment loop
#[derive(Default)]
pub(crate) struct AutoAdjustState {
    /// Whether the auto-adjust loop is currently enabled
    pub(crate) enabled: bool,
    /// Set when settings changed while auto-adjusting; the next tick re-applies
    /// immediately instead of waiting out the interval
    pub(crate) dirty: bool,
    /// Seconds elapsed since the last auto-apply; resets after each apply
    pub(crate) elapsed: u64,
}

/// UI-only schedule state
#[derive(Default)]
pub(crate) struct ScheduleState {
    /// `false` = follow sunrise/sunset (config `start`/`stop` are blank)
    pub(crate) custom_times: bool,
    /// Custom start remembered while "follow sun" is on, so toggling doesn't lose it
    pub(crate) saved_start: String,
    /// Custom stop remembered while "follow sun" is on, so toggling doesn't lose it
    pub(crate) saved_stop: String,
    /// Request one model-to-view text sync after custom times are seeded
    pub(crate) sync_time_entries: bool,
}

/// Validation state derived from the current form values
#[derive(Default)]
pub(crate) struct ValidationState {
    /// `true` when the form is invalid or location is unknown; blocks auto-save
    pub(crate) invalid: bool,
    /// Field-level validation errors for the editable schedule and range values
    pub(crate) fields: FieldValidationState,
}

/// Field-level validation errors for the editable schedule and range values
#[derive(Default)]
pub(crate) struct FieldValidationState {
    /// `true` when custom *Day starts* holds an unparsable/blank value
    pub(crate) start_invalid: bool,
    /// `true` when custom *Day ends* holds an unparsable/blank value
    pub(crate) stop_invalid: bool,
    /// `true` when minimum brightness exceeds maximum brightness
    pub(crate) range_invalid: bool,
}

/// Initial data passed to the GUI component at startup
pub(crate) struct InitData {
    /// Pre-loaded (or default) form state
    pub(crate) form: Form,
    /// Resolved path to the config file
    pub(crate) config_path: Option<PathBuf>,
    /// `true` if the config file existed but failed to load; saving waits for the first edit
    pub(crate) load_failed: bool,
}

/// Implementation of the `App` model, containing methods for computing derived state and handling
/// user interactions
impl App {
    /// The target brightness for `now`, or `None` if the settings or location aren't usable
    fn compute_target(&self, now: &DateTime<Local>) -> Option<u8> {
        let s = self.form.to_settings(self.detected).ok()?;
        let window = plan::window_for(&s, now.date_naive(), &Local).ok()?;

        Some(target_brightness(now.time(), window, s.min, s.max))
    }

    /// Recompute all derived state based on the current form, detected location, and time
    pub(crate) fn recompute(&mut self) {
        self.validation.fields.start_invalid =
            self.schedule.custom_times && !time_valid(&self.form.start);
        self.validation.fields.stop_invalid =
            self.schedule.custom_times && !time_valid(&self.form.stop);
        self.validation.fields.range_invalid = self.form.min > self.form.max;

        let now = Local::now();
        let window =
            plan::solar_window(self.form.location(self.detected), now.date_naive(), &Local);
        let (solar, solar_text) = solar_parts(window.ok());

        self.solar = solar;
        self.solar_text = solar_text;

        // Location unknown (or still being detected) leaves the target unset
        self.target = if self.inputs_ok() {
            self.compute_target(&now)
        } else {
            None
        };
        self.validation.invalid = self.target.is_none();
    }

    /// Fill any blank custom time with a sensible value: the remembered custom
    /// value if there is one, otherwise today's sunrise/sunset
    pub(crate) fn fill_blank_times(&mut self) {
        let fill = |field: &mut String, saved: &mut String, solar: Option<NaiveTime>| {
            if field.trim().is_empty() {
                let remembered = std::mem::take(saved);
                *field = if remembered.trim().is_empty() {
                    solar.map(fmt_hm).unwrap_or_default()
                } else {
                    remembered
                };
            }
        };

        fill(
            &mut self.form.start,
            &mut self.schedule.saved_start,
            self.solar.map(|s| s.0),
        );

        fill(
            &mut self.form.stop,
            &mut self.schedule.saved_stop,
            self.solar.map(|s| s.1),
        );
    }

    ///  Check whether all user inputs are valid returning `true` if they are
    pub(crate) fn inputs_ok(&self) -> bool {
        !self.validation.fields.start_invalid
            && !self.validation.fields.stop_invalid
            && !self.validation.fields.range_invalid
    }

    /// Return a human-readable string representing the current brightness target or "—" if unknown
    pub(crate) fn target_text(&self) -> String {
        self.target.map_or("—".into(), |t| format!("{t}%"))
    }

    /// Return the subtitle for the *Day starts* row in the Status view
    pub(crate) fn day_starts_text(&self) -> String {
        let time = if self.schedule.custom_times {
            self.form.start.clone()
        } else {
            self.solar
                .map_or_else(|| "—".into(), |(rise, _)| fmt_hm(rise))
        };

        let suffix = if self.schedule.custom_times {
            "custom"
        } else {
            "automatic"
        };

        format!("{time} ({suffix})")
    }

    /// Return the subtitle for the *Day ends* row in the Status view
    pub(crate) fn day_ends_text(&self) -> String {
        let time = if self.schedule.custom_times {
            self.form.stop.clone()
        } else {
            self.solar
                .map_or_else(|| "—".into(), |(_, set)| fmt_hm(set))
        };

        let suffix = if self.schedule.custom_times {
            "custom"
        } else {
            "automatic"
        };

        format!("{time} ({suffix})")
    }

    /// Return a human-readable string representing the current location or status of detection
    pub(crate) fn location_text(&self) -> String {
        if !self.form.auto_location {
            return format!(
                "{:.4}, {:.4} (custom)",
                self.form.latitude, self.form.longitude
            );
        }

        match (
            &self.detected,
            self.detection.in_flight,
            &self.detection.error,
        ) {
            (_, true, _) => "Detecting…".into(),
            (Some(l), _, _) => {
                let text = format!("{:.4}, {:.4} (automatic)", l.latitude, l.longitude);

                match self.detection.source {
                    Some(Source::Cache) => format!("{text} ({})", Source::Cache.label()),
                    _ => text,
                }
            }
            (None, _, Some(e)) => format!("Detection failed: {e}"),
            (None, _, None) => "Not detected yet".into(),
        }
    }

    /// Spawn a background task to detect the user's location based on their IP address, updating
    /// the model state accordingly
    pub(crate) fn detect(&mut self, sender: &ComponentSender<Self>) {
        if self.detection.in_flight {
            return;
        }

        self.detection.in_flight = true;
        self.detection.error = None;

        sender.oneshot_command(async {
            Cmd::Detected(
                location::resolve_async()
                    .await
                    .map_err(|e| format!("{e:#}")),
            )
        });
    }

    /// Start a background read or apply. Only one task runs at a time: an apply requested while
    /// one is running is queued (`Force` wins over `Auto`) and starts when it finishes
    pub(crate) fn work(&mut self, sender: &ComponentSender<Self>, mode: Mode) {
        if mode != Mode::Read && self.target.is_none() {
            return;
        }
        if self.job.in_flight {
            if mode != Mode::Read {
                self.job.pending = match self.job.pending {
                    Some(Mode::Force) => Some(Mode::Force),
                    _ => Some(mode),
                };
            }

            return;
        }

        self.job.in_flight = true;

        let (generation, target, applier) =
            (self.job.generation, self.target, self.applier.clone());
        sender.oneshot_command(async move {
            Cmd::Worked(generation, do_work(mode, target, applier).await)
        });
    }

    /// Fold a finished task into the model, then start any queued request
    pub(crate) fn finish_work(
        &mut self,
        generation: u64,
        result: Result<Step, String>,
        sender: &ComponentSender<Self>,
    ) {
        self.job.in_flight = false;

        match result {
            Ok(step) => {
                self.current_text = match &step.applied {
                    Some(a) => fmt_readings(&a.verified),
                    None => fmt_readings(&step.current),
                };
                if generation == self.job.generation {
                    self.applier.record(&step);
                }
            }
            Err(e) => {
                eprintln!("lumina: backlight error: {e}");
                self.current_text = "Unavailable".into();
            }
        }

        if let Some(mode) = self.job.pending.take() {
            self.work(sender, mode);
        }
    }

    /// Forget what was applied and invalidate any task still in flight
    fn reset_applier(&mut self) {
        self.applier.reset();
        self.job.generation += 1;
        self.job.pending = None;
    }

    /// Turn auto-adjust on or off, restarting from a clean applier
    fn toggle_auto(&mut self, on: bool) -> Option<Mode> {
        self.automation.enabled = on;
        self.reset_applier();
        self.automation.elapsed = 0;
        self.automation.dirty = false;

        on.then_some(Mode::Auto)
    }

    /// Switch between following the sun and custom times. Turning follow-sun on stashes the
    /// custom values; turning it off restores them (or today's sunrise/sunset) and asks the view
    /// to resync its entries
    pub(crate) fn handle_follow_sun(&mut self, follow: bool) {
        if follow && self.schedule.custom_times {
            self.schedule.saved_start = std::mem::take(&mut self.form.start);
            self.schedule.saved_stop = std::mem::take(&mut self.form.stop);
            self.schedule.custom_times = false;
        } else if !follow && !self.schedule.custom_times {
            self.schedule.custom_times = true;
            self.fill_blank_times();
            self.schedule.sync_time_entries = true;
        }
    }

    /// Apply an input message to the form. Returns whether settings changed (so the caller
    /// recomputes and saves) and any work mode to start
    pub(crate) fn handle_input(
        &mut self,
        msg: Msg,
        sender: &ComponentSender<Self>,
    ) -> (bool, Option<Mode>) {
        let mut settings_changed = true;
        let mut run = None;

        match msg {
            Msg::AutoLocation(on) => {
                self.form.auto_location = on;
                if on {
                    if self.detected.is_none() {
                        self.detect(sender);
                    }
                } else {
                    // Seed the manual lat/lon fields from the last detected location so the
                    // SpinRows show a meaningful starting point rather than 0.0000 / 0.0000
                    if let Some(loc) = self.detected {
                        self.form.latitude = loc.latitude;
                        self.form.longitude = loc.longitude;
                    }
                }
            }

            Msg::Latitude(v) => self.form.latitude = v,
            Msg::Longitude(v) => self.form.longitude = v,
            Msg::Min(v) => self.form.min = spin_u8(v),
            Msg::Max(v) => self.form.max = spin_u8(v),
            Msg::FollowSun(follow) => self.handle_follow_sun(follow),
            Msg::Start(s) => self.form.start = s,
            Msg::Stop(s) => self.form.stop = s,
            Msg::Interval(v) => {
                self.form.interval_seconds =
                    spin_u64(v, INTERVAL_MIN_SECONDS, INTERVAL_MAX_SECONDS);
            }
            Msg::RespectManual(on) => {
                self.form.respect_manual_changes = on;
                self.applier = std::mem::take(&mut self.applier).respecting_manual_changes(on);
                self.reset_applier();
            }
            Msg::ApplyNow => {
                settings_changed = false;
                run = Some(Mode::Force);
            }
            Msg::ToggleAuto(on) => {
                settings_changed = false;
                run = self.toggle_auto(on);
            }
        }

        (settings_changed, run)
    }

    /// Back up an invalid on-disk config before the first overwrite. Returns `false` if the
    /// backup failed and saving must be skipped
    fn back_up_once(&mut self) -> bool {
        if self.config_checked {
            return true;
        }

        match config::backup_if_invalid(self.config_path.as_deref()) {
            Ok(Some(bak)) => {
                eprintln!(
                    "lumina: existing config was invalid; saved a copy to {}",
                    bak.display()
                );
            }
            Ok(None) => {}
            Err(e) => {
                eprintln!("lumina: could not back up invalid config, not saving: {e:#}");
                return false;
            }
        }
        self.config_checked = true;

        true
    }

    /// Persist the current form state to the config file, if valid. If the form is invalid,
    /// return early without saving
    pub(crate) fn auto_save(&mut self) {
        if self.hold_save || self.validation.invalid || !self.back_up_once() {
            return; // Don't persist invalid state
        }

        let result = self
            .form
            .to_file_config()
            .and_then(|c| config::save(self.config_path.as_deref(), &c));

        if let Err(e) = result {
            eprintln!("lumina: auto-save failed: {e:#}");
        }
    }
}
