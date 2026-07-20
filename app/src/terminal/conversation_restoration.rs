//! Frontend-independent preparation for restoring an agent conversation into a terminal block
//! list.
//!
//! Restoration first rebuilds persisted command blocks in the terminal model, filters the
//! conversation down to agent exchanges that should be visible, and determines where each
//! exchange belongs relative to those command blocks. The resulting
//! [`ConversationBlockRestorationPlan`] lets GUI and TUI consumers create their own agent-block
//! views without duplicating command restoration or placement logic.

use chrono::{DateTime, Local};

use crate::ai::agent::AIAgentExchange;
use crate::ai::agent::conversation::AIConversation;
use crate::ai::blocklist::SerializedBlockListItem;
use crate::terminal::TerminalModel;
use crate::terminal::model::terminal_model::BlockIndex;
use crate::terminal::view::blocklist_filter::exchanges_for_blocklist;

/// One visible restored exchange and its position relative to command blocks.
pub struct RestoredConversationExchange {
    exchange: AIAgentExchange,
    command_block_index: Option<BlockIndex>,
}

impl RestoredConversationExchange {
    /// Returns the visible exchange to restore.
    pub fn exchange(&self) -> &AIAgentExchange {
        &self.exchange
    }

    /// Returns the command block before which the exchange should be inserted.
    pub fn command_block_index(&self) -> Option<BlockIndex> {
        self.command_block_index
    }
}

/// Frontend-neutral blocklist data prepared from one restored conversation.
pub struct ConversationBlockRestorationPlan {
    exchanges: Vec<RestoredConversationExchange>,
}

impl ConversationBlockRestorationPlan {
    /// Returns the visible exchanges represented by this plan.
    pub fn exchanges(&self) -> impl Iterator<Item = &AIAgentExchange> {
        self.exchanges.iter().map(|entry| &entry.exchange)
    }

    /// Consumes the plan into ordered restored exchanges.
    pub fn into_exchanges(self) -> Vec<RestoredConversationExchange> {
        self.exchanges
    }
}

/// Restores conversation-derived command blocks and plans agent-block placement.
pub fn prepare_conversation_block_restoration(
    conversation: &AIConversation,
    terminal_model: &mut TerminalModel,
) -> ConversationBlockRestorationPlan {
    let serialized_items = conversation.to_serialized_blocklist_items();
    if !serialized_items.is_empty() {
        let block_list = terminal_model.block_list_mut();
        for item in &serialized_items {
            match item {
                SerializedBlockListItem::Command { block } => {
                    block_list.insert_restored_block(block);
                }
            }
        }
    }

    let exchanges = exchanges_for_blocklist(conversation);
    let command_block_indices = command_block_indices_for_exchanges(
        terminal_model,
        exchanges.iter().copied(),
        exchanges.len(),
    );
    let exchanges = exchanges
        .into_iter()
        .zip(command_block_indices)
        .map(
            |(exchange, command_block_index)| RestoredConversationExchange {
                exchange: exchange.clone(),
                command_block_index,
            },
        )
        .collect();

    ConversationBlockRestorationPlan { exchanges }
}

/// Returns block indices where restored agent rich content should be inserted.
pub(crate) fn command_block_indices_for_exchanges<'a>(
    terminal_model: &TerminalModel,
    exchanges: impl Iterator<Item = &'a AIAgentExchange>,
    _exchange_count: usize,
) -> Vec<Option<BlockIndex>> {
    let blocks = terminal_model.block_list().blocks();
    let command_blocks: Vec<(BlockIndex, DateTime<Local>)> = blocks
        .iter()
        .enumerate()
        .filter_map(|(index, block)| {
            if !block.is_background() {
                block.start_ts().map(|ts| (BlockIndex::from(index), *ts))
            } else {
                None
            }
        })
        .collect();
    let exchange_timestamps: Vec<DateTime<Local>> =
        exchanges.map(|exchange| exchange.start_time).collect();

    find_block_indices_for_exchange_timestamps(&command_blocks, &exchange_timestamps)
}

/// Finds the earliest restored command block at or after each exchange timestamp.
fn find_block_indices_for_exchange_timestamps(
    command_blocks: &[(BlockIndex, DateTime<Local>)],
    exchange_timestamps: &[DateTime<Local>],
) -> Vec<Option<BlockIndex>> {
    let mut result = Vec::with_capacity(exchange_timestamps.len());

    for &exchange_timestamp in exchange_timestamps {
        let mut best: Option<(BlockIndex, DateTime<Local>)> = None;
        for &(idx, ts) in command_blocks.iter().rev() {
            if ts >= exchange_timestamp {
                if best.is_none_or(|(best_idx, best_ts)| {
                    ts < best_ts || (ts == best_ts && idx < best_idx)
                }) {
                    best = Some((idx, ts));
                }
            } else {
                break;
            }
        }

        result.push(best.map(|(idx, _)| idx));
    }

    result
}

#[cfg(test)]
#[path = "conversation_restoration_tests.rs"]
mod tests;
