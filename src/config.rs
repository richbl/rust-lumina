//! Config file (TOML) loading and validation
//! Functions to load, validate, resolve and save the config file

use crate::location::Location;
use anyhow::{Context, Result, bail};
use chrono::NaiveTime;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const CONFIG_HEADER: &str = concat!(
    "# Lumina Configuration (TOML)\n",
    "# v",
    env!("CARGO_PKG_VERSION"),
    "\n\n"
);

/// Default minimum brightness (percent, 0-100)
pub const DEFAULT_MIN: u8 = 20;

/// Default maximum brightness (percent, 0-100)
pub const DEFAULT_MAX: u8 = 90;

/// Default interval between brightness updates (seconds)
pub const DEFAULT_INTERVAL_SECONDS: u64 = 60;

/// Shortest allowed interval between brightness updates (seconds)
pub const INTERVAL_MIN_SECONDS: u64 = 1;

/// Longest allowed interval between brightness updates (seconds)
pub const INTERVAL_MAX_SECONDS: u64 = 3600;

/// `[location]` section (latitude/longitude in decimal degrees)
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocationSection {
    /// Latitude in decimal degrees, e.g. 47.9
    pub latitude: Option<f64>,
    /// Longitude in decimal degrees, e.g. -122.2
    pub longitude: Option<f64>,
}

/// `[brightness]` section. Values are percent (0-100)
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrightnessSection {
    /// Minimum brightness (percent, 0-100)
    pub min: Option<u8>,
    /// Maximum brightness (percent, 0-100)
    pub max: Option<u8>,
}

/// `[schedule]` section. Times are "HH:MM" (24h, local)
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduleSection {
    /// Overrides sunrise
    pub start_time: Option<String>,
    /// Overrides sunset
    pub stop_time: Option<String>,
    /// Interval between brightness updates (seconds)
    pub interval_seconds: Option<u64>,
    /// Leave brightness alone after a manual change instead of re-asserting the schedule
    pub respect_manual_changes: Option<bool>,
}

/// The raw on-disk representation of the config file
/// ALL fields are optional
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileConfig {
    /// Location section (latitude/longitude)
    pub location: Option<LocationSection>,
    /// Brightness section (min/max)
    pub brightness: Option<BrightnessSection>,
    /// Schedule section
    pub schedule: Option<ScheduleSection>,
}

/// The resolved settings, with defaults applied and validated
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// Location (latitude/longitude) if provided, or `None` if not
    pub location: Option<Location>,
    /// Minimum brightness (percent, 0-100)
    pub min: u8,
    /// Maximum brightness (percent, 0-100)
    pub max: u8,
    /// Start time (local) if provided, or `None` if not
    pub start: Option<NaiveTime>,
    /// Stop time (local) if provided, or `None` if not
    pub stop: Option<NaiveTime>,
    /// Interval between brightness updates (seconds)
    pub interval_seconds: u64,
    /// Pause scheduled writes once brightness is changed outside lumina
    pub respect_manual_changes: bool,
}

/// Methods on the resolved settings
impl Settings {
    /// Returns true if the settings require a location to be provided (i.e. if either start or
    /// stop time is missing)
    #[must_use]
    pub fn needs_location(&self) -> bool {
        self.start.is_none() || self.stop.is_none()
    }
}

/// Parse a time string in "HH:MM" format (24h, local)
///
/// # Errors
/// Returns an error if the string is not in the expected format
pub fn parse_time(s: &str) -> Result<NaiveTime> {
    NaiveTime::parse_from_str(s.trim(), "%H:%M")
        .with_context(|| format!("invalid time {s:?}, expected HH:MM (24h)"))
}

/// Returns the default config file path, or `None` if it cannot be determined
pub fn default_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;

    Some(base.join("lumina").join("config.toml"))
}

/// Parse the config file from a string
///
/// # Errors
/// Returns an error if the string is not valid TOML or does not match the expected structure
pub fn parse_file(text: &str) -> Result<FileConfig> {
    toml::from_str(text).context("invalid config file")
}

/// Read the config file from disk, returning the parsed config and the path it was read from
/// (if any)
fn read_file(path: Option<&Path>) -> Result<(FileConfig, Option<PathBuf>)> {
    let (path, explicit) = match path {
        Some(p) => (p.to_path_buf(), true),
        None => match default_path() {
            Some(p) => (p, false),
            None => return Ok((FileConfig::default(), None)),
        },
    };

    match std::fs::read_to_string(&path) {
        Ok(text) => {
            let cfg = parse_file(&text).with_context(|| format!("in {}", path.display()))?;
            Ok((cfg, Some(path)))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && !explicit => {
            Ok((FileConfig::default(), None))
        }
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

/// Load the raw on-disk representation (used by `save` round-trips and tests)
///
/// # Errors
/// Returns an error if the file cannot be read or parsed
pub fn load_file(path: Option<&Path>) -> Result<FileConfig> {
    read_file(path).map(|(cfg, _)| cfg)
}

/// Modification time of the config file that `load` would read, if it exists
#[must_use]
pub fn modified(path: Option<&Path>) -> Option<SystemTime> {
    let path = path.map(Path::to_path_buf).or_else(default_path)?;
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Load, validate, and resolve settings (defaults filled in)
///
/// # Errors
/// Returns an error if the file cannot be read, parsed, or validated
pub fn load(path: Option<&Path>) -> Result<Settings> {
    let (file, src) = read_file(path)?;
    resolve(file).with_context(|| match &src {
        Some(p) => format!("invalid settings in {}", p.display()),
        None => "invalid default settings".to_string(),
    })
}

/// Write the config as TOML, creating parent directories. `None` uses the default path
///
/// # Errors
/// Returns an error if the file cannot be written
pub fn save(path: Option<&Path>, cfg: &FileConfig) -> Result<PathBuf> {
    let path = match path {
        Some(p) => p.to_path_buf(),
        None => default_path().context("cannot determine config directory")?,
    };
    // Resolve symlinks so we update the target, not replace the link
    let path = std::fs::canonicalize(&path).unwrap_or(path);

    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }

    let text = format!(
        "{CONFIG_HEADER}{}",
        toml::to_string_pretty(cfg).context("serializing config")?
    );

    // Write a sibling temp file, then rename over the original
    let mut name = path
        .file_name()
        .context("config path has no file name")?
        .to_os_string();
    name.push(".tmp");
    let tmp = path.with_file_name(name);

    let write = || -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
        std::fs::rename(&tmp, &path)
    };

    if let Err(e) = write() {
        let _ = std::fs::remove_file(&tmp);
        return Err(e).with_context(|| format!("writing {}", path.display()));
    }

    Ok(path)
}

/// Copy the config file to `<name>.bak` if it exists but fails to load, so that a later `save`
/// doesn't destroy it. Returns the backup path when one was made
///
/// # Errors
/// Returns an error if the config path cannot be determined or the copy fails
pub fn backup_if_invalid(path: Option<&Path>) -> Result<Option<PathBuf>> {
    let path = match path {
        Some(p) => p.to_path_buf(),
        None => default_path().context("cannot determine config directory")?,
    };

    let path = std::fs::canonicalize(&path).unwrap_or(path);
    if !path.is_file() || load(Some(&path)).is_ok() {
        return Ok(None);
    }

    let mut name = path
        .file_name()
        .context("config path has no file name")?
        .to_os_string();

    name.push(".bak");
    let bak = path.with_file_name(name);
    std::fs::copy(&path, &bak).with_context(|| format!("backing up to {}", bak.display()))?;

    Ok(Some(bak))
}

/// Validate `[location]`: latitude and longitude come together or not at all
fn resolve_location(loc: &LocationSection) -> Result<Option<Location>> {
    match (loc.latitude, loc.longitude) {
        (Some(la), Some(lo)) => Ok(Some(Location::new(la, lo).context("[location]")?)),
        (None, None) => Ok(None),
        _ => bail!("[location]: latitude and longitude must be provided together"),
    }
}

/// Validate `[brightness]` and apply defaults, returning `(min, max)`
fn resolve_brightness(bri: &BrightnessSection) -> Result<(u8, u8)> {
    let min = bri.min.unwrap_or(DEFAULT_MIN);
    let max = bri.max.unwrap_or(DEFAULT_MAX);
    if min > 100 || max > 100 {
        bail!("[brightness]: values must be 0-100 (got min={min}, max={max})");
    }
    if min > max {
        bail!("[brightness]: min ({min}) must not exceed max ({max})");
    }

    Ok((min, max))
}

/// Parse an optional "HH:MM" value belonging to the named `[schedule]` key
fn parse_key(value: Option<&str>, key: &str) -> Result<Option<NaiveTime>> {
    value
        .map(|s| parse_time(s).with_context(|| format!("[schedule] {key}")))
        .transpose()
}

/// Resolve the raw on-disk representation into validated settings, filling in defaults
///
/// # Errors
/// Returns an error if the settings are invalid (e.g. min > max, missing lat/lon, invalid time
/// format, identical start and stop times, or an interval outside the allowed range)
pub fn resolve(file: FileConfig) -> Result<Settings> {
    let location = resolve_location(&file.location.unwrap_or_default())?;
    let (min, max) = resolve_brightness(&file.brightness.unwrap_or_default())?;
    let sched = file.schedule.unwrap_or_default();

    let start = parse_key(sched.start_time.as_deref(), "start_time")?;
    let stop = parse_key(sched.stop_time.as_deref(), "stop_time")?;
    if start.is_some() && start == stop {
        bail!("[schedule]: start_time and stop_time must differ");
    }

    let interval_seconds = sched.interval_seconds.unwrap_or(DEFAULT_INTERVAL_SECONDS);
    if !(INTERVAL_MIN_SECONDS..=INTERVAL_MAX_SECONDS).contains(&interval_seconds) {
        bail!(
            "[schedule]: interval_seconds must be {INTERVAL_MIN_SECONDS}-{INTERVAL_MAX_SECONDS} (got {interval_seconds})"
        );
    }

    Ok(Settings {
        location,
        min,
        max,
        start,
        stop,
        interval_seconds,
        respect_manual_changes: sched.respect_manual_changes.unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(toml: &str) -> Result<Settings> {
        resolve(parse_file(toml).unwrap())
    }

    const SAMPLE: &str = "[location]\nlatitude = 47.9\nlongitude = -122.2\n\n[brightness]\nmin = 10\nmax = 80\n\n[schedule]\nstart_time = \"06:30\"\ninterval_seconds = 30\n";

    #[test]
    fn defaults() {
        let s = resolve(FileConfig::default()).unwrap();
        assert_eq!((s.min, s.max), (DEFAULT_MIN, DEFAULT_MAX));
        assert_eq!(s.interval_seconds, DEFAULT_INTERVAL_SECONDS);
        assert!(s.location.is_none() && s.start.is_none() && s.stop.is_none());
        assert!(s.needs_location());
    }

    #[test]
    fn file_values_are_resolved() {
        let s = settings(SAMPLE).unwrap();
        assert_eq!((s.min, s.max, s.interval_seconds), (10, 80, 30));
        assert_eq!(s.start, Some(NaiveTime::from_hms_opt(6, 30, 0).unwrap()));
        assert_eq!(s.stop, None);
        assert_eq!(
            s.location,
            Some(Location {
                latitude: 47.9,
                longitude: -122.2
            })
        );
        assert!(s.needs_location()); // stop still has to come from sunset
    }

    #[test]
    fn fixed_start_and_stop_need_no_location() {
        let s = settings("[schedule]\nstart_time = \"06:30\"\nstop_time = \"21:15\"\n").unwrap();
        assert!(!s.needs_location());
    }

    #[test]
    fn partial_sections_are_fine() {
        let s = settings("[brightness]\nmax = 50\n").unwrap();
        assert_eq!((s.min, s.max), (DEFAULT_MIN, 50));
        assert!(settings("").is_ok());
    }

    #[test]
    fn rejects_bad_values() {
        let bad = |t: &str| settings(t).is_err();
        assert!(bad("[location]\nlatitude = 10.0\n")); // lon missing
        assert!(bad("[location]\nlatitude = 95.0\nlongitude = 0.0\n"));
        assert!(bad("[brightness]\nmin = 80\nmax = 30\n"));
        assert!(bad("[brightness]\nmax = 101\n"));
        assert!(bad("[schedule]\nstart_time = \"25:00\"\n"));
        assert!(bad("[schedule]\ninterval_seconds = 0\n"));
        assert!(bad("[schedule]\ninterval_seconds = 3601\n"));
    }

    #[test]
    fn unknown_keys_are_rejected() {
        assert!(parse_file("[location]\nlatitudx = 5\n").is_err());
        assert!(parse_file("[brightnesx]\nmin = 5\n").is_err());
        // Flat top-level keys are not part of the format.
        assert!(parse_file("latitude = 47.9\n").is_err());
    }

    #[test]
    fn load_explicit_missing_is_error_but_file_loads() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("c.toml");
        assert!(load(Some(&p)).is_err());
        std::fs::write(&p, "[brightness]\nmin = 15\n").unwrap();
        assert_eq!(load(Some(&p)).unwrap().min, 15);
        assert_eq!(
            load_file(Some(&p)).unwrap().brightness.unwrap().min,
            Some(15)
        );
    }

    #[test]
    fn interval_bounds_are_inclusive() {
        let ok = |n: u64| settings(&format!("[schedule]\ninterval_seconds = {n}\n")).is_ok();
        assert!(ok(INTERVAL_MIN_SECONDS) && ok(INTERVAL_MAX_SECONDS));
        assert!(!ok(INTERVAL_MIN_SECONDS - 1) && !ok(INTERVAL_MAX_SECONDS + 1));
    }

    #[test]
    fn modified_tracks_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("c.toml");
        assert!(modified(Some(&p)).is_none());
        std::fs::write(&p, "").unwrap();
        assert!(modified(Some(&p)).is_some());
    }

    #[test]
    fn invalid_settings_error_names_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("bad.toml");
        std::fs::write(&p, "[brightness]\nmin = 90\nmax = 10\n").unwrap();
        let msg = format!("{:#}", load(Some(&p)).unwrap_err());
        assert!(msg.contains("bad.toml") && msg.contains("min"));
    }

    #[test]
    fn save_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("c.toml");
        save(Some(&p), &FileConfig::default()).unwrap();
        assert!(p.exists() && !dir.path().join("c.toml.tmp").exists());
        assert_eq!(
            std::fs::read_to_string(p).unwrap(),
            "# Lumina Configuration (TOML)\n# v0.1.0\n\n"
        );
    }

    #[test]
    fn invalid_config_is_backed_up_valid_is_not() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("c.toml");
        assert_eq!(backup_if_invalid(Some(&p)).unwrap(), None); // missing

        std::fs::write(&p, "[brightness]\nmin = 15\n").unwrap();
        assert_eq!(backup_if_invalid(Some(&p)).unwrap(), None); // valid

        std::fs::write(&p, "[brightness]\nmin = 90\nmax = 10\n").unwrap();
        let bak = backup_if_invalid(Some(&p)).unwrap().unwrap();
        assert!(bak.ends_with("c.toml.bak"));
        assert_eq!(
            std::fs::read_to_string(bak).unwrap(),
            "[brightness]\nmin = 90\nmax = 10\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn save_through_symlink_updates_target() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real.toml");
        let link = dir.path().join("link.toml");
        std::fs::write(&real, "").unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();

        save(Some(&link), &parse_file(SAMPLE).unwrap()).unwrap();

        assert!(std::fs::symlink_metadata(&link).unwrap().is_symlink());
        assert_eq!(load_file(Some(&real)).unwrap(), parse_file(SAMPLE).unwrap());
    }

    #[test]
    fn save_and_load_roundtrip_omits_unset_values() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("nested").join("c.toml");
        let cfg = parse_file(SAMPLE).unwrap();
        save(Some(&p), &cfg).unwrap();
        assert_eq!(load_file(Some(&p)).unwrap(), cfg);

        let sparse = FileConfig {
            brightness: Some(BrightnessSection {
                min: Some(5),
                max: None,
            }),
            ..Default::default()
        };
        save(Some(&p), &sparse).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(
            text.contains("[brightness]") && !text.contains("location") && !text.contains("max")
        );
        assert_eq!(load_file(Some(&p)).unwrap(), sparse);
    }

    #[test]
    fn time_parsing() {
        assert!(parse_time("7:05").is_ok());
        assert!(parse_time(" 19:45 ").is_ok());
        assert!(parse_time("noon").is_err());
    }

    #[test]
    fn identical_start_and_stop_are_rejected() {
        let t = |a: &str, b: &str| {
            settings(&format!(
                "[schedule]\nstart_time = \"{a}\"\nstop_time = \"{b}\"\n"
            ))
        };
        assert!(t("06:00", "06:00").is_err());
        assert!(t("06:00", "06:01").is_ok());
    }

    #[test]
    fn respect_manual_changes_defaults_off() {
        assert!(!settings("").unwrap().respect_manual_changes);
        let on = settings("[schedule]\nrespect_manual_changes = true\n").unwrap();
        assert!(on.respect_manual_changes);
    }
}
