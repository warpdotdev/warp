use fuzzy_match::FuzzyMatchResult;
use warpui::{AppContext, Entity, SingletonEntity};

use super::search_item::RuleSearchItem;
use crate::ai::facts::{AIFact, CloudAIFactModel};
use crate::cloud_object::model::generic_string_model::GenericStringObjectId;
use crate::cloud_object::model::persistence::CloudModel;
use crate::cloud_object::CloudObject;
use crate::search::ai_context_menu::mixer::AIContextMenuSearchableAction;
use crate::search::data_source::{Query, QueryResult};
use crate::search::mixer::{DataSourceRunErrorWrapper, SyncDataSource};

const MAX_RESULTS: usize = 50;
const ZERO_STATE_BASE_SCORE: i64 = 1000;

pub struct RulesDataSource;

impl RulesDataSource {
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self
    }
}

impl SyncDataSource for RulesDataSource {
    type Action = AIContextMenuSearchableAction;

    fn run_query(
        &self,
        query: &Query,
        app: &AppContext,
    ) -> Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper> {
        let query_text = &query.text;

        let cloud_model = CloudModel::as_ref(app);
        let mut rule_results = Vec::new();

        let mut rules: Vec<_> = cloud_model
            .get_all_objects_of_type::<GenericStringObjectId, CloudAIFactModel>()
            .filter(|ai_fact| !ai_fact.is_trashed(cloud_model))
            .collect();

        // Sort by revision timestamp ascending so that position-based scores
        // assign higher values to more recently updated rules.
        rules.sort_by(|a, b| {
            let a_ts = a.metadata.revision.as_ref().map(|r| r.timestamp());
            let b_ts = b.metadata.revision.as_ref().map(|r| r.timestamp());
            a_ts.cmp(&b_ts)
        });

        let total_rules = rules.len();
        for (index, ai_fact) in rules.into_iter().enumerate() {
            let rule_uid = ai_fact.id.uid();
            let (rule_name, rule_content) = match &ai_fact.model().string_model {
                AIFact::Memory(memory) => (memory.name.clone(), memory.content.clone()),
            };
            let (match_result, is_match_on_rule_name) = if query_text.is_empty() {
                (
                    FuzzyMatchResult {
                        score: ZERO_STATE_BASE_SCORE + index as i64,
                        matched_indices: vec![],
                    },
                    false,
                )
            } else {
                let name_match = rule_name
                    .as_ref()
                    .and_then(|n| fuzzy_match::match_indices_case_insensitive(n, query_text));
                let content_match =
                    fuzzy_match::match_indices_case_insensitive(&rule_content, query_text);

                let (mut result, on_name) = match (name_match, content_match) {
                    (Some(name), Some(content)) if content.score > name.score => (content, false),
                    (Some(name), _) => (name, true),
                    (None, Some(content)) => (content, false),
                    (None, None) => continue,
                };
                // Add a recency bonus (capped at 30) so more recently updated
                // rules rank higher among results with similar fuzzy scores,
                // regardless of the total size of the rules collection.
                result.score += (30 * (index + 1) / total_rules) as i64;
                (result, on_name)
            };

            let search_item = RuleSearchItem {
                rule_uid,
                rule_name,
                rule_content,
                match_result,
                is_match_on_rule_name,
            };

            rule_results.push(QueryResult::from(search_item));
        }

        // Sort by score and take the top results
        rule_results.sort_by_key(|b| std::cmp::Reverse(b.score()));
        rule_results.truncate(MAX_RESULTS);

        Ok(rule_results)
    }
}

impl Entity for RulesDataSource {
    type Event = ();
}

#[cfg(test)]
#[path = "data_source_tests.rs"]
mod tests;
