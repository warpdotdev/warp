//! Data source for the inline Dev Container config selector.
use std::path::PathBuf;

use fuzzy_match::match_indices_case_insensitive;
use ordered_float::OrderedFloat;
use warpui::{AppContext, Entity};

use crate::search::SyncDataSource;
use crate::search::data_source::{Query, QueryResult};
use crate::search::mixer::DataSourceRunErrorWrapper;
use crate::terminal::input::dev_container_config::SelectDevContainerConfig;
use crate::terminal::input::dev_container_config::search_item::DevContainerConfigSearchItem;

/// Unlike most data sources, which read from a globally-available model, the set of Dev
/// Container configs is produced once by a filesystem scan when `/devcontainer` runs and is
/// specific to that invocation, so it's set directly rather than looked up per-query.
pub struct DevContainerConfigSelectorDataSource {
    configs: Vec<PathBuf>,
}

impl DevContainerConfigSelectorDataSource {
    pub fn new(configs: Vec<PathBuf>) -> Self {
        Self { configs }
    }

    pub fn set_configs(&mut self, configs: Vec<PathBuf>) {
        self.configs = configs;
    }

    /// Configs in the order the zero-state menu consumes them.
    ///
    /// Inline-menu results are consumed in ascending order of priority, so the last row is the
    /// one selected by default. Discovery walks the spec locations from most to least canonical,
    /// so that order is reversed here to make the first config found the default pick.
    fn zero_state_order(&self) -> impl Iterator<Item = &PathBuf> {
        self.configs.iter().rev()
    }
}

impl SyncDataSource for DevContainerConfigSelectorDataSource {
    type Action = SelectDevContainerConfig;

    fn run_query(
        &self,
        query: &Query,
        _app: &AppContext,
    ) -> Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper> {
        let query_text = query.text.trim().to_lowercase();
        if query_text.is_empty() {
            return Ok(self
                .zero_state_order()
                .map(|path| QueryResult::from(DevContainerConfigSearchItem::new(path.clone())))
                .collect());
        }

        Ok(self
            .configs
            .iter()
            .filter_map(|path| {
                let label = DevContainerConfigSearchItem::display_label(path);
                let match_result =
                    match_indices_case_insensitive(&label.to_lowercase(), &query_text)?;
                let score = OrderedFloat(match_result.score as f64);
                Some(QueryResult::from(
                    DevContainerConfigSearchItem::new(path.clone())
                        .with_match_result(match_result)
                        .with_score(score),
                ))
            })
            .collect())
    }
}

impl Entity for DevContainerConfigSelectorDataSource {
    type Event = ();
}

#[cfg(test)]
#[path = "data_source_tests.rs"]
mod tests;
