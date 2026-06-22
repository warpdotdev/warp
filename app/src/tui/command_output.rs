//! Turns captured shell-command output bytes into a styled cell grid.
//!
//! This is the byte -> grid seam for the TUI's `!`-shell feature: it parses ANSI
//! SGR escape sequences (colors, bold, etc.) into a ratatui `Text` via
//! [`ansi_to_tui`], then rasterizes that text into a [`TuiBuffer`] wrapped to the
//! given width using [`rasterize_text`]. Keeping the signature `bytes + width ->
//! grid` makes it the single place to later swap in a full VT screen
//! interpreter (e.g. `vt100` over a PTY) without touching the view or element
//! layers.

use ansi_to_tui::IntoText as _;
use warpui_core::elements::tui::{rasterize_text, TuiBuffer};

/// Cap on rasterized output lines (mirrors codex's user-shell preview cap).
/// Keeps the most recent lines: the transcript is bottom-anchored, so the
/// newest output is what stays on screen, and this bounds the grid size for
/// high-volume commands.
const MAX_OUTPUT_LINES: usize = 200;

/// Parses `bytes` (which may contain ANSI SGR escapes) and rasterizes them into
/// a [`TuiBuffer`] wrapped to `width` columns, keeping only the most recent
/// [`MAX_OUTPUT_LINES`] lines. Invalid UTF-8 is decoded lossily so arbitrary
/// command output never fails to render.
pub fn render_output_to_buffer(bytes: &[u8], width: u16) -> TuiBuffer {
    // `into_text` only fails on invalid UTF-8, so fall back to a lossy decode in
    // that case (the result is then guaranteed-valid UTF-8 and parses cleanly).
    let mut text = match std::str::from_utf8(bytes) {
        Ok(valid) => valid.into_text(),
        Err(_) => String::from_utf8_lossy(bytes).into_owned().into_text(),
    }
    .unwrap_or_default();

    // Drop the oldest lines beyond the cap so the grid (and the wrap/measure
    // work in `rasterize_text`) stays bounded.
    if text.lines.len() > MAX_OUTPUT_LINES {
        let drop = text.lines.len() - MAX_OUTPUT_LINES;
        text.lines.drain(0..drop);
    }

    rasterize_text(text, width)
}

#[cfg(test)]
#[path = "command_output_tests.rs"]
mod tests;
