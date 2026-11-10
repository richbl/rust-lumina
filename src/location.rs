//! Determine the computer's location via IP geolocation with the help of external website API
//! calls (a primary and a fallback website)
//!
//! 1. https://ipwho.is
//! 2. https://ipapi.co

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// A geographic location represented by latitude and longitude in decimal degrees
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Location {
    /// Latitude in decimal degrees, must be in the range [-90, 90]
    pub latitude: f64,
    /// Longitude in decimal degrees, must be in the range [-180, 180]
    pub longitude: f64,
}

impl Location {
    /// Create a new `Location` instance, validating that the latitude and longitude are within
    /// the acceptable ranges
    ///
    /// # Errors
    /// Returns an error if the latitude is not in the range [-90, 90] or if the longitude is not
    /// in the range [-180, 180]
    pub fn new(latitude: f64, longitude: f64) -> Result<Self> {
        if !(-90.0..=90.0).contains(&latitude) {
            bail!("latitude {latitude} out of range (-90..=90)");
        }

        if !(-180.0..=180.0).contains(&longitude) {
            bail!("longitude {longitude} out of range (-180..=180)");
        }

        Ok(Self {
            latitude,
            longitude,
        })
    }
}

/// Struct for deserializing JSON responses from IP geolocation provider (`https://ipwho.is`)
#[derive(Deserialize)]
struct IpWhoIs {
    success: Option<bool>,
    latitude: Option<f64>,
    longitude: Option<f64>,
}

/// Struct for deserializing JSON responses from IP geolocation provider (`https://ipapi.co`)
#[derive(Deserialize)]
struct IpApiCo {
    latitude: Option<f64>,
    longitude: Option<f64>,
    error: Option<bool>,
}

/// Parse the JSON response from ipwho.is and extract the latitude and longitude, returning a
/// `Location` instance
///
/// # Errors
/// Returns an error if the JSON is invalid, if the `success` field is false, or if the `latitude` or
/// `longitude` fields are missing
pub fn parse_ipwhois(body: &str) -> Result<Location> {
    let r: IpWhoIs = serde_json::from_str(body).context("invalid ipwho.is JSON")?;
    if r.success == Some(false) {
        bail!("ipwho.is reported failure");
    }
    Location::new(
        r.latitude.ok_or_else(|| anyhow!("missing latitude"))?,
        r.longitude.ok_or_else(|| anyhow!("missing longitude"))?,
    )
}

/// Parse the JSON response from ipapi.co and extract the latitude and longitude, returning a
/// `Location` instance
///
/// # Errors
/// Returns an error if the JSON is invalid, if the `error` field is true, or if the `latitude` or
/// `longitude` fields are missing
pub fn parse_ipapi_co(body: &str) -> Result<Location> {
    let r: IpApiCo = serde_json::from_str(body).context("invalid ipapi.co JSON")?;
    if r.error == Some(true) {
        bail!("ipapi.co reported an error");
    }

    Location::new(
        r.latitude.ok_or_else(|| anyhow!("missing latitude"))?,
        r.longitude.ok_or_else(|| anyhow!("missing longitude"))?,
    )
}

/// A type alias for a function that takes a string slice and returns a `Result<Location>`, used to
/// represent the parsing functions for different IP geolocation providers... the `Clippy` linter
/// forced me to do this... honest
type Parser = fn(&str) -> Result<Location>;

/// Determine the computer's location by querying multiple IP geolocation providers in order of
/// preference
///
/// # Errors
/// Returns an error if all providers fail to return a valid location
pub fn from_ip() -> Result<Location> {
    let providers: [(&str, Parser); 2] = [
        ("https://ipwho.is/", parse_ipwhois),
        ("https://ipapi.co/json/", parse_ipapi_co),
    ];

    let agent = ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(5)))
            .build(),
    );

    let mut errors = Vec::new();

    for (url, parse) in providers {
        let attempt = agent
            .get(url)
            .call()
            .map_err(anyhow::Error::from)
            .and_then(|r| r.into_body().read_to_string().map_err(anyhow::Error::from))
            .and_then(|body| parse(&body));
        match attempt {
            Ok(loc) => return Ok(loc),
            Err(e) => errors.push(format!("{url}: {e:#}")),
        }
    }

    bail!(
        "could not determine location from IP:\n  {}",
        errors.join("\n  ")
    )
}

/// Where a resolved location came from
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// A fresh IP geolocation lookup
    Ip,
    /// The last good lookup, persisted locally
    Cache,
}

impl Source {
    /// Description for user-facing output
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Ip => "IP address lookup",
            Self::Cache => "cached IP lookup",
        }
    }
}

/// The on-disk form of the cached location
#[derive(Serialize, Deserialize)]
struct Cached {
    latitude: f64,
    longitude: f64,
}

/// Default cache file path, or `None` if it cannot be determined
#[must_use]
pub fn cache_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;

    Some(base.join("lumina").join("location.json"))
}

/// Read a cached location, treating a missing, unreadable or invalid file as no cache
fn read_cache(path: &Path) -> Option<Location> {
    let text = std::fs::read_to_string(path).ok()?;
    let c: Cached = serde_json::from_str(&text).ok()?;
    Location::new(c.latitude, c.longitude).ok()
}

/// Write the cache atomically (temp file, then rename)
fn write_cache(path: &Path, loc: Location) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let text = serde_json::to_string(&Cached {
        latitude: loc.latitude,
        longitude: loc.longitude,
    })
    .context("serializing location")?;

    let tmp = path.with_extension("json.tmp");

    let result = std::fs::write(&tmp, text).and_then(|()| std::fs::rename(&tmp, path));
    if let Err(e) = result {
        let _ = std::fs::remove_file(&tmp);
        return Err(e).with_context(|| format!("writing {}", path.display()));
    }

    Ok(())
}

/// The last good IP lookup (if one was cached)
#[must_use]
pub fn load_cached() -> Option<Location> {
    read_cache(&cache_path()?)
}

/// Remember a location for later runs
///
/// # Errors
/// Returns an error if the cache directory is unknown or the file cannot be written
pub fn store_cached(loc: Location) -> Result<()> {
    write_cache(
        &cache_path().context("cannot determine cache directory")?,
        loc,
    )
}

/// Look up the location by IP, refreshing the cache on success and falling back to the cache on
/// failure
///
/// # Errors
/// Returns the lookup error if it fails and nothing is cached
pub fn resolve() -> Result<(Location, Source)> {
    match from_ip() {
        Ok(loc) => {
            let _ = store_cached(loc); // best effort
            Ok((loc, Source::Ip))
        }
        Err(e) => load_cached().map(|l| (l, Source::Cache)).ok_or(e),
    }
}

/// [`resolve`] on a blocking thread, so the `ureq` calls don't stall the async runtime
///
/// # Errors
/// Same as [`resolve`], or if the blocking task fails
pub async fn resolve_async() -> Result<(Location, Source)> {
    tokio::task::spawn_blocking(resolve)
        .await
        .context("location lookup task failed")?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ipwhois() {
        let l = parse_ipwhois(
            r#"{"success":true,"latitude":47.9,"longitude":-122.2,"city":"Everett"}"#,
        )
        .unwrap();
        assert_eq!(
            l,
            Location {
                latitude: 47.9,
                longitude: -122.2
            }
        );
    }

    #[test]
    fn ipwhois_failure_and_missing_fields() {
        assert!(parse_ipwhois(r#"{"success":false,"message":"nope"}"#).is_err());
        assert!(parse_ipwhois(r#"{"success":true}"#).is_err());
        assert!(parse_ipwhois("not json").is_err());
    }

    #[test]
    fn parses_ipapi_co() {
        let l = parse_ipapi_co(r#"{"latitude":51.5,"longitude":-0.12}"#).unwrap();
        assert_eq!(
            l,
            Location {
                latitude: 51.5,
                longitude: -0.12
            }
        );
        assert!(parse_ipapi_co(r#"{"error":true,"reason":"RateLimited"}"#).is_err());
    }

    #[test]
    fn rejects_out_of_range() {
        assert!(Location::new(91.0, 0.0).is_err());
        assert!(Location::new(0.0, -181.0).is_err());
        assert!(Location::new(-90.0, 180.0).is_ok());
    }

    #[test]
    fn cache_roundtrip_and_garbage() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("sub").join("location.json");
        assert_eq!(read_cache(&p), None);

        let loc = Location::new(47.9, -122.2).unwrap();
        write_cache(&p, loc).unwrap();
        assert_eq!(read_cache(&p), Some(loc));
        assert!(!dir.path().join("sub").join("location.json.tmp").exists());

        std::fs::write(&p, "nope").unwrap();
        assert_eq!(read_cache(&p), None);
        std::fs::write(&p, r#"{"latitude":99.0,"longitude":0.0}"#).unwrap();
        assert_eq!(read_cache(&p), None);
    }
}
