//! Data source for the inline conversation menu.

use itertools::Itertools;
use ordered_float::OrderedFloat;
use warpui::{AppContext, Entity, ModelHandle, SingletonEntity};

use crate::ai::agent_conversations_model::{
    AgentConversationEntry, AgentConversationEntryId, AgentManagementFilters,
};
use crate::ai::blocklist::agent_view::AgentViewController;
use crate::search::data_source::{Query, QueryFilter, QueryResult};
use crate::search::mixer::DataSourceRunErrorWrapper;
use crate::search::SyncDataSource;
use crate::terminal::input::conversations::search_item::ConversationSearchItem;
use crate::terminal::input::conversations::AcceptConversation;
use crate::terminal::model::session::active_session::ActiveSession;
use crate::workspace::RestoreConversationLayout;
use crate::AgentConversationsModel;

pub struct ConversationMenuDataSource {
    agent_view_controller: ModelHandle<AgentViewController>,
    active_session: ModelHandle<ActiveSession>,
}

impl ConversationMenuDataSource {
    pub fn new(
        agent_view_controller: ModelHandle<AgentViewController>,
        active_session: ModelHandle<ActiveSession>,
    ) -> Self {
        Self {
            agent_view_controller,
            active_session,
        }
    }

    fn entries(&self, app: &AppContext) -> Vec<AgentConversationEntry> {
        AgentConversationsModel::as_ref(app)
            .get_entries(&AgentManagementFilters::default(), app)
            .into_iter()
            .filter(|entry: &AgentConversationEntry| {
                entry.has_open_action(Some(RestoreConversationLayout::ActivePane), app)
            })
            .collect()
    }
}

impl SyncDataSource for ConversationMenuDataSource {
    type Action = AcceptConversation;

    fn run_query(
        &self,
        query: &Query,
        app: &AppContext,
    ) -> Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper> {
        let conversation_entries = self.entries(app);
        let query_text = query.text.trim().to_lowercase();
        let active_item_id = self
            .agent_view_controller
            .as_ref(app)
            .agent_view_state()
            .active_conversation_id()
            .map(AgentConversationEntryId::Conversation);

        let filter_by_cwd = query
            .filters
            .contains(&QueryFilter::CurrentDirectoryConversations);
        let session_pwd = if filter_by_cwd {
            self.active_session
                .as_ref(app)
                .current_working_directory()
                .cloned()
        } else {
            None
        };

        // When the "Current Directory" filter is active, include only conversations
        // whose most recent directory (falling back to initial directory) matches
        // the session's current working directory. If we can't determine the
        // session CWD, leave the results unfiltered.
        let matches_directory = |entry: &AgentConversationEntry| -> bool {
            if !filter_by_cwd {
                return true;
            }
            let Some(session_pwd) = session_pwd.as_deref() else {
                return true;
            };
            entry
                .display
                .working_directory
                .as_deref()
                .is_some_and(|dir| {
                    dir.trim_end_matches(std::path::MAIN_SEPARATOR)
                        == session_pwd.trim_end_matches(std::path::MAIN_SEPARATOR)
                })
        };

        if query_text.is_empty() {
            // By default, show 50 most recent conversations in the list.
            const DEFAULT_RESULT_COUNT: usize = 50;

            // In the zero state, sort conversations in the active pane above all other conversations.
            // Within each segment, sort to reverse chronological order.
            Ok(conversation_entries
                .into_iter()
                // Don't show the currently open conversation, that's redundant.
                .filter(|entry| Some(entry.id) != active_item_id)
                .filter(|entry| matches_directory(entry))
                .sorted_by(|a, b| b.display.last_updated.cmp(&a.display.last_updated))
                .take(DEFAULT_RESULT_COUNT)
                .map(|conversation_entry| {
                    QueryResult::from(ConversationSearchItem::new(conversation_entry))
                })
                .rev()
                .collect())
        } else {
            let mut search_results = conversation_entries
                .into_iter()
                .filter_map(|entry| {
                    if Some(entry.id) == active_item_id {
                        // Don't show the currently open conversation, that's redundant.
                        return None;
                    }
                    if !matches_directory(&entry) {
                        return None;
                    }
                    let match_result = fuzzy_match::match_indices_case_insensitive(
                        &entry.display.title,
                        &query_text,
                    )?;

                    // 25 is arbitrary.
                    if match_result.score < 25 {
                        return None;
                    }

                    Some(QueryResult::from(
                        ConversationSearchItem::new(entry)
                            .with_name_match_result(Some(match_result.clone()))
                            .with_score(OrderedFloat(match_result.score as f64)),
                    ))
                })
                .sorted_by(|a, b| b.score().cmp(&a.score()))
                .collect_vec();

            // This is basically here so the app doesn't choke.
            const MAX_SEARCH_RESULTS: usize = 500;

            search_results.truncate(MAX_SEARCH_RESULTS);
            Ok(search_results)
        }
    }
}

impl Entity for ConversationMenuDataSource {
    type Event = ();
}
