//! Command handlers and dispatch
//!
//! Read-only commands live in `query`, brightness-changing one-shot commands in `apply`. The
//! long-running `run` command is implemented in `crate::daemon`

mod apply;
mod query;

use crate::cli::{Cmd, ConfigArg};
use crate::{daemon, installer};
use anyhow::Result;
use lumina::config::{self, Settings};
use lumina::location::{self, Location, Source};
use std::fmt;

/// Load settings and, if the schedule needs a location that the config doesn't provide, fall
/// back to an IP lookup
fn load_with_location(path: Option<&std::path::Path>) -> Result<Settings> {
    let mut s = config::load(path)?;

    if s.location.is_none() && s.needs_location() {
        s.location = Some(resolve_location(&s)?.0);
    }

    Ok(s)
}

/// Where the location in use came from, for display
enum Origin {
    /// The config file
    Config,
    /// A lookup (live, or the cached fallback)
    Lookup(Source),
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config => f.write_str("config"),
            Self::Lookup(src) => f.write_str(src.label()),
        }
    }
}

/// Configured location, else an IP lookup (cached fallback); also returns where it came from
fn resolve_location(s: &Settings) -> Result<(Location, Origin)> {
    match s.location {
        Some(l) => Ok((l, Origin::Config)),
        None => location::resolve().map(|(l, src)| (l, Origin::Lookup(src))),
    }
}

/// Handle the `run` command by applying the target brightness at the configured interval until
/// interrupted
async fn handle_run(args: &ConfigArg) -> Result<()> {
    daemon::run(args.path(), config::load(args.path())?).await
}

/// Dispatch the command to the appropriate handler
pub(crate) async fn dispatch(cmd: Cmd) -> Result<()> {
    match cmd {
        Cmd::Get => query::handle_get().await,
        Cmd::Set { value } => apply::handle_set(value).await,
        Cmd::Location(args) => query::handle_location(&args),
        Cmd::Suntime(args) => query::handle_suntime(&args),
        Cmd::Daytime(args) => query::handle_daytime(&args),
        Cmd::Status(args) => query::handle_status(&args),
        Cmd::Apply(args) => apply::handle_apply(&args).await,
        Cmd::Run(args) => handle_run(&args).await,
        Cmd::Install => installer::install(),
        Cmd::Uninstall => installer::uninstall(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_labels() {
        assert_eq!(Origin::Config.to_string(), "config");
        assert_eq!(
            Origin::Lookup(Source::Cache).to_string(),
            Source::Cache.label()
        );
    }
}
