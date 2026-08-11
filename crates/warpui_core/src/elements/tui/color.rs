//! Conversions from GUI color primitives into terminal-cell colors.

use ratatui::style::Color;

use crate::elements::Fill;

impl From<Fill> for Color {
    fn from(fill: Fill) -> Self {
        match fill {
            Fill::None => Self::Reset,
            Fill::Solid(color) => Self::Rgb(color.r, color.g, color.b),
            Fill::Gradient {
                start_color,
                end_color,
                ..
            } => Self::Rgb(
                start_color.r.midpoint(end_color.r),
                start_color.g.midpoint(end_color.g),
                start_color.b.midpoint(end_color.b),
            ),
        }
    }
}
