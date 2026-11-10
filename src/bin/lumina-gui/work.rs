//! Adapts the shared [`lumina::engine`] to the GUI's background-task model

use lumina::engine::{Applier, Step};

/// Brightness-adjustment modes
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Mode {
    /// Read the current brightness... just look, don't touch!
    Read,
    /// Apply the scheduled value now (button)
    Force,
    /// Apply only if the schedule moved or something hideous changed brightness
    Auto,
}

/// Perform a background brightness read/write operation. The caller passes a *clone* of its
/// [`Applier`] and folds the returned [`Step`] back in with [`Applier::record`]
pub(crate) async fn do_work(
    mode: Mode,
    target: Option<u8>,
    applier: Applier,
) -> Result<Step, String> {
    let result = match (mode, target) {
        (Mode::Read, _) | (_, None) => Step::observe().await,
        (Mode::Force, Some(t)) => applier.step(t, true).await,
        (Mode::Auto, Some(t)) => applier.step(t, false).await,
    };

    result.map_err(|e| format!("{e:#}"))
}
