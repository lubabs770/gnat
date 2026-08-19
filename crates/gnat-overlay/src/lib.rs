//! The window the fly lives in.
//!
//! Not implemented yet. See README, milestone 2 — this crate exists so the
//! shape of the port is visible in the tree.
//!
//! The plan, concretely:
//!
//! * one `zwlr_layer_surface_v1` on the `overlay` layer, per `wl_output`;
//! * anchored to all four edges so it spans the output;
//! * `set_exclusive_zone(-1)` so it neither reserves space nor pushes tiles;
//! * `wl_surface.set_input_region` with an empty region, so every click and
//!   keystroke passes through to whatever is underneath. This is the piece
//!   that has to be proven before anything else is worth building, and unlike
//!   the macOS original it is a first-class protocol feature rather than a
//!   hack.
//!
//! The brain view is deliberately *not* a layer surface: it is an ordinary
//! xdg-toplevel, because it wants clicks.
