//! Data source for the inline conversation menu.
use std::collections::HashSet;

use ordered_float::OrderedFloat;
use warpui::{AppContext, Entity, ModelHandle, SingletonEntity};

use crate::ai::agent_conversations_model::{
    query_conversation_entries, AgentConversationEntry, AgentConversationListEntryState,
    AgentManagementFilters,
};
use crate::ai::blocklist::conversation_selection::ConversationSelectionHandle;
use crate::search::data_source::{Query, QueryFilter, QueryResult};
use crate::search::mixer::DataSourceRunErrorWrapper;
use crate::search::SyncDataSource;
use crate::terminal::input::conversations::search_item::ConversationSearchItem;
use crate::terminal::input::conversations::AcceptConversation;
use crate::terminal::model::session::active_session::ActiveSession;
use crate::AgentConversationsModel;

pub struct ConversationMenuDataSource {
    conversation_selection: ConversationSelectionHandle,
    active_session: ModelHandle<ActiveSession>,
}

impl ConversationMenuDataSource {
    pub fn new(
        conversation_selection: ConversationSelectionHandle,
        active_session: ModelHandle<ActiveSession>,
    ) -> Self {
        Self {
            conversation_selection,
            active_session,
        }
    }

    fn entries(&self, app: &AppContext) -> Vec<(AgentConversationEntry, bool)> {
        let policy = self.conversation_selection.as_ref(app);
        AgentConversationsModel::as_ref(app)
            .get_entries(&AgentManagementFilters::default(), app)
            .into_iter()
            .filter_map(|entry| match policy.classify_entry(&entry, app) {
                AgentConversationListEntryState::Available => Some((entry, false)),
                AgentConversationListEntryState::OpenElsewhere => Some((entry, true)),
                AgentConversationListEntryState::Selected
                | AgentConversationListEntryState::Unavailable => None,
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

        let mut open_elsewhere_ids = HashSet::new();
        let entries = conversation_entries
            .into_iter()
            .filter_map(|(entry, is_open_elsewhere)| {
                if !matches_directory(&entry) {
                    return None;
                }
                if is_open_elsewhere {
                    open_elsewhere_ids.insert(entry.id);
                }
                Some(entry)
            })
            .collect();
        Ok(query_conversation_entries(entries, &query.text)
            .into_iter()
            .map(|result| {
                let is_open_elsewhere = open_elsewhere_ids.contains(&result.entry.id);
                let mut item = ConversationSearchItem::new(result.entry, is_open_elsewhere);
                if let Some(title_match) = result.title_match {
                    item = item
                        .with_score(OrderedFloat(title_match.score as f64))
                        .with_name_match_result(Some(title_match));
                }
                QueryResult::from(item)
            })
            .collect())
    }
}

impl Entity for ConversationMenuDataSource {
    type Event = ();
}
