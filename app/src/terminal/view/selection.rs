//! Free functions that turn the active text selection into a string.
//!
//! These live in the view layer (not on the terminal model) because rich content (AI) block
//! selections are owned by view types, which the model must not reach into.

use itertools::Itertools as _;
use sum_tree::SeekBias;
use warp_core::semantic_selection::SemanticSelection;
use warp_terminal::model::secrets::RespectObfuscatedSecrets;
use warp_terminal::model::selection::ExpandedSelectionRange;
use warpui::{AppContext, EntityId, ViewAsRef as _};

use crate::ai::blocklist::AIBlock;
use crate::ai::blocklist::block::PendingUserQueryBlock;
use crate::env_vars::env_var_collection_block::EnvVarCollectionBlock;
use crate::terminal::model::TerminalModel;
use crate::terminal::model::alt_screen::AltScreen;
use crate::terminal::model::blocks::{
    BlockHeight, BlockHeightItem, BlockHeightSummary, BlockList, RichContentItem,
};
use crate::terminal::warpify::success_block::WarpifySuccessBlock;

/// Returns **all** selected text across the entire `TerminalView` view hierarchy.
/// This includes selected text within regular blocks, AI blocks, inline actions, etc.
pub fn selection_to_string(
    model: &TerminalModel,
    semantic_selection: &SemanticSelection,
    inverted_blocklist: bool,
    ctx: &AppContext,
) -> Option<String> {
    if model.is_alt_screen_active() {
        alt_screen_selection_to_string(model.alt_screen(), semantic_selection)
    } else {
        block_list_selection_to_string(
            model.block_list(),
            semantic_selection,
            inverted_blocklist,
            ctx,
        )
    }
}

/// Returns the selected text within the block list, including text selected inside rich content
/// blocks that manage their own selections.
pub fn block_list_selection_to_string(
    block_list: &BlockList,
    semantic_selection: &SemanticSelection,
    inverted_blocklist: bool,
    ctx: &AppContext,
) -> Option<String> {
    match block_list.expand_selection(semantic_selection, inverted_blocklist) {
        Some(ExpandedSelectionRange::Regular { start, end, .. }) => {
            let start_within_grid_point = start.within_grid_point;
            let end_within_grid_point = end.within_grid_point;

            let mut selected_texts: Vec<String> = vec![];
            let mut selection_start_cursor = block_list
                .block_heights()
                .cursor::<BlockHeight, BlockHeightSummary>();
            let original_selection = block_list
                .selection()
                .expect("Selection should exist if it can be expanded");
            let mut top_row = original_selection.head_point().row;
            let mut bottom_row = original_selection.tail_point().row;

            // Ensure that top_row is always above bottom_row so we can loop based on block heights.
            if original_selection.tail_point().row < original_selection.head_point().row {
                top_row = original_selection.tail_point().row;
                bottom_row = original_selection.head_point().row;
            }
            selection_start_cursor.seek(&BlockHeight::from(top_row), SeekBias::Right);

            // Loop over each block, adding their contents to the output.
            let transcript_scope = block_list.transcript_scope();
            while bottom_row >= selection_start_cursor.start().height {
                let Some(item) = selection_start_cursor.item() else {
                    // We reached the end of the block list.
                    break;
                };
                // Otherwise, accumulate selection depending on block type.
                match item {
                    BlockHeightItem::Block { .. } => {
                        let block_index = selection_start_cursor.start().block_count.into();
                        if let Some(command_block) = block_list.block_at(block_index) {
                            // Don't copy hidden or empty blocks.
                            if command_block.is_empty(transcript_scope) {
                                selection_start_cursor.next();
                                continue;
                            }

                            let start_point = if block_index == start.within_grid_point.block_index
                            {
                                start_within_grid_point.into()
                            } else {
                                command_block.start_point()
                            };
                            let end_point = if block_index == end.within_grid_point.block_index {
                                end_within_grid_point.into()
                            } else {
                                command_block.end_point()
                            };

                            selected_texts
                                .push(command_block.bounds_to_string(start_point, end_point));
                        }
                    }
                    BlockHeightItem::RichContent(RichContentItem { view_id, .. }) => {
                        if let Some(selected_text) = read_selected_text_from_ai_block(*view_id, ctx)
                        {
                            selected_texts.push(selected_text);
                        }
                        if let Some(selected_text) =
                            read_selected_text_from_pending_user_query_block(*view_id, ctx)
                        {
                            selected_texts.push(selected_text);
                        }

                        if let Some(active_window_id) = ctx.windows().active_window()
                            && let Some(ssh_block) =
                                ctx.view_with_id::<WarpifySuccessBlock>(active_window_id, *view_id)
                        {
                            let warpify_success_block = ctx.view(&ssh_block);
                            if let Some(selected_text) = warpify_success_block.selected_text() {
                                selected_texts.push(selected_text);
                            }
                        }
                    }
                    BlockHeightItem::Gap(_)
                    | BlockHeightItem::RestoredBlockSeparator { .. }
                    | BlockHeightItem::InlineBanner { .. }
                    | BlockHeightItem::SubshellSeparator { .. } => {}
                }

                selection_start_cursor.next();
            }

            if inverted_blocklist {
                selected_texts.reverse();
            }

            Some(selected_texts.join("\n"))
        }
        Some(ExpandedSelectionRange::Rect { rows }) => {
            let mut selected_texts: Vec<String> = vec![];

            let mut selection_start_cursor = block_list
                .block_heights()
                .cursor::<BlockHeight, BlockHeightSummary>();
            let original_selection = block_list
                .selection()
                .expect("Selection should exist if it can be expanded");

            let head_row = original_selection.head_point().row;
            let tail_row = original_selection.tail_point().row;
            let top_row = head_row.min(tail_row);
            let bottom_row = head_row.max(tail_row);

            selection_start_cursor.seek(&BlockHeight::from(top_row), SeekBias::Right);

            // Loop over each _command block_ row in the rect selection. Add the content to the selected_texts result.
            // Note that there could be rich content blocks in between the command block rows. Therefore in each iteration
            // we need to check and append the intermediate rich content selections.
            for (start, end) in rows {
                let current_row = start.absolute_point.row;

                // Read rich content selected text in the intermediate rich content blocks.
                while current_row >= selection_start_cursor.start().height {
                    if let Some(BlockHeightItem::RichContent(item)) = selection_start_cursor.item()
                    {
                        if let Some(selected_text) =
                            read_selected_text_from_ai_block(item.view_id, ctx)
                        {
                            selected_texts.push(selected_text);
                        }
                        if let Some(selected_text) =
                            read_selected_text_from_pending_user_query_block(item.view_id, ctx)
                        {
                            selected_texts.push(selected_text);
                        }
                    }
                    selection_start_cursor.next();
                }
                let Some(command_block) = block_list.block_at(start.within_grid_point.block_index)
                else {
                    continue;
                };
                let start_point = start.within_grid_point.into();
                let end_point = end.within_grid_point.into();
                selected_texts.push(command_block.bounds_to_string(start_point, end_point));
            }

            // Read AI block selected text in the trailing AI blocks.
            while bottom_row >= selection_start_cursor.start().height {
                if let Some(BlockHeightItem::RichContent(item)) = selection_start_cursor.item() {
                    if let Some(selected_text) = read_selected_text_from_ai_block(item.view_id, ctx)
                    {
                        selected_texts.push(selected_text);
                    }
                    if let Some(selected_text) =
                        read_selected_text_from_pending_user_query_block(item.view_id, ctx)
                    {
                        selected_texts.push(selected_text);
                    }
                }
                selection_start_cursor.next();
            }

            Some(selected_texts.join("\n"))
        }
        None => {
            // Check if there are rich content blocks in the selection. This is to cover
            // an edge case when selection only spans rich content blocks, expand_selection
            // will return None.
            let ids = block_list.rich_content_blocks_in_selection();

            if ids.is_empty() {
                return None;
            }

            let mut selected_texts = vec![];
            for view_id in ids {
                if let Some(selected_text) = read_selected_text_from_ai_block(view_id, ctx) {
                    selected_texts.push(selected_text);
                }

                if let Some(active_window_id) = ctx.windows().active_window() {
                    if let Some(env_var_block) =
                        ctx.view_with_id::<EnvVarCollectionBlock>(active_window_id, view_id)
                    {
                        let block = ctx.view(&env_var_block);
                        if let Some(selected_text) = block.selected_text(ctx) {
                            selected_texts.push(selected_text);
                        }
                    }

                    if let Some(ssh_block) =
                        ctx.view_with_id::<WarpifySuccessBlock>(active_window_id, view_id)
                    {
                        let warpify_success_block = ctx.view(&ssh_block);
                        if let Some(selected_text) = warpify_success_block.selected_text() {
                            selected_texts.push(selected_text);
                        }
                    }
                }

                if let Some(selected_text) =
                    read_selected_text_from_pending_user_query_block(view_id, ctx)
                {
                    selected_texts.push(selected_text);
                }
            }

            // TODO: If `selected_texts` is empty, should we return `None` instead of `Some("")`?
            // As of 02/18/2025, this scenario can be reproduced by single-clicking anywhere on an AI response block.
            Some(selected_texts.join("\n"))
        }
    }
}

/// Returns the selected text within the alt screen.
pub fn alt_screen_selection_to_string(
    alt_screen: &AltScreen,
    semantic_selection: &SemanticSelection,
) -> Option<String> {
    let selection_range = alt_screen.selection_range(semantic_selection)?;
    Some(match selection_range {
        ExpandedSelectionRange::Regular { start, end, .. } => {
            alt_screen.bounds_to_string(start, end, RespectObfuscatedSecrets::Yes)
        }
        ExpandedSelectionRange::Rect { rows } => rows
            .into_iter()
            .map(|(start, end)| {
                alt_screen.bounds_to_string(start, end, RespectObfuscatedSecrets::Yes)
            })
            .join("\n"),
    })
}

/// Given the view id of an AI block, return the active selected text in that block.
fn read_selected_text_from_ai_block(view_id: EntityId, ctx: &AppContext) -> Option<String> {
    let active_window_id = ctx.windows().active_window()?;

    let ai_block = ctx.view_with_id::<AIBlock>(active_window_id, view_id)?;
    let ai_block_view = ctx.view(&ai_block);
    ai_block_view.selected_text(ctx)
}

/// Given the view id of a pending user query block, return the active selected text in that block.
fn read_selected_text_from_pending_user_query_block(
    view_id: EntityId,
    ctx: &AppContext,
) -> Option<String> {
    let active_window_id = ctx.windows().active_window()?;

    let pending_user_query_block =
        ctx.view_with_id::<PendingUserQueryBlock>(active_window_id, view_id)?;
    let pending_user_query_block_view = ctx.view(&pending_user_query_block);
    pending_user_query_block_view.selected_text(ctx)
}

#[cfg(test)]
#[path = "selection_tests.rs"]
mod tests;
