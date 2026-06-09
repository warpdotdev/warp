//! A terminal-UI (TUI) implementation of the UI framework's platform
//! abstraction.
//!
//! This backend runs a normal WarpUI app, but instead of rendering a [`Scene`]
//! to the GPU it rasterizes it to a grid of terminal cells (see [`render`]) and
//! drives input from the terminal. It is modeled closely on the `headless`
//! backend, sharing its channel-based main loop, but adds real rendering, a
//! terminal input thread, and `Redraw` plumbing.
//!
//! [`Scene`]: crate::Scene

mod app;
mod delegate;
mod event_loop;
mod font;
mod input;
mod render;
mod windowing;

pub use app::App;
