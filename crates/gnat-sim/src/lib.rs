//! Connectome simulation core. Pure computation: no Wayland, no Hyprland, no
//! rendering. Everything here builds and tests on any platform.

pub mod circuit;
pub mod lif;
pub mod points;
pub mod rng;
pub mod simtest;

pub use circuit::{Circuit, Neuron, Role, Side};
pub use lif::{Groups, Inputs, Lif, Rates};
pub use points::{BrainPoints, Point};
pub use rng::Rng;
pub use simtest::Report;
