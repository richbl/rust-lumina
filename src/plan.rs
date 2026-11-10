//! Combine settings + solar data into today's "brightness window"

use crate::config::Settings;
use crate::location::Location;
use crate::schedule::{Window, target_brightness};
use crate::solar::{SunTimes, sun_times};
use anyhow::{Result, bail};
use chrono::{DateTime, NaiveDate, TimeZone};

/// The sunrise-to-sunset window for `date`, ignoring any start/stop overrides
///
/// # Errors
/// Returns an error if no location is given
pub fn solar_window<Tz: TimeZone>(
    location: Option<Location>,
    date: NaiveDate,
    tz: &Tz,
) -> Result<Window> {
    let Some(loc) = location else {
        bail!(
            "location required to compute sunrise/sunset: set latitude/longitude in the config, set both start_time and stop_time, or allow IP lookup"
        );
    };

    Ok(match sun_times(date, loc.latitude, loc.longitude, tz) {
        SunTimes::PolarDay => Window::AlwaysDay,
        SunTimes::PolarNight => Window::AlwaysNight,
        SunTimes::Normal { sunrise, sunset } => Window::Range {
            start: sunrise.time(),
            stop: sunset.time(),
        },
    })
}

/// Resolve the daytime window for `date`. User-supplied start/stop overrides the computed
/// sunrise/sunset individually. When the sun does not rise or set that day, a one-sided override
/// has nothing to pair with, so the polar "brightness window" stands
///
/// # Errors
/// Returns an error if the location is not set and sunrise/sunset are needed
pub fn window_for<Tz: TimeZone>(s: &Settings, date: NaiveDate, tz: &Tz) -> Result<Window> {
    if let (Some(start), Some(stop)) = (s.start, s.stop) {
        return Ok(Window::Range { start, stop });
    }

    Ok(match solar_window(s.location, date, tz)? {
        Window::Range { start, stop } => Window::Range {
            start: s.start.unwrap_or(start),
            stop: s.stop.unwrap_or(stop),
        },
        polar @ (Window::AlwaysDay | Window::AlwaysNight) => polar,
    })
}

/// Compute the target brightness at `now` from the resolved window and the min/max range
///
/// # Errors
/// Returns an error if the location is not set and sunrise/sunset are needed
pub fn brightness_at<Tz: TimeZone>(s: &Settings, now: &DateTime<Tz>) -> Result<u8> {
    let w = window_for(s, now.date_naive(), &now.timezone())?;
    Ok(target_brightness(now.time(), w, s.min, s.max))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{parse_file, resolve};
    use chrono::{FixedOffset, NaiveTime};

    fn tz() -> FixedOffset {
        FixedOffset::west_opt(7 * 3600).unwrap()
    }

    fn at(h: u32, m: u32) -> DateTime<FixedOffset> {
        tz().with_ymd_and_hms(2026, 6, 21, h, m, 0).unwrap()
    }

    fn settings(toml: &str) -> Settings {
        resolve(parse_file(toml).unwrap()).unwrap()
    }

    // Home sweet home...
    const SEATTLE: &str = "[location]\nlatitude = 47.98\nlongitude = -122.2\n";

    #[test]
    fn computed_from_location() {
        let s = settings(SEATTLE);

        // Solar noon is ~13:10 local; midday is close to max
        let noon = brightness_at(&s, &at(13, 10)).unwrap();
        assert!((88..=90).contains(&noon), "noon {noon}");
        assert_eq!(brightness_at(&s, &at(2, 0)).unwrap(), 20);
        assert_eq!(brightness_at(&s, &at(23, 0)).unwrap(), 20);
    }

    #[test]
    fn manual_times_need_no_location() {
        let s = settings("[schedule]\nstart_time = \"08:00\"\nstop_time = \"18:00\"\n");

        assert_eq!(brightness_at(&s, &at(7, 59)).unwrap(), 20);
        assert_eq!(brightness_at(&s, &at(8, 0)).unwrap(), 20);
        assert_eq!(brightness_at(&s, &at(13, 0)).unwrap(), 90);
        assert_eq!(brightness_at(&s, &at(18, 0)).unwrap(), 20);
    }

    #[test]
    fn override_only_start_keeps_computed_sunset() {
        let s = settings(&format!("{SEATTLE}\n[schedule]\nstart_time = \"09:00\"\n"));

        match window_for(&s, at(0, 0).date_naive(), &tz()).unwrap() {
            Window::Range { start, stop } => {
                assert_eq!(start, NaiveTime::from_hms_opt(9, 0, 0).unwrap());
                assert!(stop.format("%H").to_string() == "21"); // ~21:10 sunset
            }
            other => assert!(
                matches!(other, Window::Range { .. }),
                "unexpected {other:?}"
            ),
        }
    }

    #[test]
    fn missing_location_is_an_error() {
        let s = settings("");

        assert!(brightness_at(&s, &at(12, 0)).is_err());
    }

    #[test]
    fn polar_summer_stays_bright() {
        let s = settings("[location]\nlatitude = 78.2\nlongitude = 15.6\n");

        assert_eq!(brightness_at(&s, &at(3, 0)).unwrap(), 90);
    }

    #[test]
    fn solar_window_ignores_overrides() {
        let date = at(0, 0).date_naive();
        assert!(solar_window(None, date, &tz()).is_err());

        let s = settings(&format!("{SEATTLE}\n[schedule]\nstart_time = \"09:00\"\n"));
        let sun = solar_window(s.location, date, &tz()).unwrap();
        assert_ne!(sun, window_for(&s, date, &tz()).unwrap());
    }

    #[test]
    fn one_sided_override_yields_to_polar_windows() {
        let z = tz();
        let summer = NaiveDate::from_ymd_opt(2026, 6, 21).unwrap();
        let winter = NaiveDate::from_ymd_opt(2026, 12, 21).unwrap();
        let pole = "[location]\nlatitude = 78.2\nlongitude = 15.6\n";

        let s = settings(&format!("{pole}\n[schedule]\nstart_time = \"09:00\"\n"));
        assert_eq!(window_for(&s, summer, &z).unwrap(), Window::AlwaysDay);

        let s = settings(&format!("{pole}\n[schedule]\nstop_time = \"18:00\"\n"));
        assert_eq!(window_for(&s, winter, &z).unwrap(), Window::AlwaysNight);
    }
}
