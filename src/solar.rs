//! Sunrise/sunset calculations
//!
//! Computed locally with the standard "sunrise equation" (accurate to ~1-2 min), so no network is
//! needed once the location is known

use chrono::{DateTime, Duration, NaiveDate, TimeZone};

/// Sunrise and sunset times for a local calendar day
#[derive(Debug, Clone, PartialEq)]
pub enum SunTimes<Tz: TimeZone> {
    /// Normal day with both sunrise and sunset times
    Normal {
        /// Time of sunrise for the local day
        sunrise: DateTime<Tz>,
        /// Time of sunset for the local day
        sunset: DateTime<Tz>,
    },
    /// Sun never sets (midnight sun)
    PolarDay,
    /// Sun never rises (nuclear winter!)
    PolarNight,
}

/// Dates in Julian date (_jd)
enum Raw {
    Normal { rise_jd: f64, set_jd: f64 },
    PolarDay,
    PolarNight,
}

const J2000: f64 = 2_451_545.0;
const UNIX_EPOCH_JD: f64 = 2_440_587.5;

/// Compute the raw sunrise/sunset times for a given day (in days since the Unix epoch) and
/// location
fn raw_for_day(days_since_unix_epoch: i64, lat: f64, lon: f64) -> Raw {
    // Days since J2000 for the UTC noon of that date
    #[allow(clippy::cast_precision_loss)]
    let n = (days_since_unix_epoch - 10_957) as f64;

    let j_star = n + 0.0009 - lon / 360.0;
    let m = (357.5291 + 0.985_600_28 * j_star)
        .rem_euclid(360.0)
        .to_radians();

    let c = 1.9148 * m.sin() + 0.02 * (2.0 * m).sin() + 0.0003 * (3.0 * m).sin();

    let lambda = (m.to_degrees() + c + 180.0 + 102.9372)
        .rem_euclid(360.0)
        .to_radians();

    let j_transit = J2000 + j_star + 0.0053 * m.sin() - 0.0069 * (2.0 * lambda).sin();

    let sin_dec = lambda.sin() * 23.4397_f64.to_radians().sin();
    let cos_dec = (1.0 - sin_dec * sin_dec).sqrt();
    let phi = lat.to_radians();
    let cos_w = ((-0.833_f64).to_radians().sin() - phi.sin() * sin_dec) / (phi.cos() * cos_dec);

    if cos_w > 1.0 {
        Raw::PolarNight
    } else if cos_w < -1.0 {
        Raw::PolarDay
    } else {
        let w = cos_w.acos().to_degrees();
        Raw::Normal {
            rise_jd: j_transit - w / 360.0,
            set_jd: j_transit + w / 360.0,
        }
    }
}

/// Convert a Julian date to a `DateTime` in the given timezone
fn jd_to_datetime<Tz: TimeZone>(jd: f64, tz: &Tz) -> DateTime<Tz> {
    // Convert to seconds since the Unix epoch, rounding to the nearest second
    #[allow(clippy::cast_possible_truncation)]
    let secs = ((jd - UNIX_EPOCH_JD) * 86_400.0).round() as i64;
    tz.timestamp_opt(secs, 0).single().expect("valid timestamp")
}

const UNIX_EPOCH_DATE: NaiveDate =
    NaiveDate::from_ymd_opt(1970, 1, 1).expect("Unix epoch date must be valid");

/// Sunrise/sunset for the given *local* calendar date at (lat, lon) in degrees (north/east
/// positive), expressed in `tz`
fn day_times<Tz: TimeZone>(day: i64, lat: f64, lon: f64, tz: &Tz) -> SunTimes<Tz> {
    match raw_for_day(day, lat, lon) {
        Raw::Normal { rise_jd, set_jd } => SunTimes::Normal {
            sunrise: jd_to_datetime(rise_jd, tz),
            sunset: jd_to_datetime(set_jd, tz),
        },
        Raw::PolarDay => SunTimes::PolarDay,
        Raw::PolarNight => SunTimes::PolarNight,
    }
}

/// Index of the candidate days to use, given each candidate's distance in days between its sunrise
/// and the target date (`None` for polar candidates): an exact hit, else the first polar one, else
/// the nearest, with ties going to the earliest
fn pick(gaps: [Option<i64>; 3]) -> usize {
    gaps.iter()
        .position(|g| *g == Some(0))
        .or_else(|| gaps.iter().position(Option::is_none))
        .or_else(|| {
            gaps.iter()
                .enumerate()
                .filter_map(|(i, g)| g.map(|g| (g, i)))
                .min()
                .map(|(_, i)| i)
        })
        .unwrap_or(0)
}

/// Choose which of the candidate days describes the local `date`
fn choose_for_date<Tz: TimeZone>(date: NaiveDate, days: [SunTimes<Tz>; 3]) -> SunTimes<Tz> {
    let gaps = days.each_ref().map(|t| match t {
        SunTimes::Normal { sunrise, .. } => Some((sunrise.date_naive() - date).num_days().abs()),
        _ => None,
    });

    // The fallback is unreachable: `pick` always returns an index in range
    days.into_iter()
        .nth(pick(gaps))
        .unwrap_or(SunTimes::PolarNight)
}

/// Near the date line the UTC-based event can land on a neighboring local date, so the adjacent
/// days are computed too. If no sunrise falls on `date` (possible near the polar transitions),
/// a polar result is used when a neighbor has one, otherwise the nearest day's times
pub fn sun_times<Tz: TimeZone>(date: NaiveDate, lat: f64, lon: f64, tz: &Tz) -> SunTimes<Tz> {
    let base = (date - UNIX_EPOCH_DATE).num_days();
    let days = [0_i64, -1, 1].map(|off| day_times(base + off, lat, lon, tz));

    choose_for_date(date, days)
}

/// Compute the length of the day (sunrise to sunset) for a given `SunTimes` value
pub fn day_length<Tz: TimeZone>(t: &SunTimes<Tz>) -> Duration {
    match t {
        SunTimes::Normal { sunrise, sunset } => sunset.clone() - sunrise.clone(),
        SunTimes::PolarDay => Duration::hours(24),
        SunTimes::PolarNight => Duration::zero(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{FixedOffset, Timelike};

    fn within(actual: DateTime<FixedOffset>, h: u32, m: u32, tol_min: i64) -> bool {
        let expect = actual.date_naive().and_hms_opt(h, m, 0).unwrap();
        (actual.naive_local() - expect).num_minutes().abs() <= tol_min
    }

    #[test]
    fn seattle_summer_solstice() {
        // Everett WA area: sunrise ~05:11 PDT, sunset ~21:10 PDT on 2026-06-21
        let tz = FixedOffset::west_opt(7 * 3600).unwrap();
        let d = NaiveDate::from_ymd_opt(2026, 6, 21).unwrap();
        match sun_times(d, 47.98, -122.20, &tz) {
            SunTimes::Normal { sunrise, sunset } => {
                assert!(within(sunrise, 5, 11, 6), "sunrise {sunrise}");
                assert!(within(sunset, 21, 10, 6), "sunset {sunset}");
            }
            other => assert!(
                matches!(other, SunTimes::Normal { .. }),
                "unexpected {other:?}"
            ),
        }
    }

    #[test]
    fn seattle_winter_solstice() {
        // ~07:54 PST / ~16:20 PST
        let tz = FixedOffset::west_opt(8 * 3600).unwrap();
        let d = NaiveDate::from_ymd_opt(2026, 12, 21).unwrap();
        match sun_times(d, 47.98, -122.20, &tz) {
            SunTimes::Normal { sunrise, sunset } => {
                assert!(within(sunrise, 7, 54, 6), "sunrise {sunrise}");
                assert!(within(sunset, 16, 20, 6), "sunset {sunset}");
            }
            other => assert!(
                matches!(other, SunTimes::Normal { .. }),
                "unexpected {other:?}"
            ),
        }
    }

    #[test]
    fn equator_equinox_is_about_twelve_hours() {
        let tz = FixedOffset::east_opt(0).unwrap();
        let d = NaiveDate::from_ymd_opt(2026, 3, 20).unwrap();
        let t = sun_times(d, 0.0, 0.0, &tz);
        let len = day_length(&t).num_minutes();
        assert!((len - 727).abs() < 8, "len {len}"); // ~12h07 with refraction
    }

    #[test]
    fn polar_cases() {
        let tz = FixedOffset::east_opt(3600).unwrap();
        let summer = NaiveDate::from_ymd_opt(2026, 6, 21).unwrap();
        let winter = NaiveDate::from_ymd_opt(2026, 12, 21).unwrap();
        assert_eq!(sun_times(summer, 78.2, 15.6, &tz), SunTimes::PolarDay);
        assert_eq!(sun_times(winter, 78.2, 15.6, &tz), SunTimes::PolarNight);
    }

    #[test]
    fn far_east_longitude_keeps_local_date() {
        let tz = FixedOffset::east_opt(13 * 3600).unwrap();
        let d = NaiveDate::from_ymd_opt(2026, 1, 15).unwrap();
        match sun_times(d, -36.85, 174.76, &tz) {
            SunTimes::Normal { sunrise, sunset } => {
                assert_eq!(sunrise.date_naive(), d);
                assert!(sunrise.hour() >= 5 && sunrise.hour() <= 7);
                assert!(sunset > sunrise);
            }
            other => assert!(
                matches!(other, SunTimes::Normal { .. }),
                "unexpected {other:?}"
            ),
        }
    }

    fn normal_on(date: NaiveDate) -> SunTimes<FixedOffset> {
        let tz = FixedOffset::east_opt(0).unwrap();
        let at = |h| tz.from_utc_datetime(&date.and_hms_opt(h, 0, 0).unwrap());
        SunTimes::Normal {
            sunrise: at(6),
            sunset: at(18),
        }
    }

    fn sunrise_date(t: &SunTimes<FixedOffset>) -> Option<NaiveDate> {
        match t {
            SunTimes::Normal { sunrise, .. } => Some(sunrise.date_naive()),
            _ => None,
        }
    }

    #[test]
    fn a_sunrise_on_the_date_wins() {
        let d = NaiveDate::from_ymd_opt(2026, 3, 10).unwrap();
        let day = |n| d + Duration::days(n);
        let r = choose_for_date(
            d,
            [SunTimes::PolarDay, normal_on(day(-1)), normal_on(day(0))],
        );
        assert_eq!(sunrise_date(&r), Some(d));
    }

    #[test]
    fn no_sunrise_on_the_date_prefers_polar_then_nearest() {
        let d = NaiveDate::from_ymd_opt(2026, 3, 10).unwrap();
        let day = |n| d + Duration::days(n);

        let r = choose_for_date(
            d,
            [normal_on(day(1)), SunTimes::PolarNight, normal_on(day(-1))],
        );
        assert_eq!(r, SunTimes::PolarNight);

        let r = choose_for_date(
            d,
            [normal_on(day(2)), normal_on(day(-1)), normal_on(day(3))],
        );
        assert_eq!(sunrise_date(&r), Some(day(-1)));
    }
}
