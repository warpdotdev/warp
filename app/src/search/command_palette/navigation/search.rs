use std::ops::Range;

use fuzzy_match::match_indices_case_insensitive;
use itertools::Itertools;
use warpui::{AppContext, ModelHandle};

use crate::pane_group::PaneId;
use crate::search::command_palette::navigation::render::CommandRenderInfo;
use crate::search::command_palette::navigation::search_item::SearchItem;
use crate::search::command_palette::navigation::DataSource;
use crate::search::data_source::QueryResult;
use crate::search::SyncDataSource;
use crate::session_management::{CommandContext, SessionNavigationData, SessionSource};

/// A session that was fuzzy matched against a search term.
pub struct MatchedSession {
    pub session: SessionNavigationData,
    pub match_result: SessionMatchResult,
}

impl MatchedSession {
    /// Returns the score for the [`MatchedSession`]. If there was no match result, a score of `0`
    /// is returned.
    pub fn score(&self) -> i64 {
        self.match_result.score
    }

    /// Returns the [`SessionHighlightIndices`] belonging to the matched session.
    pub fn highlight_indices(&self) -> &SessionHighlightIndices {
        &self.match_result.highlight_indices
    }
}

/// Result from matching a session.
#[derive(Debug)]
pub struct SessionMatchResult {
    score: i64,
    highlight_indices: SessionHighlightIndices,
}

impl SessionMatchResult {
    /// Returns a dummy match result when there is no match.
    pub fn no_match() -> Self {
        SessionMatchResult {
            score: 0,
            highlight_indices: SessionHighlightIndices {
                command_indices: None,
                hint_text_indices: vec![],
            },
        }
    }
}

/// Matching indices for a matched session.
#[derive(Debug)]
pub struct SessionHighlightIndices {
    pub(super) command_indices: Option<Vec<usize>>,
    pub(super) hint_text_indices: Vec<usize>,
}

impl SessionHighlightIndices {
    fn new(
        matched_indices: Vec<usize>,
        session_highlights: SearchableSessionStringRanges,
    ) -> SessionHighlightIndices {
        // Allow lazy evaluations here. Using `then_some` will eagerly compute these
        // values, which can lead to underflow.
        #[allow(clippy::unnecessary_lazy_evaluations)]
        let command_indices = session_highlights.command_range.map(|command_range| {
            matched_indices
                .iter()
                .filter(|&idx| command_range.contains(idx))
                .map(|idx| *idx - command_range.start)
                .collect::<Vec<usize>>()
        });

        #[allow(clippy::unnecessary_lazy_evaluations)]
        let hint_text_indices = matched_indices
            .iter()
            .filter(|&idx| session_highlights.hint_text_range.contains(idx))
            .map(|idx| *idx - session_highlights.hint_text_range.start)
            .collect::<Vec<usize>>();

        SessionHighlightIndices {
            command_indices,
            hint_text_indices,
        }
    }
}

/// Returns an iterator of sessions that match `search_term`.
pub fn filter_sessions<'a, 'b, I>(
    sessions_iter: I,
    search_term: &'b str,
) -> impl Iterator<Item = MatchedSession> + use<'a, 'b, I>
where
    I: IntoIterator<Item = &'a SessionNavigationData>,
{
    sessions_iter
        .into_iter()
        .filter_map(move |session| {
            if search_term.is_empty() {
                Some((SessionMatchResult::no_match(), session.clone()))
            } else {
                let (searchable_string, session_highlights) =
                    searchable_session_string_and_ranges(session);

                match_indices_case_insensitive(&searchable_string, search_term).map(|result| {
                    let highlight_indices =
                        SessionHighlightIndices::new(result.matched_indices, session_highlights);
                    (
                        SessionMatchResult {
                            score: result.score,
                            highlight_indices,
                        },
                        session.clone(),
                    )
                })
            }
        })
        .map(|(match_result, session)| MatchedSession {
            session,
            match_result,
        })
}

/// The searchable string format is: [prompt] [command] [hint text],
/// where [command] may or may not be present.
fn searchable_session_string_and_ranges(
    session: &SessionNavigationData,
) -> (String, SearchableSessionStringRanges) {
    let mut searchable_string = session.prompt().to_string();
    let prompt_end = session.prompt().chars().count();

    let command_range = match session.command_context() {
        CommandContext::LastRunCommand {
            last_run_command,
            mins_since_completion: _,
        } => {
            // Fuzzy search gives different weights to characters in the same word vs different words.
            searchable_string.push(' ');
            searchable_string.push_str(last_run_command.as_str());

            let start = prompt_end + 1;
            let end = start + last_run_command.chars().count();
            Some(start..end)
        }
        CommandContext::RunningCommand { running_command } => {
            // Fuzzy search gives different weights to characters in the same word vs different words.
            searchable_string.push(' ');
            searchable_string.push_str(running_command.as_str());

            let start = prompt_end + 1;
            let end = start + running_command.chars().count();
            Some(start..end)
        }
        CommandContext::LastRunAIBlock { prompt } | CommandContext::RunningAIBlock { prompt } => {
            // Fuzzy search gives different weights to characters in the same word vs different words.
            searchable_string.push(' ');
            searchable_string.push_str(prompt.as_str());

            let start = prompt_end + 1;
            let end = start + prompt.chars().count();
            Some(start..end)
        }
        CommandContext::None => None,
    };

    let command_info = CommandRenderInfo::from_context(session.command_context());
    searchable_string.push(' ');
    searchable_string.push_str(command_info.hint_text.as_str());
    let hint_text_range = match &command_range {
        Some(command_range) => {
            let start = command_range.end + 1;
            let end = start + command_info.hint_text.chars().count();
            start..end
        }
        None => {
            let start = prompt_end + 1;
            let end = start + command_info.hint_text.chars().count();
            start..end
        }
    };

    (
        searchable_string,
        SearchableSessionStringRanges {
            command_range,
            hint_text_range,
        },
    )
}

struct SearchableSessionStringRanges {
    command_range: Option<Range<usize>>,
    hint_text_range: Range<usize>,
}

type SearcherAction = <DataSource as SyncDataSource>::Action;

pub trait SessionSearcher {
    fn search(
        &self,
        _search_term: &str,
        _app: &AppContext,
    ) -> anyhow::Result<Vec<QueryResult<SearcherAction>>>;

    fn active_session_id(&self, app: &AppContext) -> Option<PaneId>;
}

pub struct FuzzySessionSearcher {
    pub(crate) session_source_handle: ModelHandle<SessionSource>,
}

impl SessionSearcher for FuzzySessionSearcher {
    fn search(
        &self,
        search_term: &str,
        app: &AppContext,
    ) -> anyhow::Result<Vec<QueryResult<SearcherAction>>> {
        let active_session_id = match self.session_source_handle.as_ref(app) {
            SessionSource::None => None,
            SessionSource::Set { active_pane_id, .. } => Some(*active_pane_id),
        };

        // Sort sessions by last focus timestamp so sessions that were focused first are shown first.
        let all_sessions =
            SessionNavigationData::all_sessions(app).sorted_by_key(|x| x.last_focus_ts());

        Ok(filter_sessions(all_sessions.as_slice(), search_term)
            .map(|matched_session| SearchItem::new(matched_session, active_session_id).into())
            .collect())
    }

    fn active_session_id(&self, app: &AppContext) -> Option<PaneId> {
        match self.session_source_handle.as_ref(app) {
            SessionSource::None => None,
            SessionSource::Set { active_pane_id, .. } => Some(*active_pane_id),
        }
    }
}

#[cfg(not(target_family = "wasm"))]
pub use full_text_searcher::FullTextSessionSearcher;
#[cfg(not(target_family = "wasm"))]
mod full_text_searcher {
    use std::collections::HashMap;

    use itertools::Itertools;
    use warp_search_core::define_search_schema;
    use warpui::{AppContext, ModelHandle};

    use crate::pane_group::PaneId;
    use crate::search::command_palette::navigation::search::{
        searchable_session_string_and_ranges, MatchedSession, SearcherAction,
        SessionHighlightIndices, SessionMatchResult, SessionSearcher,
    };
    use crate::search::command_palette::navigation::search_item::SearchItem;
    use crate::search::data_source::QueryResult;
    use crate::search::searcher::{DEFAULT_MEMORY_BUDGET, SCORE_CONVERSION_FACTOR};
    use crate::session_management::{SessionNavigationData, SessionSource};

    define_search_schema!(
        schema_name: SESSION_SEARCH_SCHEMA,
        config_name: SessionSearchConfig,
        search_doc: SessionSearchDocument,
        identifying_doc: SessionIdDocument,
        search_fields: [session: 1.0],
        id_fields: [search_id: u64],
    );

    pub struct FullTextSessionSearcher {
        pub(crate) session_source_handle: ModelHandle<SessionSource>,
    }

    impl SessionSearcher for FullTextSessionSearcher {
        fn search(
            &self,
            search_term: &str,
            app: &AppContext,
        ) -> anyhow::Result<Vec<QueryResult<SearcherAction>>> {
            let searcher = SESSION_SEARCH_SCHEMA.create_searcher(DEFAULT_MEMORY_BUDGET);

            let mut sessions = HashMap::new();
            let documents =
                SessionNavigationData::all_sessions(app)
                    .enumerate()
                    .map(|(idx, session)| {
                        let (search_string, highlight) =
                            searchable_session_string_and_ranges(&session);
                        let search_id = SessionSearchId(idx);

                        sessions.insert(search_id, (session, highlight, search_string.clone()));
                        SessionSearchDocument {
                            session: search_string,
                            search_id: search_id.0 as u64,
                        }
                    });

            searcher.build_index(documents)?;

            let active_session_id = match self.session_source_handle.as_ref(app) {
                SessionSource::None => None,
                SessionSource::Set { active_pane_id, .. } => Some(*active_pane_id),
            };

            if search_term.is_empty() {
                return Ok(sessions
                    .into_iter()
                    .sorted_by_key(|(_, (session, ..))| session.last_focus_ts())
                    .map(|(_, (session, ..))| {
                        let matched_session = MatchedSession {
                            session,
                            match_result: SessionMatchResult::no_match(),
                        };
                        SearchItem::new(matched_session, active_session_id).into()
                    })
                    .collect());
            }

            let matched_sessions = searcher.search_id(search_term)?;
            Ok(matched_sessions
                .into_iter()
                .filter_map(|search_match| {
                    let (session, highlight, search_string) = sessions
                        .remove(&SessionSearchId(search_match.values.search_id as usize))?;

                    let char_indices = byte_indices_to_char_indices(
                        &search_string,
                        search_match.highlights.session,
                    );
                    let highlight_indices = SessionHighlightIndices::new(char_indices, highlight);
                    let match_result = SessionMatchResult {
                        score: (search_match.score * SCORE_CONVERSION_FACTOR) as i64,
                        highlight_indices,
                    };
                    let matched_session = MatchedSession {
                        session,
                        match_result,
                    };
                    Some(SearchItem::new(matched_session, active_session_id).into())
                })
                .collect())
        }

        fn active_session_id(&self, app: &AppContext) -> Option<PaneId> {
            match self.session_source_handle.as_ref(app) {
                SessionSource::None => None,
                SessionSource::Set { active_pane_id, .. } => Some(*active_pane_id),
            }
        }
    }

    impl FullTextSessionSearcher {
        pub fn new(session_source_handle: ModelHandle<SessionSource>) -> Self {
            Self {
                session_source_handle,
            }
        }
    }

    /// Converts byte-based indices (from Tantivy snippet highlighting) into
    /// char-based indices that align with the char-based ranges used by
    /// [`SessionHighlightIndices`].
    pub(super) fn byte_indices_to_char_indices(text: &str, byte_indices: Vec<usize>) -> Vec<usize> {
        let byte_to_char: HashMap<usize, usize> = text
            .char_indices()
            .enumerate()
            .map(|(char_idx, (byte_idx, _))| (byte_idx, char_idx))
            .collect();

        byte_indices
            .into_iter()
            .filter_map(|byte_idx| byte_to_char.get(&byte_idx).copied())
            .collect()
    }

    /// A unique identifier for a session.
    #[derive(Copy, Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
    struct SessionSearchId(usize);
}

#[cfg(all(test, not(target_family = "wasm")))]
#[path = "search_tests.rs"]
mod tests;
