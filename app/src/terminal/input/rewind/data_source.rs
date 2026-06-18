//! Data source for the rewind menu.

use std::collections::HashSet;

use ai::agent::action_result::RequestFileEditsResult;
use itertools::Itertools;
use ordered_float::OrderedFloat;
use warpui::{AppContext, Entity, SingletonEntity};

use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::{AIAgentActionResultType, AIAgentExchangeId, AIAgentInput};
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::search::data_source::{Query, QueryResult};
use crate::search::mixer::DataSourceRunErrorWrapper;
use crate::search::SyncDataSource;
use crate::terminal::input::rewind::search_item::RewindSearchItem;

/// Action emitted when a rewind point is selected.
#[derive(Clone, Debug)]
pub struct SelectRewindPoint {
    /// The exchange ID to rewind to, or None for "Current" (dismiss without rewinding).
    pub exchange_id: Option<AIAgentExchangeId>,
}

/// Information about file changes for a rewind point.
#[derive(Debug, Clone, Default)]
pub struct FileChangesInfo {
    pub lines_added: usize,
    pub lines_removed: usize,
}

pub struct RewindDataSource {
    conversation_id: AIConversationId,
}

impl RewindDataSource {
    pub fn new(conversation_id: AIConversationId) -> Self {
        Self { conversation_id }
    }

    pub fn set_conversation_id(&mut self, conversation_id: AIConversationId) {
        self.conversation_id = conversation_id;
    }

    /// Get file changes info for a "block" of exchanges (from user query to next user query)
    fn get_file_changes_for_block(
        exchanges: &[&crate::ai::agent::AIAgentExchange],
    ) -> FileChangesInfo {
        let mut file_paths: HashSet<String> = HashSet::new();
        let mut total_lines_added: usize = 0;
        let mut total_lines_removed: usize = 0;

        for exchange in exchanges {
            for input in &exchange.input {
                if let Some(action_result) = input.action_result() {
                    if let AIAgentActionResultType::RequestFileEdits(
                        RequestFileEditsResult::Success {
                            updated_files,
                            lines_added,
                            lines_removed,
                            ..
                        },
                    ) = &action_result.result
                    {
                        total_lines_added += lines_added;
                        total_lines_removed += lines_removed;

                        for updated_file in updated_files {
                            file_paths.insert(updated_file.file_context.file_name.clone());
                        }
                    }
                }
            }
        }

        if file_paths.is_empty() {
            return FileChangesInfo::default();
        }

        FileChangesInfo {
            lines_added: total_lines_added,
            lines_removed: total_lines_removed,
        }
    }
}

impl SyncDataSource for RewindDataSource {
    type Action = SelectRewindPoint;

    fn run_query(
        &self,
        query: &Query,
        app: &AppContext,
    ) -> Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper> {
        let history_model = BlocklistAIHistoryModel::as_ref(app);
        let Some(conversation) = history_model.conversation(&self.conversation_id) else {
            return Ok(vec![]);
        };

        let all_exchanges = conversation.root_task_exchanges().collect_vec();

        // Find indices of exchanges with user queries
        let user_query_indices: Vec<usize> = all_exchanges
            .iter()
            .enumerate()
            .filter_map(|(idx, ex)| ex.has_user_query().then_some(idx))
            .collect();

        let search_query = query.text.trim().to_lowercase();

        // Build rewind points in chronological order (oldest first)
        let mut results = Vec::new();

        for &exchange_idx in user_query_indices.iter() {
            let exchange = &all_exchanges[exchange_idx];
            let query_text = exchange
                .input
                .iter()
                .find_map(AIAgentInput::display_query)
                .unwrap_or_default();

            // Find the end of this "block" - either the next user query or end of exchanges
            let next_user_query_idx = user_query_indices
                .iter()
                .find(|&&idx| idx > exchange_idx)
                .copied()
                .unwrap_or(all_exchanges.len());

            // Get file changes for all exchanges in this block
            let exchanges_in_block = &all_exchanges[exchange_idx..next_user_query_idx];
            let file_changes = Self::get_file_changes_for_block(exchanges_in_block);

            // Filter by search query if present
            if !search_query.is_empty() {
                let match_result =
                    fuzzy_match::match_indices_case_insensitive(&query_text, &search_query);
                if let Some(match_result) = match_result {
                    results.push(QueryResult::from(
                        RewindSearchItem::new_rewind_point(exchange.id, query_text, file_changes)
                            .with_query_match_result(Some(match_result.clone()))
                            .with_score(OrderedFloat(match_result.score as f64)),
                    ));
                }
            } else {
                results.push(QueryResult::from(RewindSearchItem::new_rewind_point(
                    exchange.id,
                    query_text,
                    file_changes,
                )));
            }
        }

        // Sort by score if filtering
        if !search_query.is_empty() {
            results.sort_by_key(|b| std::cmp::Reverse(b.score()));
        } else {
            // Add "Current" as the last item (appears at bottom, closest to input)
            results.push(QueryResult::from(RewindSearchItem::new_current()));
        }

        Ok(results)
    }
}

impl Entity for RewindDataSource {
    type Event = ();
}
