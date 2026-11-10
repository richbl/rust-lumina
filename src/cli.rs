//! Command-line interface (CLI) definition for the application

use clap::{Args, Parser, Subcommand};
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(
    about = lumina::ABOUT,
    disable_version_flag = true,
    override_usage = "lumina <COMMAND> [OPTIONS]",
    arg_required_else_help = true,
    after_help = "When running a command (except get/set/install/uninstall), the following flag is available:\n  \
      -c, --config <FILE>  Config file (default: $XDG_CONFIG_HOME/lumina/config.toml)"
)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) cmd: Cmd,
}

/// Options shared by every command that reads the config file
#[derive(Args)]
pub(crate) struct ConfigArg {
    /// Config file (default: `$XDG_CONFIG_HOME/lumina/config.toml`)
    #[arg(short = 'c', long, value_name = "FILE")]
    config: Option<PathBuf>,
}

/// Helper to get the config file path from a `ConfigArg`
impl ConfigArg {
    pub(crate) fn path(&self) -> Option<&Path> {
        self.config.as_deref()
    }
}

/// Commands supported by the `lumina` CLI
#[derive(Subcommand)]
pub(crate) enum Cmd {
    /// Show current brightness of each device display
    Get,
    /// Set display brightness (0-100%) immediately
    Set {
        /// Brightness percent
        #[arg(value_parser = clap::value_parser!(u8).range(0..=100))]
        value: u8,
    },
    /// Show the detected/configured device location
    Location(ConfigArg),
    /// Show today's sunrise and sunset times
    Suntime(ConfigArg),
    /// Show the day start and end times currently in use
    Daytime(ConfigArg),
    /// Show the target display brightness
    Status(ConfigArg),
    /// Apply the target brightness once and exit
    Apply(ConfigArg),
    /// Apply the target brightness until interrupted
    Run(ConfigArg),
    /// Install lumina for the current user (under ~/.local)
    #[command(short_flag = 'i', long_flag = "install")]
    Install,
    /// Remove the current user's lumina installation
    #[command(short_flag = 'u', long_flag = "uninstall")]
    Uninstall,
}

/// Parse the process arguments, exiting with a usage message on error
pub(crate) fn parse() -> Cli {
    use clap::{CommandFactory, FromArgMatches};

    let matches = Cli::command()
        .before_help(lumina::app_title())
        .get_matches();

    Cli::from_arg_matches(&matches).unwrap_or_else(|err| err.exit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    fn parse<const N: usize>(args: [&str; N]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(args)
    }

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn override_flags_are_rejected() {
        assert!(parse(["this_app", "status", "--interval", "10"]).is_err());
        assert!(parse(["this_app", "run", "--lat", "47"]).is_err());
    }

    #[test]
    fn set_rejects_out_of_range() {
        assert!(parse(["this_app", "set", "101"]).is_err());
        assert!(parse(["this_app", "set", "100"]).is_ok());
    }

    #[test]
    fn config_flag_follows_command() {
        for cmd in ["location", "suntime", "daytime", "status", "apply", "run"] {
            assert!(parse(["this_app", cmd, "-c", "x.toml"]).is_ok(), "{cmd} -c");
            assert!(
                parse(["this_app", cmd, "--config", "x.toml"]).is_ok(),
                "{cmd} --config"
            );
        }
    }

    #[test]
    fn config_flag_before_command_is_rejected() {
        for cmd in [
            "get", "location", "suntime", "daytime", "status", "apply", "run",
        ] {
            assert!(
                parse(["this_app", "-c", "x.toml", cmd]).is_err(),
                "-c {cmd}"
            );
            assert!(
                parse(["this_app", "--config", "x.toml", cmd]).is_err(),
                "--config {cmd}"
            );
        }
        assert!(parse(["this_app", "-c", "x.toml", "set", "50"]).is_err());
    }

    #[test]
    fn config_flag_rejected_on_commands_that_ignore_it() {
        assert!(parse(["this_app", "get", "-c", "x.toml"]).is_err());
        assert!(parse(["this_app", "set", "50", "-c", "x.toml"]).is_err());
        assert!(parse(["this_app", "set", "-c", "x.toml", "50"]).is_err());
    }

    #[test]
    fn config_value_is_captured() {
        let cli = parse(["this_app", "run", "-c", "x.toml"]).unwrap();
        let is_run = matches!(&cli.cmd, Cmd::Run(_));
        assert!(is_run, "expected `run`");
        if let Cmd::Run(a) = cli.cmd {
            assert_eq!(a.path(), Some(Path::new("x.toml")));
        }
    }

    #[test]
    fn top_level_help_documents_config_option() {
        let cmd = Cli::command();
        let help = cmd.clone().render_help().to_string();
        assert!(help.contains("-c, --config <FILE>"));
        assert!(help.contains("Usage: lumina <COMMAND> [OPTIONS]"));
    }

    #[test]
    fn subcommand_help_has_no_stale_global_options() {
        let mut cmd = Cli::command();
        let apply = cmd.find_subcommand_mut("apply").unwrap();
        let help = apply.render_help().to_string();
        assert!(help.contains("--config"));
        let get = cmd.find_subcommand_mut("get").unwrap();
        assert!(!get.render_help().to_string().contains("--config"));
    }

    #[test]
    fn daytime_command_is_parsed() {
        // bare form
        assert!(parse(["this_app", "daytime"]).is_ok());
        // with explicit config path
        let cli = parse(["this_app", "daytime", "-c", "x.toml"]).unwrap();
        assert!(matches!(&cli.cmd, Cmd::Daytime(_)), "expected `daytime`");
        if let Cmd::Daytime(a) = cli.cmd {
            assert_eq!(a.path(), Some(Path::new("x.toml")));
        }
    }

    #[test]
    fn install_flags_are_parsed() {
        for flag in ["-i", "--install"] {
            let cli = parse(["this_app", flag]).unwrap();
            assert!(matches!(cli.cmd, Cmd::Install), "{flag}");
        }
        for flag in ["-u", "--uninstall"] {
            let cli = parse(["this_app", flag]).unwrap();
            assert!(matches!(cli.cmd, Cmd::Uninstall), "{flag}");
        }
    }

    #[test]
    fn install_flags_are_exclusive_and_take_no_options() {
        assert!(parse(["this_app", "-i", "-u"]).is_err());
        assert!(parse(["this_app", "-i", "status"]).is_err());
        assert!(parse(["this_app", "-i", "-c", "x.toml"]).is_err());
        assert!(parse(["this_app", "-u", "-c", "x.toml"]).is_err());
    }
}
