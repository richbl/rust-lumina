//! The apply workflow shared by the CLI and the GUI: read the displays, decide whether a write is
//! warranted, write, and verify. Callers own an [`Applier`] and feed it a target brightness

use crate::backlight::{self, Device, Failure, Outcome, Reading};
use anyhow::{Context, Result};
use std::fmt;

/// Why a brightness write was (or would be) performed
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyReason {
    /// The caller explicitly asked for the write (e.g. the GUI *Apply* button)
    Forced,
    /// First write since startup (or since the applier was reset)
    Initial,
    /// The scheduled target moved since the last write
    Schedule,
    /// Brightness no longer matches what we last verified, so something else changed it
    UserChange,
}

impl fmt::Display for ApplyReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Forced => "forced",
            Self::Initial => "initial",
            Self::Schedule => "schedule",
            Self::UserChange => "user change detected",
        })
    }
}

/// A completed brightness write
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    /// Why the write happened
    pub reason: ApplyReason,
    /// The brightness percentage that was requested
    pub target: u8,
    /// Readings taken back from the displays after the write
    pub verified: Vec<Reading>,
    /// Devices whose write or verification failed
    pub failed: Vec<Failure>,
}

/// The result of one pass through the workflow
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// Readings taken *before* any write
    pub current: Vec<Reading>,
    /// Devices that could not be read before any write
    pub unreadable: Vec<Failure>,
    /// Present when a write was performed
    pub applied: Option<Applied>,
}

impl Step {
    /// Read the displays without ever writing
    ///
    /// # Errors
    /// Returns an error if the devices cannot be enumerated, or if every device fails to read
    pub async fn observe() -> Result<Self> {
        let seen = backlight::get_all()
            .await
            .and_then(Outcome::require_any)
            .context("read failed")?;

        Ok(Self {
            current: seen.readings,
            unreadable: seen.failures,
            applied: None,
        })
    }
}

/// Write `target` to every device and package the result
async fn write_all(devices: &mut [Device], target: u8, reason: ApplyReason) -> Result<Applied> {
    let out = backlight::write(devices, u32::from(target))
        .await
        .and_then(Outcome::require_any)
        .context("set failed")?;

    Ok(Applied {
        reason,
        target,
        verified: out.readings,
        failed: out.failures,
    })
}

/// Remembers what was last applied so that redundant writes can be skipped
#[derive(Debug, Default, Clone)]
pub struct Applier {
    last_target: Option<u8>,
    last_verified: Vec<Reading>,
    respect_manual: bool,
}

impl Applier {
    /// Choose whether a brightness change made outside lumina pauses scheduled writes
    ///
    /// When `true`, once a display no longer matches what was last verified, nothing is written
    /// until the readings match again, [`reset`](Self::reset) is called, or a write is forced.
    /// When `false` (the default) the next step writes with [`ApplyReason::UserChange`] and takes
    /// control back
    #[must_use]
    pub fn respecting_manual_changes(mut self, yes: bool) -> Self {
        self.respect_manual = yes;
        self
    }

    /// Forget what was written; the next non-forced step will report [`ApplyReason::Initial`]
    pub fn reset(&mut self) {
        self.last_target = None;
        self.last_verified.clear();
    }

    /// True if a display we wrote to now reports a different value than we verified
    fn overridden(&self, current: &[Reading]) -> bool {
        current.iter().any(|r| {
            self.last_verified
                .iter()
                .any(|v| v.name == r.name && v.percent != r.percent)
        })
    }

    /// Decide whether `target` needs to be written given what the displays currently report
    ///
    /// A device that has vanished since the last write is ignored; one that is new counts as a
    /// user change. A known device whose brightness differs from what we verified is a user
    /// change too, unless manual changes are being respected, in which case nothing is written
    #[must_use]
    pub fn reason(&self, target: u8, current: &[Reading], force: bool) -> Option<ApplyReason> {
        if force {
            return Some(ApplyReason::Forced);
        }

        if self.last_target.is_none() {
            return Some(ApplyReason::Initial);
        }

        if self.respect_manual && self.overridden(current) {
            return None;
        }

        if self.last_target != Some(target) {
            return Some(ApplyReason::Schedule);
        }

        current
            .iter()
            .any(|r| !self.last_verified.contains(r))
            .then_some(ApplyReason::UserChange)
    }

    /// Read, decide, and (if warranted) write and verify
    ///
    /// # Errors
    /// Returns an error if the devices cannot be enumerated, if every device fails to read or to
    /// write, or if there are no devices to write to
    pub async fn step(&self, target: u8, force: bool) -> Result<Step> {
        let mut devices = backlight::devices().await.context("read failed")?;

        let seen = backlight::read(&devices)
            .await
            .require_any()
            .context("read failed")?;

        let applied = match self.reason(target, &seen.readings, force) {
            Some(reason) => Some(write_all(&mut devices, target, reason).await?),
            None => None,
        };

        Ok(Step {
            current: seen.readings,
            unreadable: seen.failures,
            applied,
        })
    }

    /// Fold a finished [`Step`] into the applier's memory
    /// A step without a write is a no-op
    pub fn record(&mut self, step: &Step) {
        if let Some(a) = &step.applied {
            self.last_target = Some(a.target);
            self.last_verified.clone_from(&a.verified);
            // A device we could not write keeps its pre-write reading, so it is not mistaken for a
            // user change on every later tick
            self.last_verified.extend(
                step.current
                    .iter()
                    .filter(|r| a.failed.iter().any(|f| f.name == r.name))
                    .cloned(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(p: u32) -> Vec<Reading> {
        vec![Reading::new("intel", p)]
    }

    fn applied(target: u8, verified: Vec<Reading>) -> Step {
        Step {
            current: Vec::new(),
            unreadable: Vec::new(),
            applied: Some(Applied {
                reason: ApplyReason::Initial,
                target,
                verified,
                failed: Vec::new(),
            }),
        }
    }

    #[test]
    fn first_step_is_initial() {
        let a = Applier::default();
        assert_eq!(a.reason(55, &at(55), false), Some(ApplyReason::Initial));
    }

    #[test]
    fn steady_state_is_quiet() {
        let mut a = Applier::default();
        a.record(&applied(55, at(55)));
        assert_eq!(a.reason(55, &at(55), false), None);
    }

    #[test]
    fn schedule_move_and_user_change_are_distinguished() {
        let mut a = Applier::default();
        a.record(&applied(55, at(55)));
        assert_eq!(a.reason(56, &at(55), false), Some(ApplyReason::Schedule));
        assert_eq!(a.reason(55, &at(70), false), Some(ApplyReason::UserChange));
    }

    #[test]
    fn force_always_writes() {
        let mut a = Applier::default();
        a.record(&applied(55, at(55)));
        assert_eq!(a.reason(55, &at(55), true), Some(ApplyReason::Forced));
    }

    #[test]
    fn reset_returns_to_initial() {
        let mut a = Applier::default();
        a.record(&applied(55, at(55)));
        a.reset();
        assert_eq!(a.reason(55, &at(55), false), Some(ApplyReason::Initial));
    }

    #[test]
    fn read_only_step_does_not_update_memory() {
        let mut a = Applier::default();
        a.record(&Step {
            current: at(10),
            unreadable: Vec::new(),
            applied: None,
        });
        assert_eq!(a.reason(55, &at(10), false), Some(ApplyReason::Initial));
    }

    #[test]
    fn reasons_render_for_logs() {
        assert_eq!(ApplyReason::UserChange.to_string(), "user change detected");
    }

    #[test]
    fn new_device_counts_as_a_change() {
        let mut a = Applier::default();
        a.record(&applied(55, at(55)));
        let mut now = at(55);
        now.push(Reading::new("hdmi", 80));
        assert_eq!(a.reason(55, &now, false), Some(ApplyReason::UserChange));
    }

    #[test]
    fn vanished_device_is_quiet() {
        let mut a = Applier::default();
        a.record(&applied(
            55,
            vec![Reading::new("a", 55), Reading::new("b", 55)],
        ));
        assert_eq!(a.reason(55, &[Reading::new("a", 55)], false), None);
    }

    #[test]
    fn write_failed_device_does_not_retrigger() {
        let mut step = applied(55, at(55));
        step.current = vec![Reading::new("intel", 30), Reading::new("hdmi", 80)];
        if let Some(a) = step.applied.as_mut() {
            a.failed = vec![Failure {
                name: "hdmi".into(),
                error: "boom".into(),
            }];
        }

        let mut a = Applier::default();
        a.record(&step);

        let now = vec![Reading::new("intel", 55), Reading::new("hdmi", 80)];
        assert_eq!(a.reason(55, &now, false), None);
    }

    fn respectful() -> Applier {
        Applier::default().respecting_manual_changes(true)
    }

    #[test]
    fn respecting_pauses_until_readings_match_again() {
        let mut a = respectful();
        a.record(&applied(55, at(55)));
        assert_eq!(a.reason(56, &at(70), false), None);
        assert_eq!(a.reason(56, &at(55), false), Some(ApplyReason::Schedule));
    }

    #[test]
    fn respecting_survives_reset_and_yields_to_force() {
        let mut a = respectful();
        a.record(&applied(55, at(55)));
        assert_eq!(a.reason(55, &at(70), true), Some(ApplyReason::Forced));

        a.reset();
        assert_eq!(a.reason(55, &at(70), false), Some(ApplyReason::Initial));
        a.record(&applied(55, at(55)));
        assert_eq!(a.reason(56, &at(70), false), None);
    }

    #[test]
    fn respecting_still_adopts_new_devices() {
        let mut a = respectful();
        a.record(&applied(55, at(55)));
        let mut now = at(55);
        now.push(Reading::new("hdmi", 80));
        assert_eq!(a.reason(55, &now, false), Some(ApplyReason::UserChange));
    }
}
