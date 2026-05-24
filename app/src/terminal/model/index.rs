//! Indexing-related newtypes for strongly typed tty/grid/terminal APIs.

pub use black_terminal::model::{Index, IndexRange, Point, VisiblePoint, VisibleRow};

/// The side of a cell.
pub type Side = Direction;

/// Horizontal direction.
#[derive(Debug, Copy, Clone, Eq, PartialEq, PartialOrd)]
pub enum Direction {
    Left,
    Right,
}

impl Direction {
    pub fn opposite(self) -> Self {
        match self {
            Side::Right => Side::Left,
            Side::Left => Side::Right,
        }
    }
}
