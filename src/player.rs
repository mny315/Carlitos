//! Playback state and rate controls shared by every platform.
#[cfg(target_os = "linux")]
#[path = "player/gstreamer/mod.rs"]
mod backend;
#[cfg(windows)]
#[path = "windows/player.rs"]
mod backend;
#[cfg(any(target_os = "linux", windows))]
pub use backend::{Event, EventKind, Player};

pub const MIN_RATE: f64 = 0.5;
pub const MAX_RATE: f64 = 3.0;

pub fn stepped_rate(rate: f64, forward: bool) -> f64 {
    ((rate * 100. + if forward { 5. } else { -5. }).round() / 100.).clamp(MIN_RATE, MAX_RATE)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Empty,
    Loading,
    Seeking,
    Ready,
    Error,
}
