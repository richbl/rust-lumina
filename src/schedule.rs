//! Maps the time of day to a target brightness percentage
//!
//! Brightness follows a diurnal curve: a half-sine arc across the daytime window. It sits at `min`
//! at the start time, rises fastest around dawn, levels off and peaks at `max` at the midpoint of
//! the window (solar noon), then eases back to `min` at the stop time and holds at `min` overnight

use chrono::{NaiveTime, Timelike};
use std::f64::consts::PI;

/// The "daytime" window over which brightness rises to `max` and falls back
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Window {
    /// Sun never sets: stay at maximum
    AlwaysDay,
    /// Sun never rises: stay at minimum
    AlwaysNight,
    /// Daytime runs from `start` to `stop` (may cross midnight)
    Range {
        /// The start of the daytime window
        start: NaiveTime,
        /// The end of the daytime window
        stop: NaiveTime,
    },
}

/// Seconds in a day... I feel a song coming on...
const DAY: i64 = 86_400;

/// Convert a `NaiveTime` to seconds since midnight
fn secs(t: NaiveTime) -> i64 {
    i64::from(t.num_seconds_from_midnight())
}

/// Compute the target brightness for a given time of day, daytime window, and min/max brightness
/// The result always lies within `min..=max` and never exceeds 100; a `max` below `min` is treated
/// as `min`. A zero-length window (`start == stop`) has no daylight and holds `min`
#[must_use]
pub fn target_brightness(now: NaiveTime, window: Window, min: u8, max: u8) -> u8 {
    let min = min.min(100);
    let max = max.clamp(min, 100);

    let (start, stop) = match window {
        Window::AlwaysDay => return max,
        Window::AlwaysNight => return min,
        Window::Range { start, stop } => (secs(start), secs(stop)),
    };

    let day_len = (stop - start).rem_euclid(DAY);
    if day_len == 0 {
        return min;
    }

    let t = (secs(now) - start).rem_euclid(DAY);
    if t >= day_len {
        return min;
    }

    let frac = {
        #[allow(clippy::cast_precision_loss)]
        let numerator = t as f64;
        #[allow(clippy::cast_precision_loss)]
        let denominator = day_len as f64;
        numerator / denominator
    };

    let (lo, hi) = (f64::from(min), f64::from(max));
    let brightness = (lo + (hi - lo) * (PI * frac).sin()).round().clamp(lo, hi);

    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    let brightness = brightness as u8;
    brightness
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }
    fn win() -> Window {
        Window::Range {
            start: t(6, 0),
            stop: t(18, 0),
        }
    }
    fn at(h: u32, m: u32) -> u8 {
        target_brightness(t(h, m), win(), 20, 90)
    }

    #[test]
    fn min_at_edges_and_overnight_max_at_midpoint() {
        assert_eq!(at(6, 0), 20);
        assert_eq!(at(18, 0), 20);
        assert_eq!(at(0, 0), 20);
        assert_eq!(at(23, 59), 20);
        assert_eq!(at(12, 0), 90);
    }

    #[test]
    fn follows_a_sine_curve() {
        // Quarter of the way through the day: 20 + 70*sin(pi/4) = 69.5
        assert_eq!(at(9, 0), 69);
        assert_eq!(at(15, 0), 69);

        // One hour in: 20 + 70*sin(pi/12) = 38.1
        assert_eq!(at(7, 0), 38);
    }

    #[test]
    fn rises_then_falls_and_is_symmetric() {
        let mut prev = 0;

        for m in (6 * 60)..=(12 * 60) {
            let v = at(m / 60, m % 60);
            assert!(v >= prev, "not rising at {m}");
            prev = v;
        }

        for m in (12 * 60)..=(18 * 60) {
            let v = at(m / 60, m % 60);
            assert!(v <= prev, "not falling at {m}");
            prev = v;
        }

        assert_eq!(at(8, 0), at(16, 0));
    }

    #[test]
    fn rate_of_change_is_steep_at_dawn_and_flat_at_noon() {
        let dawn = i32::from(at(7, 0)) - i32::from(at(6, 0));
        let noon = i32::from(at(12, 0)) - i32::from(at(11, 0));

        assert!(dawn > 5 * noon.max(1));
    }

    #[test]
    fn window_crossing_midnight() {
        let w = Window::Range {
            start: t(22, 0),
            stop: t(4, 0),
        };

        // midpoint
        assert_eq!(target_brightness(t(1, 0), w, 20, 90), 90);
        assert_eq!(target_brightness(t(12, 0), w, 20, 90), 20);
    }

    #[test]
    fn polar_windows() {
        assert_eq!(target_brightness(t(3, 0), Window::AlwaysDay, 20, 90), 90);
        assert_eq!(target_brightness(t(12, 0), Window::AlwaysNight, 20, 90), 20);
    }

    #[test]
    fn bad_inputs_stay_in_bounds() {
        for h in 0..24 {
            assert_eq!(target_brightness(t(h, 0), win(), 50, 50), 50);
        }

        let same = Window::Range {
            start: t(6, 0),
            stop: t(6, 0),
        };

        assert_eq!(target_brightness(t(3, 0), same, 20, 90), 20);

        for m in 0..(24 * 60) {
            assert!((0..=100).contains(&target_brightness(t(m / 60, m % 60), win(), 0, 100)));
        }
    }

    #[test]
    fn output_is_clamped_to_min_max() {
        for m in 0..(24 * 60) {
            assert_eq!(target_brightness(t(m / 60, m % 60), win(), 80, 30), 80);
        }
        assert_eq!(target_brightness(t(12, 0), win(), 20, 200), 100);
        assert_eq!(target_brightness(t(12, 0), Window::AlwaysDay, 20, 200), 100);
        assert_eq!(
            target_brightness(t(0, 0), Window::AlwaysNight, 150, 200),
            100
        );
    }
}
