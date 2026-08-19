//! The fly's body: behaviour states, gait, flight, ledges, sleep.
//!
//! FlyWire has no body data, so unlike [`gnat_sim`] this is modelled rather
//! than measured. The connectome decides *what* the fly does; this decides what
//! that looks like.

pub mod behaviortest;
pub mod circadian;
pub mod fly;
pub mod signals;

pub use fly::{EDGE_MARGIN, FLY_SCALE, Fly, Ledge, Leg, State, Wing};
pub use signals::{SignalBuilder, Signals};
