// The color-mapping helpers (`compute_fg_rgb`, `compute_bg_rgb`, and
// `get_override_color`) below are adapted from the alacritty_terminal crate
// under the Apache license; see: crates/warp_terminal/src/model/LICENSE-ALACRITTY.

use pathfinder_color::ColorU;

use crate::{
    terminal::{
        color,
        model::{
            ansi::{color_index, Color, NamedColor},
            cell::{Cell, Flags},
            ObfuscateSecrets,
        },
    },
    util::color::OPAQUE,
};

use super::{BLOCK_FILTER_MATCH_COLOR, FOCUSED_MATCH_COLOR, MATCH_COLOR, URL_COLOR};

#[derive(PartialEq)]
pub(super) struct Secret {
    pub(super) hovered: bool,
    pub(super) is_obfuscated: bool,
}

/// Determines whether a match is focused.
#[derive(PartialEq, Clone, Copy)]
pub(super) enum IsFocused {
    Yes,
    No,
}

#[derive(Default, PartialEq)]
pub(super) struct CellType {
    pub(super) is_find_match: Option<IsFocused>,
    pub(super) is_url: bool,
    pub(super) secret: Option<Secret>,
    pub(super) is_filter_match: bool,
    pub(super) is_marked_text_char: bool,
}

impl CellType {
    pub(super) fn marked_text_char() -> Self {
        Self {
            is_marked_text_char: true,
            ..Default::default()
        }
    }

    pub(super) fn is_find_match(&self) -> bool {
        self.is_find_match.is_some()
    }

    pub(super) fn is_focused_find_match(&self) -> bool {
        self.is_find_match
            .map_or_else(|| false, |is_focused| matches!(is_focused, IsFocused::Yes))
    }

    pub(super) fn is_unfocused_find_match(&self) -> bool {
        self.is_find_match
            .map_or_else(|| false, |is_focused| matches!(is_focused, IsFocused::No))
    }

    pub(super) fn is_filter_match(&self) -> bool {
        self.is_filter_match
    }

    pub(super) fn is_url(&self) -> bool {
        self.is_url
    }

    pub(super) fn is_secret(&self) -> bool {
        self.secret.is_some()
    }

    pub(super) fn is_hovered_secret(&self) -> bool {
        self.secret
            .as_ref()
            .map_or_else(|| false, |secret| secret.hovered)
    }

    pub(super) fn is_unhovered_secret(&self) -> bool {
        self.secret
            .as_ref()
            .map_or_else(|| false, |secret| !secret.hovered)
    }

    /// Used to check if a CellType is equivalent to the default (i.e. no matches, urls, secrets, etc).
    pub(super) fn is_default(&self) -> bool {
        &Self::default() == self
    }

    pub(super) fn is_marked_text_char(&self) -> bool {
        self.is_marked_text_char
    }

    /// Calculate the foreground color for a cell. Does not set alpha value.
    pub(super) fn foreground_color(
        &self,
        cell: &Cell,
        colors: &color::List,
        override_colors: &color::OverrideList,
        obfuscate_mode: ObfuscateSecrets,
    ) -> ColorU {
        let is_unhovered_secret = self.is_unhovered_secret();

        if self.is_filter_match() {
            *BLOCK_FILTER_MATCH_COLOR
        } else if self.is_url() || self.is_hovered_secret() {
            *URL_COLOR
        } else if matches!(obfuscate_mode, ObfuscateSecrets::Strikethrough) && is_unhovered_secret {
            warpui::color::ColorU::new(128, 128, 128, 255)
        } else if self.is_default()
            || self.is_marked_text_char()
            || is_unhovered_secret
            || matches!(obfuscate_mode, ObfuscateSecrets::AlwaysShow)
        {
            compute_fg_rgb(colors, override_colors, cell.fg, cell.flags)
        } else {
            ColorU::black()
        }
    }

    /// Calculate the background color (including the alpha) for a cell.
    pub(super) fn background_color(
        &self,
        cell: &Cell,
        colors: &color::List,
        override_colors: &color::OverrideList,
    ) -> ColorU {
        let mut bg_color =
            if self.is_unfocused_find_match() && !self.is_hovered_secret() && !self.is_url() {
                *MATCH_COLOR
            } else if self.is_focused_find_match() && !self.is_hovered_secret() && !self.is_url() {
                *FOCUSED_MATCH_COLOR
            } else {
                compute_bg_rgb(colors, override_colors, cell.bg)
            };
        let bg_alpha = if cell.flags.contains(Flags::INVERSE) {
            OPAQUE
        } else if self.is_default()
            || self.is_url()
            || (self.is_filter_match() && !self.is_find_match())
            || (self.is_secret() && !self.is_find_match())
            || self.is_hovered_secret()
        {
            if cell.bg == Color::Named(NamedColor::Background) {
                // If the background of the cell is the same as the terminal background, treat it as
                // an alpha value of 0.
                0
            } else {
                OPAQUE
            }
        } else {
            OPAQUE
        };
        bg_color.a = bg_alpha;
        bg_color
    }
}

/// Get the RGB color from a cell's foreground color.
fn compute_fg_rgb(
    colors: &color::List,
    override_colors: &color::OverrideList,
    fg: Color,
    flags: Flags,
) -> ColorU {
    match fg {
        Color::Spec(rgb) => match flags & Flags::DIM {
            Flags::DIM => crate::terminal::color::dim(rgb),
            _ => rgb,
        },
        Color::Named(ansi) => {
            match flags & Flags::DIM_BOLD {
                // If no bright foreground is set, treat it like the BOLD flag doesn't exist.
                Flags::DIM_BOLD if ansi == NamedColor::Foreground => {
                    get_override_color(colors, override_colors, color_index::DIM_FOREGROUND)
                }
                // Cell is marked as dim and not bold.
                Flags::DIM => {
                    get_override_color(colors, override_colors, ansi.to_dim().into_color_index())
                }
                // None of the above, keep original color..
                _ => get_override_color(colors, override_colors, ansi.into_color_index()),
            }
        }
        Color::Indexed(idx) => {
            let idx = match (flags & Flags::DIM_BOLD, idx) {
                (Flags::DIM, 8..=15) => idx as usize - 8,
                (Flags::DIM, 0..=7) => color_index::DIM_BLACK + idx as usize,
                _ => idx as usize,
            };

            get_override_color(colors, override_colors, idx)
        }
    }
}

/// Get the RGB color from a cell's background color.
fn compute_bg_rgb(
    colors: &color::List,
    override_colors: &color::OverrideList,
    bg: Color,
) -> ColorU {
    match bg {
        Color::Spec(rgb) => rgb,
        Color::Named(ansi) => get_override_color(colors, override_colors, ansi.into_color_index()),
        Color::Indexed(idx) => get_override_color(colors, override_colors, idx as usize),
    }
}

pub fn get_override_color(
    colors: &color::List,
    override_colors: &color::OverrideList,
    index: usize,
) -> ColorU {
    override_colors[index].unwrap_or(colors[index])
}
