//! Lumina (GUI mode) created using Relm4 to create a modern interface using GTK4/LibAdwaita

mod component;
mod display;
mod messages;
mod model;
mod work;

use clap::Parser;
use lumina::APP_ID;
use lumina::config;
use lumina::form::Form;
use model::{App, InitData};
use relm4::RelmApp;
use std::path::PathBuf;

/// Command-line options for the GUI
#[derive(Parser)]
#[command(
about = lumina::ABOUT,
before_help = lumina::gui_app_title(),)]
struct Args {
    /// Config file (default: `$XDG_CONFIG_HOME/lumina/config.toml`)
    #[arg(short = 'c', long, value_name = "FILE")]
    config: Option<PathBuf>,
}

/// Entry point for the Lumina GUI application
fn main() {
    relm4::gtk::gio::resources_register_include!("lumina.gresource")
        .expect("unable to register application resources");

    let args = Args::parse();

    let default_form = || {
        let s = config::resolve(config::FileConfig::default()).expect("defaults are valid");
        Form::from_settings(&s)
    };

    let (form, load_failed) = match config::load(args.config.as_deref()) {
        Ok(s) => (Form::from_settings(&s), false),
        Err(e) => {
            eprintln!("lumina: config not loaded, using defaults: {e:#}");
            (default_form(), true)
        }
    };

    let app = RelmApp::new(APP_ID);
    app.run::<App>(InitData {
        form,
        config_path: args.config.or_else(config::default_path),
        load_failed,
    });
}
