//! This library supports the `lumina` CLI and the optional `lumina-gui` front end

/// One-line description of the tool, shared by the CLI and GUI help text
pub const ABOUT: &str = "Display Brightness Based on Location and Time of Day";

/// application name for the CLI
pub const APP_NAME: &str = "Lumina";

/// application name for the GUI
pub const GUI_APP_NAME: &str = "Lumina-GUI";

/// application version based on Cargo literal
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Application identification used when creating/registering the GUI
pub const APP_ID: &str = "io.github.richbl.Lumina";

/// Device I/O: read and set the brightness of each display
pub mod backlight;
/// Load, validate, resolve and save the TOML config
pub mod config;
/// The apply workflow: decides when and what to write
pub mod engine;
/// Editable GUI form state, free of GTK dependencies
pub mod form;
/// IP geolocation with an on-disk cache
pub mod location;
/// Combines settings and solar data into a window and a target brightness
pub mod plan;
/// The pure sine brightness curve
pub mod schedule;
/// Sunrise and sunset calculation
pub mod solar;

/// return the CLI application title
#[must_use]
pub fn app_title() -> String {
    format!("{APP_NAME} {APP_VERSION}")
}

/// return the GUI application title
#[must_use]
pub fn gui_app_title() -> String {
    format!("{GUI_APP_NAME} {APP_VERSION}")
}
