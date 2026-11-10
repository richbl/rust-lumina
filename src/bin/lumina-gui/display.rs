//! Utility functions for the GUI, including formatting and validation helpers

use chrono::NaiveTime;
use lumina::backlight::Reading;
use lumina::schedule::Window;

/// Format a list of `(display_name, brightness)` readings as a human-readable string. If the list
/// is empty, return a placeholder message
pub(crate) fn fmt_readings(v: &[Reading]) -> String {
    if v.is_empty() {
        "No controllable display found".into()
    } else {
        v.iter()
            .map(|r| format!("{}% on display {}", r.percent, r.name))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Format a `NaiveTime` as `HH:MM` (24-hour) for display in the GUI
pub(crate) fn fmt_hm(t: NaiveTime) -> String {
    t.format("%H:%M").to_string()
}

/// Return the sunrise/sunset times and a human-readable summary for a given `Window`
pub(crate) fn solar_parts(w: Option<Window>) -> (Option<(NaiveTime, NaiveTime)>, String) {
    match w {
        None => (None, "—".into()),
        Some(Window::AlwaysDay) => (None, "Sun is up all day".into()),
        Some(Window::AlwaysNight) => (None, "Sun is down all day".into()),
        Some(Window::Range { start, stop }) => (
            Some((start, stop)),
            format!("Sunrise:\t{}\nSunset:\t{}", fmt_hm(start), fmt_hm(stop)),
        ),
    }
}

/// Return true if the given string is a valid `HH:MM` time (24-hour) for display in the GUI
pub(crate) fn time_valid(s: &str) -> bool {
    NaiveTime::parse_from_str(s.trim(), "%H:%M").is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solar_range_exposes_times_and_summary() {
        let (times, text) = solar_parts(Some(Window::Range {
            start: NaiveTime::from_hms_opt(6, 15, 0).unwrap(),
            stop: NaiveTime::from_hms_opt(18, 45, 0).unwrap(),
        }));

        assert_eq!(
            times,
            Some((
                NaiveTime::from_hms_opt(6, 15, 0).unwrap(),
                NaiveTime::from_hms_opt(18, 45, 0).unwrap()
            ))
        );
        assert_eq!(text, "Sunrise:\t06:15\nSunset:\t18:45");
    }

    #[test]
    fn polar_windows_have_distinct_text_and_no_times() {
        let (t1, day) = solar_parts(Some(Window::AlwaysDay));
        let (t2, night) = solar_parts(Some(Window::AlwaysNight));
        assert!(t1.is_none() && t2.is_none());
        assert_ne!(day, night);
    }

    #[test]
    fn unknown_window_is_placeholder() {
        assert_eq!(solar_parts(None), (None, "—".to_string()));
    }

    #[test]
    fn time_valid_accepts_hh_mm() {
        assert!(time_valid("06:30"));
        assert!(time_valid(" 23:59 "));
    }

    #[test]
    fn time_valid_rejects_blank_and_garbage() {
        assert!(!time_valid(""));
        assert!(!time_valid("   "));
        assert!(!time_valid("abc"));
        assert!(!time_valid("25:00"));
        assert!(!time_valid("12:61"));
        assert!(!time_valid("12"));
    }
}
