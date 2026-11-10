//! Routines to read and set the brightness of all controllable display devices on Linux. It uses
//! the `brightness` crate to enumerate devices and set brightness, and reads from
//! `/sys/class/backlight/<device>/{brightness,max_brightness}` to get the current brightness
//! percentage of the device display

use anyhow::{Context, Result};
use brightness::Brightness;
use futures_util::TryStreamExt;
use std::fmt;
use std::fs;

/// A controllable display device handle
pub type Device = brightness::BrightnessDevice;

/// A single display's name and brightness percentage
///
/// Ordered by name so that collections of readings can be sorted and compared without depending
/// on the order in which the OS enumerates devices
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Reading {
    /// Device name, e.g. `intel_backlight`
    pub name: String,
    /// Brightness as a percentage (0-100)
    pub percent: u32,
}

impl Reading {
    /// Create a new reading
    #[must_use]
    pub fn new(name: impl Into<String>, percent: u32) -> Self {
        Self {
            name: name.into(),
            percent,
        }
    }
}

impl fmt::Display for Reading {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}%", self.name, self.percent)
    }
}

/// A device that could not be read or written, with the reason
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Failure {
    /// Device name, or `device N` if the name itself could not be read
    pub name: String,
    /// The rendered error chain
    pub error: String,
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.name, self.error)
    }
}

/// Format failures as `name: error; name: error`
#[must_use]
pub fn fmt_failures(v: &[Failure]) -> String {
    v.iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

/// Per-device results of a read or write, both sorted by device name
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outcome {
    /// Devices that succeeded
    pub readings: Vec<Reading>,
    /// Devices that failed
    pub failures: Vec<Failure>,
}

impl Outcome {
    /// True when no devices were found at all
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.readings.is_empty() && self.failures.is_empty()
    }

    /// Fold one device's result in
    fn record(&mut self, name: String, result: Result<u32>) {
        match result {
            Ok(percent) => self.readings.push(Reading { name, percent }),
            Err(e) => self.failures.push(Failure {
                name,
                error: format!("{e:#}"),
            }),
        }
    }

    fn sorted(mut self) -> Self {
        self.readings.sort();
        self.failures.sort();
        self
    }

    /// Pass partial success through, but treat total failure as an error
    ///
    /// # Errors
    /// Returns an error summarizing the failures if devices were found and none succeeded
    pub fn require_any(self) -> Result<Self> {
        if self.readings.is_empty() && !self.failures.is_empty() {
            anyhow::bail!("{}", fmt_failures(&self.failures));
        }

        Ok(self)
    }
}

/// Converts a raw brightness value to a percentage of the maximum brightness, rounding to the
/// nearest integer. Returns 0 if `max` is 0 to avoid division by zero
#[must_use]
pub fn percent_from_raw(cur: u32, max: u32) -> u32 {
    if max == 0 {
        return 0;
    }

    let percent = (u64::from(cur) * 100 + u64::from(max) / 2) / u64::from(max);
    u32::try_from(percent).unwrap_or(0)
}

/// Gets the current brightness percentage of a device by reading from
/// `/sys/class/backlight/<device>/{brightness,max_brightness}`
///
/// # Errors
/// Returns an error if the device does not exist, or if the brightness values cannot be read
pub fn get_brightness_percent(device: &str) -> Result<u32> {
    let base = format!("/sys/class/backlight/{device}");

    let read = |name: &str| -> Result<u32> {
        fs::read_to_string(format!("{base}/{name}"))
            .with_context(|| format!("Failed to read {name}"))?
            .trim()
            .parse()
            .with_context(|| format!("Failed to parse {name}"))
    };

    Ok(percent_from_raw(
        read("brightness")?,
        read("max_brightness")?,
    ))
}

/// Enumerates the controllable devices
///
/// # Errors
/// Returns an error if the devices cannot be enumerated
pub async fn devices() -> Result<Vec<Device>> {
    brightness::brightness_devices()
        .try_collect()
        .await
        .context("Failed to enumerate brightness devices")
}

/// A device's name, falling back to its position in the enumeration if the name is unavailable
async fn name_of(dev: &Device, index: usize) -> String {
    dev.device_name()
        .await
        .unwrap_or_else(|_| format!("device {index}"))
}

/// Reads every device in turn; devices that cannot be read are reported as failures
pub async fn read(devices: &[Device]) -> Outcome {
    let mut out = Outcome::default();

    for (i, dev) in devices.iter().enumerate() {
        let name = name_of(dev, i).await;
        let result = get_brightness_percent(&name);
        out.record(name, result);
    }

    out.sorted()
}

/// Sets one device and reads it back
async fn set_one(dev: &mut Device, name: &str, level: u32) -> Result<u32> {
    dev.set(level).await.context("Failed to set brightness")?;
    get_brightness_percent(name)
}

/// Sets every device to `value` (clamped to 0-100) and verifies each; devices that fail are
/// reported as failures without affecting the others
///
/// # Errors
/// Returns an error if `devices` is empty
pub async fn write(devices: &mut [Device], value: u32) -> Result<Outcome> {
    if devices.is_empty() {
        anyhow::bail!("No brightness-controllable devices found.");
    }

    let level = value.clamp(0, 100);
    let mut out = Outcome::default();

    for (i, dev) in devices.iter_mut().enumerate() {
        let name = name_of(dev, i).await;
        let result = set_one(dev, &name, level).await;
        out.record(name, result);
    }

    Ok(out.sorted())
}

/// Gets the current brightness of all controllable devices
///
/// # Errors
/// Returns an error if the devices cannot be enumerated
pub async fn get_all() -> Result<Outcome> {
    Ok(read(&devices().await?).await)
}

/// Sets the brightness of all controllable devices to the given percentage value (0-100) and
/// returns the verified readings
///
/// # Errors
/// Returns an error if the devices cannot be enumerated, or if there are none
pub async fn set_all(value: u32) -> Result<Outcome> {
    let mut devs = devices().await?;
    write(&mut devs, value).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounds_to_nearest() {
        assert_eq!(percent_from_raw(0, 255), 0);
        assert_eq!(percent_from_raw(255, 255), 100);
        assert_eq!(percent_from_raw(128, 255), 50);
        assert_eq!(percent_from_raw(1, 400), 0);
        assert_eq!(percent_from_raw(2, 400), 1);
    }

    #[test]
    fn zero_max_does_not_divide_by_zero() {
        assert_eq!(percent_from_raw(5, 0), 0);
    }

    #[test]
    fn readings_sort_by_name_and_display() {
        let mut v = [Reading::new("b", 1), Reading::new("a", 2)];
        v.sort();
        assert_eq!(v[0].name, "a");
        assert_eq!(v[0].to_string(), "a 2%");
    }

    #[test]
    fn outcome_splits_and_sorts() {
        let mut o = Outcome::default();
        o.record("c".into(), Ok(3));
        o.record("a".into(), Err(anyhow::anyhow!("boom")));
        o.record("b".into(), Ok(1));
        let o = o.sorted();

        assert_eq!(o.readings, [Reading::new("b", 1), Reading::new("c", 3)]);
        assert_eq!(o.failures[0].to_string(), "a: boom");
        assert!(o.require_any().is_ok());
    }

    #[test]
    fn total_failure_is_an_error() {
        let mut o = Outcome::default();
        o.record("a".into(), Err(anyhow::anyhow!("boom")));
        o.record("b".into(), Err(anyhow::anyhow!("bang")));

        let e = o.sorted().require_any().unwrap_err();
        assert_eq!(e.to_string(), "a: boom; b: bang");
    }

    #[test]
    fn no_devices_is_empty_not_an_error() {
        let o = Outcome::default();
        assert!(o.is_empty());
        assert!(o.require_any().is_ok());
    }
}
