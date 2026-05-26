//! Run-length-encoded storage for the OSC 8 `hyperlink_id` attribute,
//! parallel to [`super::style::FgColorMap`] and [`super::style::BgAndStyleMap`].
//!
//! A typical OSC 8 span covers many adjacent cells with the same id, so RLE
//! deduplicates a 100-cell hyperlink into a single map entry. The default
//! attribute value is `None` (cell is not part of a hyperlink), which RLE-
//! collapses unbroken runs of plain output to no map entries at all — keeping
//! the storage cost zero for the overwhelming majority of cells.

use crate::model::grid::{cell, HyperlinkId};

use super::attribute_map::AttributeMap;

/// Map holding each cell's OSC 8 hyperlink id (or `None` for cells that
/// aren't part of a hyperlink).
pub type HyperlinkIdMap = AttributeMap<Option<HyperlinkId>>;

impl From<&cell::Cell> for Option<HyperlinkId> {
    fn from(value: &cell::Cell) -> Self {
        value.hyperlink_id()
    }
}

impl PartialEq<&cell::Cell> for Option<HyperlinkId> {
    fn eq(&self, other: &&cell::Cell) -> bool {
        *self == other.hyperlink_id()
    }
}
