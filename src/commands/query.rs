//! Read-only commands: `get`, `location`, `suntime`, `daytime`, and `status`

use super::{load_with_location, resolve_location};
use crate::cli::ConfigArg;
use crate::output::{fmt_loc, report_failures};
use anyhow::Result;
use chrono::Local;
use lumina::backlight;
use lumina::config;
use lumina::plan;
use lumina::schedule::Window;
use lumina::solar::{SunTimes, sun_times};

/// A day's span as the `suntime` and `daytime` commands show it: two clock times, or a polar
/// day/night with nothing to show
enum Span {
    /// The two times, already formatted as `HH:MM`
    Times { first: String, second: String },
    /// The sun never sets
    AlwaysDay,
    /// The sun never rises
    AlwaysNight,
}

impl From<SunTimes<Local>> for Span {
    fn from(t: SunTimes<Local>) -> Self {
        match t {
            SunTimes::Normal { sunrise, sunset } => Self::Times {
                first: sunrise.format("%H:%M").to_string(),
                second: sunset.format("%H:%M").to_string(),
            },
            SunTimes::PolarDay => Self::AlwaysDay,
            SunTimes::PolarNight => Self::AlwaysNight,
        }
    }
}

impl From<Window> for Span {
    fn from(w: Window) -> Self {
        match w {
            Window::Range { start, stop } => Self::Times {
                first: start.format("%H:%M").to_string(),
                second: stop.format("%H:%M").to_string(),
            },
            Window::AlwaysDay => Self::AlwaysDay,
            Window::AlwaysNight => Self::AlwaysNight,
        }
    }
}

impl Span {
    /// The output lines, with `labels` (padding included) prefixed to the two times
    fn lines(&self, labels: [&str; 2]) -> Vec<String> {
        match self {
            Self::Times { first, second } => {
                vec![
                    format!("{}{first}", labels[0]),
                    format!("{}{second}", labels[1]),
                ]
            }
            Self::AlwaysDay => vec!["Sun does not set today.".to_owned()],
            Self::AlwaysNight => vec!["Sun does not rise today.".to_owned()],
        }
    }

    /// Print the output lines to stdout
    fn print(&self, labels: [&str; 2]) {
        for line in self.lines(labels) {
            outln!("{line}");
        }
    }
}

/// Handle the `get` command by printing the current brightness of each device display
pub(super) async fn handle_get() -> Result<()> {
    let out = backlight::get_all().await?;
    if out.is_empty() {
        errln!("No brightness-controllable devices found.");
    }

    for r in &out.readings {
        outln!("{}% on display {}", r.percent, r.name);
    }

    report_failures(&out.failures)
}

/// Handle the `location` command by printing the configured or detected location
pub(super) fn handle_location(args: &ConfigArg) -> Result<()> {
    let (loc, src) = resolve_location(&config::load(args.path())?)?;
    outln!("{} ({src})", fmt_loc(&loc));

    Ok(())
}

/// Handle the `suntime` command by printing today's sunrise and sunset times for the configured or
/// detected location
pub(super) fn handle_suntime(args: &ConfigArg) -> Result<()> {
    let (loc, _) = resolve_location(&config::load(args.path())?)?;
    let today = Local::now().date_naive();

    Span::from(sun_times(today, loc.latitude, loc.longitude, &Local))
        .print(["Sunrise:  ", "Sunset:   "]);

    Ok(())
}

/// Handle the `daytime` command by printing the effective day start and end times
///
/// If both `start_time` and `stop_time` are configured, those values are used directly
/// Otherwise the computed sunrise/sunset (optionally overridden by a single configured time)
/// for the current location is used
pub(super) fn handle_daytime(args: &ConfigArg) -> Result<()> {
    let settings = load_with_location(args.path())?;
    let today = Local::now().date_naive();

    Span::from(plan::window_for(&settings, today, &Local)?)
        .print(["Day starts:  ", "Day ends:    "]);

    Ok(())
}

/// Handle the `status` command by printing the target brightness for the current time
pub(super) fn handle_status(args: &ConfigArg) -> Result<()> {
    let settings = load_with_location(args.path())?;
    let target = plan::brightness_at(&settings, &Local::now())?;

    outln!("Target display brightness: {target}%");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const LABELS: [&str; 2] = ["A:  ", "B:  "];

    #[test]
    fn times_are_labeled_in_order() {
        let span = Span::Times {
            first: "06:30".to_owned(),
            second: "18:45".to_owned(),
        };
        assert_eq!(span.lines(LABELS), ["A:  06:30", "B:  18:45"]);
    }

    #[test]
    fn polar_cases_ignore_labels() {
        assert_eq!(Span::AlwaysDay.lines(LABELS), ["Sun does not set today."]);
        assert_eq!(
            Span::AlwaysNight.lines(LABELS),
            ["Sun does not rise today."]
        );
    }
}
