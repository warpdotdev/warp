use std::fmt::{self, Display, Formatter};
use std::ops::{Add, AddAssign, Range, Sub, SubAssign};

use serde::{Deserialize, Serialize};

#[derive(
    Default, Clone, Copy, Debug, Deserialize, PartialEq, PartialOrd, Eq, Ord, Hash, Serialize,
)]
pub struct BlockIndex(pub usize);

impl Display for BlockIndex {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl BlockIndex {
    pub fn zero() -> Self {
        Self(0)
    }

    pub fn range_as_iter(range: Range<BlockIndex>) -> impl Iterator<Item = BlockIndex> {
        (range.start.0..range.end.0).map(BlockIndex::from)
    }

    pub fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

impl From<usize> for BlockIndex {
    fn from(index: usize) -> Self {
        BlockIndex(index)
    }
}

impl From<BlockIndex> for usize {
    fn from(block_index: BlockIndex) -> usize {
        block_index.0
    }
}

impl Add for BlockIndex {
    type Output = BlockIndex;

    fn add(self, rhs: Self) -> Self::Output {
        Self(self.0 + rhs.0)
    }
}

impl AddAssign for BlockIndex {
    fn add_assign(&mut self, rhs: Self) {
        self.0 += rhs.0
    }
}

impl Sub for BlockIndex {
    type Output = BlockIndex;

    fn sub(self, rhs: Self) -> Self::Output {
        Self(self.0 - rhs.0)
    }
}

impl SubAssign for BlockIndex {
    fn sub_assign(&mut self, rhs: Self) {
        self.0 -= rhs.0
    }
}
