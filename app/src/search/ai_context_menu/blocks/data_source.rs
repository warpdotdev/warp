use fuzzy_match::FuzzyMatchResult;
use itertools::Itertools;
use warpui::{AppContext, Entity, SingletonEntity};

use super::search_item::BlockSearchItem;
use crate::search::ai_context_menu::mixer::AIContextMenuSearchableAction;
use crate::search::data_source::{Query, QueryResult};
use crate::search::item::SearchItem;
use crate::search::mixer::{DataSourceRunErrorWrapper, SyncDataSource};
use crate::terminal::model::block::Block;
use crate::terminal::TerminalView;
use crate::workspace::ActiveSession;

const MAX_RESULTS: usize = 20;
const ZERO_STATE_BASE_SCORE: i64 = 1000;
const RECENCY_SCALE: usize = 30;
const ACTIVE_SESSION_BONUS: i64 = 5;

pub struct BlockDataSource;

impl BlockDataSource {
    #![cfg_attr(target_family = "wasm", allow(dead_code))]
    pub fn new() -> Self {
        Self
    }

    /// Helper function to process all eligible blocks from terminal views.
    /// The processor closure receives the command text, the block, and whether
    /// the block belongs to the currently active terminal session.
    fn process_eligible_blocks<F, R>(&self, app: &AppContext, mut processor: F) -> Vec<R>
    where
        F: FnMut(&str, &Block, bool) -> Option<R>,
    {
        let mut results = Vec::new();
        let active_session = ActiveSession::as_ref(app);

        // Iterate over all window IDs to search across all terminal views
        for window_id in app.window_ids() {
            let active_view_id = active_session.terminal_view_id(window_id);

            // Try to get all terminal views for this window
            if let Some(terminal_views) = app.views_of_type::<TerminalView>(window_id) {
                for terminal_view_handle in terminal_views {
                    let is_active =
                        active_view_id.is_some_and(|id| id == terminal_view_handle.id());
                    let terminal_view = terminal_view_handle.as_ref(app);
                    let terminal_model = terminal_view.model.lock();
                    let block_list = terminal_model.block_list();

                    // Process all eligible blocks
                    for block in block_list.blocks().iter() {
                        if !block.can_be_ai_context(block_list.agent_view_state()) {
                            continue;
                        }

                        let command = block.command_to_string();

                        // Skip empty commands
                        if command.trim().is_empty() {
                            continue;
                        }

                        if let Some(result) = processor(&command, block, is_active) {
                            results.push(result);
                        }
                    }
                }
            }
        }

        results
    }

    /// Create a BlockSearchItem from a command and block
    fn create_block_search_item(
        &self,
        command: String,
        block: &Block,
        match_result: FuzzyMatchResult,
        is_active_session: bool,
    ) -> BlockSearchItem {
        // Get output lines (limit to last 3 lines for performance)
        let output = block.output_to_string();
        let output_lines: Vec<String> = output
            .lines()
            .rev()
            .take(3)
            .map(|s| s.to_string())
            .collect();

        BlockSearchItem {
            block_id: block.id().clone(),
            command,
            directory: block.pwd().cloned(),
            exit_code: block.exit_code(),
            output_lines,
            completed_ts: block.completed_ts().cloned(),
            match_result,
            is_active_session,
        }
    }

    /// Get terminal blocks from all sessions' block lists by searching command text
    fn get_matching_blocks(&self, query: &str, app: &AppContext) -> Vec<BlockSearchItem> {
        let results = self.process_eligible_blocks(app, |command, block, is_active| {
            self.fuzzy_match_command(command, query)
                .map(|mut match_result| {
                    // Give active-session blocks a score bonus so they rank
                    // above equally-matched blocks from other sessions without
                    // being pinned to a separate priority tier.
                    if is_active {
                        match_result.score += ACTIVE_SESSION_BONUS;
                    }
                    self.create_block_search_item(
                        command.to_string(),
                        block,
                        match_result,
                        is_active,
                    )
                })
        });

        results
            .into_iter()
            .k_largest_relaxed_by_key(MAX_RESULTS, |item| item.score())
            .collect()
    }

    /// Handle zero-state query.
    ///
    /// Each block gets a composite score:
    ///   ZERO_STATE_BASE_SCORE + recency (0..RECENCY_SCALE) + active-session bonus
    ///
    /// Recency is position-based: sort all blocks by timestamp ascending,
    /// map position onto 0..RECENCY_SCALE. Active-session blocks get a
    /// flat ACTIVE_SESSION_BONUS on top. A very recent inactive block can
    /// outrank an old active block, but blocks of similar age will be
    /// boosted by the active-session bonus.
    ///
    /// Results are sorted descending by score and truncated to MAX_RESULTS.
    /// The mixer sorts ascending by (priority_tier, score, source_order)
    /// and the search bar reverses with .rev(), so higher scores appear
    /// at the top.
    fn run_zero_state_query(
        &self,
        app: &AppContext,
    ) -> Result<Vec<QueryResult<AIContextMenuSearchableAction>>, DataSourceRunErrorWrapper> {
        let mut results = self.process_eligible_blocks(app, |command, block, is_active| {
            let match_result = FuzzyMatchResult {
                score: 0,
                matched_indices: vec![],
            };
            Some(self.create_block_search_item(command.to_string(), block, match_result, is_active))
        });

        // Sort by timestamp ascending to assign position-based recency.
        results.sort_by(
            |a, b| match (a.completed_ts.as_ref(), b.completed_ts.as_ref()) {
                (Some(a_ts), Some(b_ts)) => a_ts.cmp(b_ts),
                (Some(_), None) => std::cmp::Ordering::Greater,
                (None, Some(_)) => std::cmp::Ordering::Less,
                (None, None) => std::cmp::Ordering::Equal,
            },
        );

        let total = results.len();
        for (index, item) in results.iter_mut().enumerate() {
            let recency = (RECENCY_SCALE * (index + 1) / total) as i64;
            let active_bonus = if item.is_active_session {
                ACTIVE_SESSION_BONUS
            } else {
                0
            };
            item.match_result.score = ZERO_STATE_BASE_SCORE + recency + active_bonus;
        }

        let mut query_results: Vec<QueryResult<AIContextMenuSearchableAction>> =
            results.into_iter().map(QueryResult::from).collect();
        query_results.sort_by_key(|r| std::cmp::Reverse(r.score()));
        query_results.truncate(MAX_RESULTS);

        Ok(query_results)
    }

    /// Handle non-empty query with fuzzy matching
    fn run_fuzzy_search_query(
        &self,
        app: &AppContext,
        query_text: &str,
    ) -> Result<Vec<QueryResult<AIContextMenuSearchableAction>>, DataSourceRunErrorWrapper> {
        let matching_blocks = self.get_matching_blocks(query_text, app);
        let results: Vec<QueryResult<AIContextMenuSearchableAction>> =
            matching_blocks.into_iter().map(QueryResult::from).collect();
        Ok(results)
    }

    fn fuzzy_match_command(&self, command: &str, query: &str) -> Option<FuzzyMatchResult> {
        fuzzy_match::match_indices_case_insensitive_ignore_spaces(command, query).map(
            |mut match_result| {
                // Normalize command and query for comparison
                let normalized_command = command
                    .split_whitespace()
                    .collect::<Vec<&str>>()
                    .join(" ")
                    .to_lowercase();
                let normalized_query = query
                    .split_whitespace()
                    .collect::<Vec<&str>>()
                    .join(" ")
                    .to_lowercase();

                let is_exact_match = normalized_command == normalized_query;

                // Check if query matches the root command (first word)
                let command_root = command
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .to_lowercase();
                let query_normalized = normalized_query.clone();
                let is_root_command_match = !is_exact_match && command_root == query_normalized;

                if is_exact_match {
                    // Apply highest boost for exact matches to prioritize them over everything else
                    match_result.score *= 10;
                } else if is_root_command_match {
                    // Apply medium boost for root command matches (e.g., "tail" matches "tail -f file.log")
                    // This should rank higher than partial matches from files but lower than exact matches
                    match_result.score *= 6;
                } else {
                    // Apply standard 3x weighted multiplier for other fuzzy matches
                    match_result.score *= 3;
                }

                match_result
            },
        )
    }
}

impl SyncDataSource for BlockDataSource {
    type Action = AIContextMenuSearchableAction;

    fn run_query(
        &self,
        query: &Query,
        app: &AppContext,
    ) -> Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper> {
        let query_text = &query.text;

        if query_text.is_empty() {
            // Zero state: prioritize active-session blocks, then recency
            self.run_zero_state_query(app)
        } else {
            // Non-empty query: fuzzy match against command text
            self.run_fuzzy_search_query(app, query_text)
        }
    }
}

impl Entity for BlockDataSource {
    type Event = ();
}

#[cfg(test)]
#[path = "data_source_tests.rs"]
mod tests;
