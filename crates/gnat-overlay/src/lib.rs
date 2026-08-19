//! The windows this program opens, and the software canvas they are painted on.
//!
//! Two kinds, because they want opposite things:
//!
//! * [`layer::Overlay`] — a `wlr-layer-shell` surface for the fly. Floats above
//!   everything and, with an empty input region, lets every click through.
//! * [`window::Window`] — an ordinary xdg-toplevel for the brain view, which
//!   very much *does* want clicks.

pub mod canvas;
pub mod layer;
pub mod window;

pub use canvas::{Canvas, Rgba};
pub use layer::{Config, Overlay, ShellLayer};
pub use window::{Click, Window, WindowConfig};

/// What a draw callback returns.
pub enum Flow {
    /// Ask for another frame.
    Continue,
    /// Tear the surface down and return from the run loop.
    Exit,
}
