//! Data source for the inline plan menu.

use fuzzy_match::match_indices_case_insensitive;
use itertools::Itertools;
use ordered_float::OrderedFloat;
use warpui::{AppContext, Entity, SingletonEntity};

use crate::ai::agent::conversation::AIConversationId;
use crate::ai::document::ai_document_model::AIDocumentModel;
use crate::search::SyncDataSource;
use crate::search::data_source::{Query, QueryResult};
use crate::search::mixer::DataSourceRunErrorWrapper;
use crate::terminal::input::plans::AcceptPlan;
use crate::terminal::input::plans::search_item::PlanSearchItem;

pub struct PlanMenuDataSource {
    conversation_id: AIConversationId,
}

impl PlanMenuDataSource {
    pub fn new(conversation_id: AIConversationId) -> Self {
        Self { conversation_id }
    }

    pub fn set_conversation_id(&mut self, id: AIConversationId) {
        self.conversation_id = id;
    }
}

impl SyncDataSource for PlanMenuDataSource {
    type Action = AcceptPlan;

    fn run_query(
        &self,
        query: &Query,
        app: &AppContext,
    ) -> Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper> {
        let doc_model = AIDocumentModel::as_ref(app);
        let docs = doc_model.get_all_documents_for_conversation(self.conversation_id);
        let query_text = query.text.trim().to_lowercase();

        if query_text.is_empty() {
            // Documents are already sorted oldest-first (ascending created_at).
            // InlineMenuView renders items bottom-to-top (last item = bottom = most recent),
            // so this ordering places the most recent plan at the bottom as desired.
            return Ok(docs
                .into_iter()
                .enumerate()
                .map(|(idx, (id, doc))| QueryResult::from(PlanSearchItem::new(id, doc, idx)))
                .collect());
        }

        Ok(docs
            .into_iter()
            .filter_map(|(id, doc)| {
                let match_result = match_indices_case_insensitive(&doc.title, &query_text)?;
                // Avoid spamming results with extremely weak matches. 10 is arbitrary
                if match_result.score < 10 {
                    return None;
                }
                let score = OrderedFloat(match_result.score as f64);
                Some(QueryResult::from(
                    PlanSearchItem::new(id, doc, 0)
                        .with_name_match_result(Some(match_result))
                        .with_score(score),
                ))
            })
            .sorted_by(|a, b| b.score().cmp(&a.score()))
            .collect())
    }
}

impl Entity for PlanMenuDataSource {
    type Event = ();
}
