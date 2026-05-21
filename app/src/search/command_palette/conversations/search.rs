use crate::ai::conversation_navigation::ConversationNavigationData;
use crate::search::command_palette::conversations::search_item::ConversationAction;
use crate::search::command_palette::conversations::search_item::ConversationSearchItem;
use crate::search::command_palette::conversations::DataSource;
use crate::search::data_source::QueryResult;
use crate::search::SyncDataSource;
use fuzzy_match::match_indices_case_insensitive;
use warpui::AppContext;

/// A conversation that was fuzzy matched against a search term.
#[derive(Debug)]
pub struct MatchedConversation {
    pub conversation: ConversationNavigationData,
    pub match_result: ConversationMatchResult,
}

impl MatchedConversation {
    /// Returns the score for the [`MatchedConversation`]. If there was no match result, a score of `0`
    /// is returned.
    pub fn score(&self) -> i64 {
        self.match_result.score
    }

    /// Returns the [`ConversationHighlightIndices`] belonging to the matched conversation.
    pub fn highlight_indices(&self) -> &ConversationHighlightIndices {
        &self.match_result.highlight_indices
    }
}

/// Result from matching a conversation.
#[derive(Debug)]
pub struct ConversationMatchResult {
    score: i64,
    highlight_indices: ConversationHighlightIndices,
}

impl ConversationMatchResult {
    /// Returns a dummy match result when there is no match.
    pub fn no_match() -> Self {
        ConversationMatchResult {
            score: 0,
            highlight_indices: ConversationHighlightIndices {
                title_indices: vec![],
                initial_query_indices: vec![],
                working_directory_indices: vec![],
            },
        }
    }

    pub fn score(&self) -> i64 {
        self.score
    }
}

/// Matching indices for a matched conversation.
#[derive(Debug)]
pub struct ConversationHighlightIndices {
    pub(super) title_indices: Vec<usize>,
    pub(super) initial_query_indices: Vec<usize>,
    pub(super) working_directory_indices: Vec<usize>,
}

impl ConversationHighlightIndices {
    fn new(
        title_indices: Vec<usize>,
        initial_query_indices: Vec<usize>,
        working_directory_indices: Vec<usize>,
    ) -> ConversationHighlightIndices {
        ConversationHighlightIndices {
            title_indices,
            initial_query_indices,
            working_directory_indices,
        }
    }

    /// Returns the highlight indices for the conversation title.
    pub fn title_indices(&self) -> &Vec<usize> {
        &self.title_indices
    }

    /// Returns the highlight indices for the initial query.
    pub fn initial_query_indices(&self) -> &Vec<usize> {
        &self.initial_query_indices
    }

    /// Returns the highlight indices for the working directory.
    pub fn working_directory_indices(&self) -> &Vec<usize> {
        &self.working_directory_indices
    }
}

/// Returns an iterator of conversations that match `search_term`.
pub fn filter_conversations<'a, 'b, I>(
    conversations_iter: I,
    search_term: &'b str,
) -> impl Iterator<Item = MatchedConversation> + use<'a, 'b, I>
where
    I: IntoIterator<Item = &'a ConversationNavigationData>,
{
    conversations_iter
        .into_iter()
        .filter_map(move |conversation| {
            if search_term.is_empty() {
                Some((ConversationMatchResult::no_match(), conversation.clone()))
            } else {
                // Match against title, initial_query, and initial_working_directory
                let title_match = match_indices_case_insensitive(&conversation.title, search_term);
                let initial_query_match =
                    conversation
                        .initial_query
                        .as_deref()
                        .and_then(|initial_query| {
                            match_indices_case_insensitive(initial_query, search_term)
                        });
                let working_directory_match = conversation
                    .initial_working_directory
                    .as_deref()
                    .and_then(|initial_working_directory| {
                        match_indices_case_insensitive(initial_working_directory, search_term)
                    });

                // If none of the fields match, filter this conversation out
                if title_match.is_none()
                    && initial_query_match.is_none()
                    && working_directory_match.is_none()
                {
                    return None;
                }

                // Determine the best score among all matches
                let best_score = [
                    title_match.as_ref(),
                    initial_query_match.as_ref(),
                    working_directory_match.as_ref(),
                ]
                .into_iter()
                .flatten()
                .map(|r| r.score)
                .max()
                .unwrap_or(0);

                let title_indices = title_match.map(|r| r.matched_indices).unwrap_or_default();
                let initial_query_indices = initial_query_match
                    .map(|r| r.matched_indices)
                    .unwrap_or_default();
                let working_directory_indices = working_directory_match
                    .map(|r| r.matched_indices)
                    .unwrap_or_default();

                let highlight_indices = ConversationHighlightIndices::new(
                    title_indices,
                    initial_query_indices,
                    working_directory_indices,
                );

                Some((
                    ConversationMatchResult {
                        score: best_score,
                        highlight_indices,
                    },
                    conversation.clone(),
                ))
            }
        })
        .map(|(match_result, conversation)| MatchedConversation {
            conversation,
            match_result,
        })
}

type SearcherAction = <DataSource as SyncDataSource>::Action;

pub trait ConversationSearcher {
    fn search(
        &self,
        _search_term: &str,
        _app: &AppContext,
    ) -> anyhow::Result<Vec<QueryResult<SearcherAction>>>;
}

pub struct FuzzyConversationSearcher;

impl FuzzyConversationSearcher {
    pub fn new() -> Self {
        Self
    }

    pub fn searchable_conversations(&self, app: &AppContext) -> Vec<ConversationNavigationData> {
        ConversationNavigationData::all_conversations(app)
    }
}

impl ConversationSearcher for FuzzyConversationSearcher {
    fn search(
        &self,
        search_term: &str,
        app: &AppContext,
    ) -> anyhow::Result<Vec<QueryResult<SearcherAction>>> {
        let conversations = self.searchable_conversations(app);
        Ok(filter_conversations(conversations.as_slice(), search_term)
            .map(|matched_conversation| {
                ConversationSearchItem::new(ConversationAction::Resume(Box::new(
                    matched_conversation,
                )))
                .into()
            })
            .collect())
    }
}

#[cfg(test)]
#[path = "search_tests.rs"]
mod tests;
