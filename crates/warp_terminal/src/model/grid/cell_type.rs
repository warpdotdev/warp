use super::cell::{Cell, Flags};

/// The type of a cell.
#[derive(Debug, Copy, Clone, PartialEq)]
pub enum CellType {
    /// A cell containing a standard single-width character.
    RegularChar,
    /// The first cell in a double-width "wide" character.
    WideChar,
    /// The second cell in a double-width "wide" character.
    WideCharSpacer,
    /// A spacer at the end of a row where a wide character had to be wrapped
    /// to the next row due to having a cell width of 2 but only one cell was
    /// left in the row.
    LeadingWideCharSpacer,
}

impl From<&Cell> for CellType {
    fn from(cell: &Cell) -> Self {
        // A cell is the base of a wide/Indic cluster if its span is > 1 --
        // NOT just when the legacy `WIDE_CHAR` flag is set, which `Cell::
        // set_span` only keeps in sync for span == 2 (backward compat for
        // code that still reads that boolean flag directly). Checking
        // `span()` here is what makes this correctly generalize to any
        // span 1-8, not just the fixed CJK width=2 case.
        if cell.span() > 1 {
            return Self::WideChar;
        }
        // First, check if the cell has _any_ of the relevant flags.  If not,
        // we're able to return NarrowChar with only one comparison/branch.
        // The other cell types are much less common, so we don't care as much
        // about the cost of extra comparisons for them.
        if !cell
            .flags()
            .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
        {
            Self::RegularChar
        } else if cell.flags().intersects(Flags::WIDE_CHAR_SPACER) {
            Self::WideCharSpacer
        } else {
            // At this point, there are no other possible cell types.
            Self::LeadingWideCharSpacer
        }
    }
}
