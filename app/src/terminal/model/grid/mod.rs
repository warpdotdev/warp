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
pub use warp_terminal::model::grid::row;

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
