//! Connectome simulation core. Pure computation: no Wayland, no Hyprland, no
//! rendering. Everything here must build and test on any platform.

pub mod connectome;
pub mod lif;
pub mod probe;

pub use connectome::{Connectome, Neuron, Role};
pub use lif::{LifParams, Sim};
pub use probe::RateProbe;
