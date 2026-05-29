//! A brand-new terminal-UI surface for interacting with the Warp agent.
//!
//! This is *not* a TUI rendering of the desktop app; it is a distinct UI built
//! from new, TUI-specific components ([`components`]) composed into a root view
//! ([`agent_view`]). The view observes a [`agent_bridge::TuiAgentBridge`] model
//! that drives the real (headless) agent and exposes streamed output, and
//! [`bootstrap`] opens a window backed by the TUI platform backend
//! (`warpui::platform::tui`).
//!
//! Rendering goes through the normal WarpUI pipeline: views produce `Element`s
//! that lay out and paint into a `Scene`, which the TUI backend rasterizes to
//! the terminal. Components are authored in *cell units* (1 cell == 1 "pixel").

pub mod agent_bridge;
pub mod agent_view;
pub mod bootstrap;
pub mod components;
