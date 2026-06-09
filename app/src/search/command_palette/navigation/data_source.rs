use warpui::{AppContext, Entity, ModelHandle};

use crate::search::command_palette::mixer::CommandPaletteItemAction;
use crate::search::command_palette::navigation::search::{
    FuzzySessionSearcher, MatchedSession, SessionMatchResult, SessionSearcher,
};
use crate::search::command_palette::navigation::search_item::SearchItem;
use crate::search::data_source::{DataSourceSearchError, Query, QueryResult};
use crate::search::mixer::DataSourceRunErrorWrapper;
use crate::search::SyncDataSource;
use crate::session_management::{SessionNavigationData, SessionSource};
use crate::workspace::PaneViewLocator;

/// Data source that produces possible running sessions a user could navigate to.
pub struct DataSource {
    searcher: Box<dyn SessionSearcher>,
}

impl DataSource {
    #[cfg(not(target_family = "wasm"))]
    pub fn new(active_session_handle: ModelHandle<SessionSource>) -> Self {
        if warp_core::features::FeatureFlag::UseTantivySearch.is_enabled() {
            Self::new_full_text(active_session_handle)
        } else {
            Self::new_fuzzy(active_session_handle)
        }
    }

    #[cfg(target_family = "wasm")]
    pub fn new(active_session_handle: ModelHandle<SessionSource>) -> Self {
        Self::new_fuzzy(active_session_handle)
    }

    #[cfg(not(target_family = "wasm"))]
    fn new_full_text(active_session_handle: ModelHandle<SessionSource>) -> Self {
        use crate::search::command_palette::navigation::search::FullTextSessionSearcher;
        let searcher = Box::new(FullTextSessionSearcher::new(active_session_handle));
        Self { searcher }
    }

    fn new_fuzzy(active_session_handle: ModelHandle<SessionSource>) -> Self {
        let searcher = Box::new(FuzzySessionSearcher {
            session_source_handle: active_session_handle,
        });
        Self { searcher }
    }
}

impl SyncDataSource for DataSource {
    type Action = CommandPaletteItemAction;

    fn run_query(
        &self,
        query: &Query,
        app: &AppContext,
    ) -> Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper> {
        self.searcher
            .search(&query.text.trim().to_lowercase(), app)
            .map_err(|err| {
                let search_error = DataSourceSearchError::new(err.to_string());
                Box::new(search_error) as DataSourceRunErrorWrapper
            })
    }
}

impl DataSource {
    /// Returns a [`QueryResult`] for a workflow identified by `sync_id`. `None` if no result was
    /// found with the given ID.
    pub fn query_result(
        &self,
        pane_view_locator: PaneViewLocator,
        app: &AppContext,
    ) -> Option<QueryResult<CommandPaletteItemAction>> {
        let session = SessionNavigationData::all_sessions(app)
            .find(|session| session.pane_view_locator() == pane_view_locator)?;

        let matched_session = MatchedSession {
            session,
            match_result: SessionMatchResult::no_match(),
        };

        let active_session_id = self.searcher.active_session_id(app);

        Some(SearchItem::new(matched_session, active_session_id).into())
    }
}

impl Entity for DataSource {
    type Event = ();
}
