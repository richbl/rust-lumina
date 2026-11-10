//! GUI-facing editable state, kept free of GTK4 dependencies so it can be unit-tested

use crate::config::{
    self, BrightnessSection, FileConfig, LocationSection, ScheduleSection, Settings,
};
use crate::location::Location;
use anyhow::Result;

/// A form that can be edited in the GUI. This is a separate type from `Settings` so that we can have
/// a "blank" state for the start/stop times, and so we can have a separate "auto location" checkbox
#[derive(Debug, Clone, PartialEq)]
pub struct Form {
    /// Detect location via IP instead of using `latitude`/`longitude`
    pub auto_location: bool,
    /// Latitude in decimal degrees; ignored if `auto_location` is true
    pub latitude: f64,
    /// Longitude in decimal degrees; ignored if `auto_location` is true
    pub longitude: f64,
    /// Minimum brightness, in percent
    pub min: u8,
    /// Maximum brightness, in percent
    pub max: u8,
    /// Blank means "use sunrise"
    pub start: String,
    /// Blank means "use sunset"
    pub stop: String,
    /// Interval between brightness updates, in seconds
    pub interval_seconds: u64,
    /// Leave brightness alone after a manual change instead of re-asserting the schedule
    pub respect_manual_changes: bool,
}

/// Convert a blank string to None, otherwise trim and return Some
fn blank_to_none(s: &str) -> Option<String> {
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
}

/// Convert between `Form` and `Settings`
impl Form {
    /// Create a form from settings
    /// The `start` and `stop` times are formatted as HH:MM, or blank if not set
    #[must_use]
    pub fn from_settings(s: &Settings) -> Self {
        Self {
            auto_location: s.location.is_none(),
            latitude: s.location.map_or(0.0, |l| l.latitude),
            longitude: s.location.map_or(0.0, |l| l.longitude),
            min: s.min,
            max: s.max,
            start: s
                .start
                .map(|t| t.format("%H:%M").to_string())
                .unwrap_or_default(),
            stop: s
                .stop
                .map(|t| t.format("%H:%M").to_string())
                .unwrap_or_default(),
            interval_seconds: s.interval_seconds,
            respect_manual_changes: s.respect_manual_changes,
        }
    }

    /// The on-disk representation of the form, without validation
    fn file_config(&self) -> FileConfig {
        FileConfig {
            location: (!self.auto_location).then_some(LocationSection {
                latitude: Some(self.latitude),
                longitude: Some(self.longitude),
            }),
            brightness: Some(BrightnessSection {
                min: Some(self.min),
                max: Some(self.max),
            }),
            schedule: Some(ScheduleSection {
                start_time: blank_to_none(&self.start),
                stop_time: blank_to_none(&self.stop),
                interval_seconds: Some(self.interval_seconds),
                respect_manual_changes: self.respect_manual_changes.then_some(true),
            }),
        }
    }

    /// Convert the form into a `FileConfig`, which can be serialized to disk. This will fail if
    /// the form is invalid (e.g. min > max, invalid times, etc.)
    ///
    /// # Errors
    /// Returns an error if the form is invalid, e.g. min > max, invalid times, etc.
    pub fn to_file_config(&self) -> Result<FileConfig> {
        let cfg = self.file_config();
        config::resolve(cfg.clone())?;

        Ok(cfg)
    }

    /// Convert the form into a `Settings`, which can be used to control the brightness. This will
    /// fail if the form is invalid (e.g. min > max, invalid times, etc.)
    ///
    /// # Errors
    /// Returns an error if the form is invalid, e.g. min > max, invalid times, etc.
    pub fn to_settings(&self, detected: Option<Location>) -> Result<Settings> {
        let mut s = config::resolve(self.file_config())?;
        if self.auto_location {
            s.location = detected;
        }

        Ok(s)
    }

    /// The location in effect: the detected one when auto-detect is on, otherwise the manual
    /// one, or `None` if that is out of range
    #[must_use]
    pub fn location(&self, detected: Option<Location>) -> Option<Location> {
        if self.auto_location {
            detected
        } else {
            Location::new(self.latitude, self.longitude).ok()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn form() -> Form {
        Form {
            auto_location: false,
            latitude: 47.98,
            longitude: -122.2,
            min: 20,
            max: 90,
            start: String::new(),
            stop: " 20:30 ".into(),
            interval_seconds: 60,
            respect_manual_changes: false,
        }
    }

    #[test]
    fn file_config_omits_auto_location_and_blank_times() {
        let mut f = form();
        f.auto_location = true;
        let cfg = f.to_file_config().unwrap();
        assert!(cfg.location.is_none());
        let sched = cfg.schedule.unwrap();
        assert_eq!(sched.start_time, None);
        assert_eq!(sched.stop_time.as_deref(), Some("20:30"));
    }

    #[test]
    fn settings_roundtrip_through_form() {
        let s = form().to_settings(None).unwrap();
        assert_eq!(
            Form::from_settings(&s),
            Form {
                stop: "20:30".into(),
                ..form()
            }
        );
    }

    #[test]
    fn auto_location_uses_detected() {
        let mut f = form();
        f.auto_location = true;
        let d = Location::new(10.0, 20.0).unwrap();
        assert_eq!(f.to_settings(Some(d)).unwrap().location, Some(d));
        assert_eq!(f.to_settings(None).unwrap().location, None);
    }

    #[test]
    fn validation_errors() {
        let bad = |f: Form| f.to_file_config().is_err();
        assert!(bad(Form {
            min: 80,
            max: 30,
            ..form()
        }));
        assert!(bad(Form { max: 101, ..form() }));
        assert!(bad(Form {
            start: "6am".into(),
            ..form()
        }));
        assert!(bad(Form {
            latitude: 123.0,
            ..form()
        }));
        assert!(bad(Form {
            interval_seconds: 0,
            ..form()
        }));
        assert!(bad(Form {
            interval_seconds: 3601,
            ..form()
        }));
        // An out-of-range manual location is ignored while auto-detect is on
        assert!(!bad(Form {
            latitude: 123.0,
            auto_location: true,
            ..form()
        }));
    }

    #[test]
    fn respect_manual_changes_roundtrips_and_is_omitted_when_off() {
        let on = Form {
            respect_manual_changes: true,
            ..form()
        };
        let cfg = on.to_file_config().unwrap();
        assert_eq!(cfg.schedule.unwrap().respect_manual_changes, Some(true));
        assert!(Form::from_settings(&on.to_settings(None).unwrap()).respect_manual_changes);

        let off = form().to_file_config().unwrap();
        assert_eq!(off.schedule.unwrap().respect_manual_changes, None);
    }

    #[test]
    fn location_follows_mode_and_range() {
        let d = Location::new(10.0, 20.0).unwrap();
        let mut f = form();
        assert_eq!(f.location(Some(d)), Location::new(47.98, -122.2).ok());
        f.latitude = 123.0;
        assert_eq!(f.location(Some(d)), None);
        f.auto_location = true;
        assert_eq!(f.location(Some(d)), Some(d));
        assert_eq!(f.location(None), None);
    }

    #[test]
    fn identical_start_and_stop_are_rejected() {
        let f = Form {
            start: "06:00".into(),
            stop: "06:00".into(),
            ..form()
        };
        assert!(f.to_file_config().is_err());
    }
}
