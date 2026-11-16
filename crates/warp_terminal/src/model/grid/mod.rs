mod displayed_output;
pub mod grid_handler;
mod grid_storage;
mod indexing;
mod selection_cursor;
mod storage;

pub(super) mod grapheme_cursor;
#[cfg(test)]
mod tests;

pub use displayed_output::RespectDisplayedOutput;
pub use grid_storage::*;
pub(super) use indexing::ConvertToAbsolute;
pub use indexing::IndexRegion;
pub use selection_cursor::SelectionCursor;

enum CursorDirection {
    Up,
    Down,
    Left,
    Right,
}

enum CursorState {
    Valid,
    Exhausted(CursorDirection),
    Invalid,
}
pub mod cell;
mod cell_type;
mod dimensions;
pub mod flat_storage;
pub mod hyperlink_registry;
pub mod row;

pub use cell_type::CellType;
pub use dimensions::Dimensions;
pub use flat_storage::FlatStorage;
pub use hyperlink_registry::{HyperlinkId, HyperlinkRegistry, MAX_DISTINCT_ENTRIES};
