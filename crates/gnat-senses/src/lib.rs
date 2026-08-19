//! Everything the fly can perceive about the desktop it lives on.
//!
//! Each module is one sense, and each is independently testable: the fly's
//! world is a pure function of these readings.

pub mod activity;
pub mod circadian;
pub mod hypr;
pub mod terrain;
pub mod thermal;

pub use activity::Activity;
pub use hypr::{Client, CursorPos, Event, EventStream, Hypr, LayerLevel, LayerSurface, Monitor};
pub use terrain::{CursorTracker, Ledge, ledges, loom_drive};
pub use thermal::Thermal;
