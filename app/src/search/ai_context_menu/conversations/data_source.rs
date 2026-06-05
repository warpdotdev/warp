use std::collections::HashSet;

use fuzzy_match::FuzzyMatchResult;
use warpui::{AppContext, Entity, SingletonEntity};

use super::search_item::ConversationSearchItem;
use super::ConversationContextItem;
use crate::ai::agent_conversations_model::AgentConversationsModel;
use crate::ai::conversation_navigation::ConversationNavigationData;
use crate::search::ai_context_menu::mixer::AIContextMenuSearchableAction;
use crate::search::data_source::{Query, QueryResult};
use crate::search::mixer::{DataSourceRunErrorWrapper, SyncDataSource};

const MAX_RESULTS: usize = 50;
/// Minimum fuzzy match score to include a conversation in filtered results.
const MIN_FUZZY_SCORE: i64 = 25;
/// Score assigned to zero-state (unfiltered) results so they rank above low fuzzy matches.
const ZERO_STATE_SCORE: i64 = 1000;

pub struct ConversationDataSource;

impl ConversationDataSource {
    /// Merges local conversations and cloud agent tasks, deduplicated by
    /// `server_conversation_token`.
    fn collect_conversations(app: &AppContext) -> Vec<ConversationContextItem> {
        let mut seen_tokens: HashSet<String> = HashSet::new();
        let mut items: Vec<ConversationContextItem> = Vec::new();

        // Source 1: local + historical conversations (excludes ambient agent conversations).
        for nav in ConversationNavigationData::all_conversations(app) {
            if let Some(token) = &nav.server_conversation_token {
                if !seen_tokens.contains(token.as_str()) {
                    let token_str = token.as_str().to_string();
                    seen_tokens.insert(token_str.clone());
                    items.push(ConversationContextItem {
                        title: nav.title,
                        server_conversation_token: token_str,
                        last_updated: nav.last_updated.to_utc(),
                    });
                }
            }
        }

        // Source 2: cloud agent tasks. Every ambient agent conversation has a
        // corresponding task, so this covers all cloud conversations.
        let agent_model = AgentConversationsModel::as_ref(app);
        for task in agent_model.tasks_iter() {
            if let Some(conv_id) = &task.conversation_id {
                if seen_tokens.insert(conv_id.clone()) {
                    items.push(ConversationContextItem {
                        title: task.title.clone(),
                        server_conversation_token: conv_id.clone(),
                        last_updated: task.updated_at,
                    });
                }
            }
        }

        items
    }
}

impl SyncDataSource for ConversationDataSource {
    type Action = AIContextMenuSearchableAction;

    fn run_query(
        &self,
        query: &Query,
        app: &AppContext,
    ) -> Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper> {
        let all_conversations = Self::collect_conversations(app);
        let query_text = query.text.trim().to_lowercase();

        // Always sort by last_updated ascending so that position-based scores
        // assign higher values to more recently updated conversations. This ensures
        // recency acts as a tiebreaker when fuzzy scores are similar.
        let mut all_conversations = all_conversations;
        all_conversations.sort_by(|a, b| a.last_updated.cmp(&b.last_updated));
        let total_conversations = all_conversations.len();

        let mut results: Vec<QueryResult<Self::Action>> = if query_text.is_empty() {
            // Zero state: score encodes recency so the mixer orders newest items highest.
            all_conversations
                .into_iter()
                .enumerate()
                .map(|(index, item)| {
                    let search_item = ConversationSearchItem::new(
                        item,
                        FuzzyMatchResult {
                            score: ZERO_STATE_SCORE
                                + (30 * (index + 1) / total_conversations) as i64,
                            matched_indices: vec![],
                        },
                    );
                    QueryResult::from(search_item)
                })
                .collect()
        } else {
            // Fuzzy match on conversation title.
            all_conversations
                .into_iter()
                .enumerate()
                .filter_map(|(index, item)| {
                    let mut match_result =
                        fuzzy_match::match_indices_case_insensitive(&item.title, &query_text)?;

                    if match_result.score < MIN_FUZZY_SCORE {
                        return None;
                    }

                    // Add a recency bonus (capped at 30) so more recently updated
                    // conversations rank higher among results with similar fuzzy
                    // scores, regardless of the total number of conversations.
                    match_result.score += (30 * (index + 1) / total_conversations) as i64;

                    let search_item = ConversationSearchItem::new(item, match_result);
                    Some(QueryResult::from(search_item))
                })
                .collect()
        };

        results.sort_by_key(|r| std::cmp::Reverse(r.score()));
        results.truncate(MAX_RESULTS);
        Ok(results)
    }
}

impl Entity for ConversationDataSource {
    type Event = ();
}
