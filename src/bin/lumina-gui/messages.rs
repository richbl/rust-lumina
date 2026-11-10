//! Definitions of the messages sent from the view layer to the component and the messages sent
//! from background tasks to the component

use lumina::engine::Step;
use lumina::location::{Location, Source};

/// Messages produced by user input and other events
#[derive(Debug)]
pub(crate) enum Msg {
    /// The *Detect location automatically* switch was toggled
    AutoLocation(bool),
    /// The latitude spin-row value changed
    Latitude(f64),
    /// The longitude spin-row value changed
    Longitude(f64),
    /// The minimum brightness spin-row value changed
    Min(f64),
    /// The maximum brightness spin-row value changed
    Max(f64),
    /// The *Follow sunrise and sunset* switch was toggled
    FollowSun(bool),
    /// The *Day starts* entry text changed
    Start(String),
    /// The *Day ends* entry text changed
    Stop(String),
    /// The update interval spin-row value changed
    Interval(f64),
    /// The *Apply* button was clicked; force-apply the target brightness now
    ApplyNow,
    /// The *Respect manual changes* switch was toggled
    RespectManual(bool),
    /// The *Adjust automatically* switch was toggled
    ToggleAuto(bool),
}

/// Messages produced by background tasks and timers
#[derive(Debug)]
pub(crate) enum Cmd {
    /// 1-second wall-clock tick from the background timer loop
    Tick,
    /// Result of an IP-based location detection attempt, with where the location came from
    Detected(Result<(Location, Source), String>),
    /// Result of a [`do_work`](crate::work::do_work) task, tagged with the applier generation it
    /// was started under
    Worked(u64, Result<Step, String>),
}
