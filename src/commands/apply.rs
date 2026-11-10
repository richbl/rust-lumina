//! One-shot commands that change display brightness: `set` and `apply`

use super::load_with_location;
use crate::cli::ConfigArg;
use crate::output::report_set;
use anyhow::Result;
use chrono::Local;
use lumina::backlight;
use lumina::plan;

/// Handle the `set` command by setting the brightness of each device display to the specified
/// value and printing the results
pub(super) async fn handle_set(value: u8) -> Result<()> {
    report_set(value, &backlight::set_all(u32::from(value)).await?)
}

/// Handle the `apply` command by applying the target brightness once and printing the results
pub(super) async fn handle_apply(args: &ConfigArg) -> Result<()> {
    let settings = load_with_location(args.path())?;
    let target = plan::brightness_at(&settings, &Local::now())?;

    report_set(target, &backlight::set_all(u32::from(target)).await?)
}
