//! Types relating to styling.
//!
//! The split between foreground color and other styling is a performance
//! optimization.  Foreground color changes more frequently than other style
//! attribues, so combining all styling into one map means we're storing 16b
//! of data whenever foreground color changes, instead of only 8b.  Splitting
//! these into two maps improves cache line efficiency.
//!
//! TODO(vorporeal): Concretely validate the above assertion using benchmarks.

use get_size::GetSize;

use super::attribute_map::AttributeMap;
use crate::model::ansi;
use crate::model::grid::cell;

/// A map that holds foreground color information.
pub type FgColorMap = AttributeMap<ansi::Color>;

/// A map that holds background color and other styling information.
pub type BgAndStyleMap = AttributeMap<BgAndStyle>;

/// A bitmask for flags that represent style information.
const STYLE_MASK: cell::Flags = cell::Flags::from_bits_truncate(
    cell::Flags::INVERSE.bits()
        | cell::Flags::BOLD.bits()
        | cell::Flags::ITALIC.bits()
        | cell::Flags::UNDERLINE.bits()
        | cell::Flags::DOUBLE_UNDERLINE.bits()
        | cell::Flags::DIM.bits()
        | cell::Flags::HIDDEN.bits()
        | cell::Flags::STRIKEOUT.bits(),
);

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BgAndStyle {
    /// The background color for a cell.
    pub bg: ansi::Color,

    /// Additional styling-related flags.
    pub flags: cell::Flags,
}

impl Default for BgAndStyle {
    fn default() -> Self {
        Self {
            bg: ansi::Color::Named(ansi::NamedColor::Background),
            flags: cell::Flags::empty(),
        }
    }
}

impl GetSize for BgAndStyle {}

impl From<&cell::Cell> for BgAndStyle {
    fn from(value: &cell::Cell) -> Self {
        Self {
            bg: value.bg,
            flags: value.flags & STYLE_MASK,
        }
    }
}

impl PartialEq<&cell::Cell> for BgAndStyle {
    fn eq(&self, other: &&cell::Cell) -> bool {
        self.bg == other.bg && self.flags == (other.flags & STYLE_MASK)
    }
}
