//! Data source for the user query menu.

use itertools::Itertools;
use ordered_float::OrderedFloat;
use warpui::{AppContext, Entity, SingletonEntity};

use crate::ai::agent::AIAgentExchangeId;
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::search::SyncDataSource;
use crate::search::data_source::{Query, QueryResult};
use crate::search::mixer::DataSourceRunErrorWrapper;
use crate::terminal::input::user_query::search_item::UserQuerySearchItem;

/// Action emitted when a query is selected in the user query menu.
#[derive(Clone, Debug)]
pub struct SelectUserQuery {
    pub exchange_id: AIAgentExchangeId,
}

pub struct UserQueryDataSource {
    conversation_id: AIConversationId,
}

impl UserQueryDataSource {
    pub fn new(conversation_id: AIConversationId) -> Self {
        Self { conversation_id }
    }

    pub fn set_conversation_id(&mut self, conversation_id: AIConversationId) {
        self.conversation_id = conversation_id;
    }
}

impl SyncDataSource for UserQueryDataSource {
    type Action = SelectUserQuery;

    fn run_query(
        &self,
        query: &Query,
        app: &AppContext,
    ) -> Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper> {
        let history_model = BlocklistAIHistoryModel::as_ref(app);
        let Some(conversation) = history_model.conversation(&self.conversation_id) else {
            return Ok(vec![]);
        };

        let search_query = query.text.trim().to_lowercase();
        if search_query.is_empty() {
            // With no search, we just return all queries in chronological order (oldest first).
            let results: Vec<QueryResult<Self::Action>> = conversation
                .root_task_exchanges()
                .filter(|exchange| exchange.has_user_query())
                .map(|exchange| {
                    let query_text = exchange.format_input_for_copy();
                    QueryResult::from(UserQuerySearchItem::new(exchange.id, query_text))
                })
                .collect();
            Ok(results)
        } else {
            // Filter by fuzzy matching and sort by score.
            let results = conversation
                .root_task_exchanges()
                .filter(|exchange| exchange.has_user_query())
                .filter_map(|exchange| {
                    let query_text = exchange.format_input_for_copy();
                    let match_result =
                        fuzzy_match::match_indices_case_insensitive(&query_text, &search_query)?;

                    Some(QueryResult::from(
                        UserQuerySearchItem::new(exchange.id, query_text)
                            .with_query_match_result(Some(match_result.clone()))
                            .with_score(OrderedFloat(match_result.score as f64)),
                    ))
                })
                .sorted_by(|a, b| b.score().cmp(&a.score()))
                .collect();
            Ok(results)
        }
    }
}

impl Entity for UserQueryDataSource {
    type Event = ();
}
