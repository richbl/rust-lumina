//! CLI output: write-error-tolerant print macros, formatting helpers, and result reporting

use anyhow::Result;
use chrono::Local;
use lumina::backlight::{self, Failure, Outcome, Reading};
use lumina::engine::Step;
use lumina::location::Location;

/// A simple`println!` that ignores write errors, so a closed terminal or pipe won't kill the
/// process
macro_rules! outln {
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        let _ = writeln!(std::io::stdout(), $($arg)*);
    }};
}

/// An `eprintln!` macro that ignores write errors
macro_rules! errln {
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        let _ = writeln!(std::io::stderr(), $($arg)*);
    }};
}

/// Format a `Location` as a string with 4 decimal places of precision
pub(crate) fn fmt_loc(l: &Location) -> String {
    format!("{:.4}, {:.4}", l.latitude, l.longitude)
}

/// Format a list of `Reading` as a string, e.g. `intel_backlight 55%, nvidia_0 40%`
fn fmt_devices(v: &[Reading]) -> String {
    v.iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Format the current local time as a string for logging
pub(crate) fn ts() -> impl std::fmt::Display {
    Local::now().format("%F %T")
}

/// Output any failures to stderr and then report the failure
pub(crate) fn report_failures(failures: &[Failure]) -> Result<()> {
    for f in failures {
        errln!("{f}");
    }

    match failures.len() {
        0 => Ok(()),
        n => anyhow::bail!("{n} device(s) failed"),
    }
}

/// Report the results of a `set` operation: successes to stdout, failures to stderr
pub(crate) fn report_set(target: u8, out: &Outcome) -> Result<()> {
    for r in &out.readings {
        outln!(
            "Set to {target}% on display {}; verified at {}%",
            r.name,
            r.percent
        );
    }

    report_failures(&out.failures)
}

/// Report device values, including any unreadable devices
fn status_line(step: &Step, target: u8) -> String {
    let devices = fmt_devices(&step.current);

    if step.unreadable.is_empty() {
        format!("{devices} (target {target}%)")
    } else {
        format!(
            "{devices} (target {target}%); unreadable: {}",
            backlight::fmt_failures(&step.unreadable)
        )
    }
}

/// Logs run-loop steps, suppressing duplicate status lines
#[derive(Default)]
pub(crate) struct StepLogger {
    /// Last no-op status line, so repeats aren't logged
    quiet: Option<String>,
}

impl StepLogger {
    /// Forget the last status line, so the next one is logged even if unchanged
    pub(crate) fn reset(&mut self) {
        self.quiet = None;
    }

    /// Log one step. Writes are always logged, and status (only when they change)
    pub(crate) fn log(&mut self, step: &Step, target: u8) {
        if let Some(a) = &step.applied {
            outln!(
                "{}: [{}]: was {} -> target {}%, now {}",
                ts(),
                a.reason,
                fmt_devices(&step.current),
                a.target,
                fmt_devices(&a.verified)
            );

            for f in step.unreadable.iter().chain(&a.failed) {
                errln!("{}: {f}", ts());
            }

            self.quiet = None;
        } else {
            let line = status_line(step, target);
            if self.quiet.as_deref() != Some(line.as_str()) {
                outln!("{}: [update]: {line}", ts());
                self.quiet = Some(line);
            }
        }
    }
}
