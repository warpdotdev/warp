use fuzzy_match::match_indices_case_insensitive;

use super::{AgentConversationEntry, AgentConversationQueryResult};

pub(super) const DEFAULT_RESULT_COUNT: usize = 50;
pub(super) const MAX_SEARCH_RESULTS: usize = 500;
const MINIMUM_FUZZY_SCORE: i64 = 25;

/// Applies the shared conversation-menu recency and fuzzy-ranking policy.
pub fn query_conversation_entries(
    mut entries: Vec<AgentConversationEntry>,
    query: &str,
) -> Vec<AgentConversationQueryResult> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        entries.sort_by(|a, b| b.display.last_updated.cmp(&a.display.last_updated));
        entries.truncate(DEFAULT_RESULT_COUNT);
        entries.reverse();
        return entries
            .into_iter()
            .map(|entry| AgentConversationQueryResult {
                entry,
                title_match: None,
            })
            .collect();
    }

    let mut matches = entries
        .into_iter()
        .filter_map(|entry| {
            let title_match = match_indices_case_insensitive(&entry.display.title, &query)?;
            (title_match.score >= MINIMUM_FUZZY_SCORE).then_some(AgentConversationQueryResult {
                entry,
                title_match: Some(title_match),
            })
        })
        .collect::<Vec<_>>();
    matches.sort_by_key(|result| {
        let score = result
            .title_match
            .as_ref()
            .map_or(i64::MIN, |title_match| title_match.score);
        (score, result.entry.display.last_updated.timestamp_millis())
    });
    if matches.len() > MAX_SEARCH_RESULTS {
        matches.drain(..matches.len() - MAX_SEARCH_RESULTS);
    }
    matches
}
