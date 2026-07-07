//! Exports helper test-only methods for use in unit and integration tests.
use itertools::Itertools;

use super::block_list::BlockListMatch;
use super::{BlockListFindRun, TerminalFindModel};
use crate::terminal::model::terminal_model::BlockIndex;

impl TerminalFindModel {
    pub fn visible_block_list_match_count(&self) -> usize {
        self.block_list_find_run
            .as_ref()
            .map(|run| {
                run.matches()
                    .filter(|find_match| !find_match.is_filtered())
                    .collect_vec()
                    .len()
            })
            .unwrap_or(0)
    }
}

impl BlockListFindRun {
    pub fn matches_for_block(&self, index: BlockIndex) -> impl Iterator<Item = &BlockListMatch> {
        self.matches()
            .filter(move |find_match| find_match.matches_block(index))
    }

    pub fn focused_match_block_index(&self) -> Option<BlockIndex> {
        self.focused_match_index().and_then(|index| {
            self.matches().nth(index).and_then(|m| {
                if let BlockListMatch::CommandBlock(block_match) = m {
                    Some(block_match.block_index)
                } else {
                    None
                }
            })
        })
    }
}
