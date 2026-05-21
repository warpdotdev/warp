use crate::search::command_palette::mixer::CommandPaletteItemAction;
use crate::search::command_palette::tabs::SearchItem;
use crate::search::data_source::{Query, QueryResult};
use crate::search::mixer::{DataSourceRunErrorWrapper, SyncDataSource};
use crate::session_management::TabNavigationData;
use warpui::{AppContext, Entity};

/// Data source that produces tabs sorted by MRU order for the Ctrl+Tab palette.
///
/// Holds a pre-computed snapshot of tab data rather than a workspace handle,
/// because the synchronous query runs while the workspace view is borrowed
/// and a `WeakViewHandle::upgrade()` would fail.
pub struct DataSource {
    tabs: Vec<TabNavigationData>,
}

impl Default for DataSource {
    fn default() -> Self {
        Self::new()
    }
}

impl DataSource {
    pub fn new() -> Self {
        Self { tabs: vec![] }
    }

    pub fn set_tabs(&mut self, tabs: Vec<TabNavigationData>) {
        self.tabs = tabs;
    }
}

impl SyncDataSource for DataSource {
    type Action = CommandPaletteItemAction;

    fn run_query(
        &self,
        query: &Query,
        _ctx: &AppContext,
    ) -> Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper> {
        let query_text = query.text.trim().to_lowercase();

        let results = self
            .tabs
            .iter()
            .enumerate()
            .filter(|(_, tab)| {
                query_text.is_empty()
                    || tab.title.to_lowercase().contains(&query_text)
                    || tab
                        .subtitle
                        .as_deref()
                        .is_some_and(|s| s.to_lowercase().contains(&query_text))
            })
            .map(|(i, tab)| QueryResult::from(SearchItem::new(tab.clone(), i)))
            .collect();

        Ok(results)
    }
}

impl Entity for DataSource {
    type Event = ();
}
