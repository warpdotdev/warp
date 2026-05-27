use std::collections::HashMap;
use std::sync::Arc;

use fuzzy_match::match_indices_case_insensitive;
use warpui::{AppContext, Entity, ModelContext, SingletonEntity};

use crate::launch_configs::launch_config::LaunchConfig;
use crate::search::command_palette::launch_config::search_item::SearchItem;
use crate::search::command_palette::mixer::CommandPaletteItemAction;
use crate::search::data_source::{DataSourceSearchError, Query, QueryResult};
use crate::search::mixer::{DataSourceRunErrorWrapper, SyncDataSource};
use crate::user_config::{WarpConfig, WarpConfigUpdateEvent};

/// Datasource that searches against `LaunchConfig`s.
pub struct DataSource {
    searcher: Box<dyn LaunchConfigSearcher>,
}

impl DataSource {
    #[cfg(not(target_family = "wasm"))]
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        if warp_core::features::FeatureFlag::UseTantivySearch.is_enabled() {
            Self::new_full_text(ctx)
        } else {
            Self::new_fuzzy(ctx)
        }
    }

    #[cfg(target_family = "wasm")]
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        Self::new_fuzzy(ctx)
    }

    fn new_fuzzy(ctx: &mut ModelContext<Self>) -> Self {
        ctx.subscribe_to_model(&WarpConfig::handle(ctx), Self::handle_config_event);
        let mut searcher = Box::new(FuzzyLaunchConfigSearcher::default());
        searcher.refresh_search_index(ctx);
        Self { searcher }
    }

    #[cfg(not(target_family = "wasm"))]
    fn new_full_text(ctx: &mut ModelContext<Self>) -> Self {
        ctx.subscribe_to_model(&WarpConfig::handle(ctx), Self::handle_config_event);
        let mut searcher = Box::new(full_text_searcher::FullTextLaunchConfigSearcher::new(
            ctx.background_executor(),
        ));
        searcher.refresh_search_index(ctx);
        Self { searcher }
    }

    fn handle_config_event(&mut self, event: &WarpConfigUpdateEvent, ctx: &mut ModelContext<Self>) {
        if matches!(event, WarpConfigUpdateEvent::LaunchConfigs) {
            self.searcher.refresh_search_index(ctx);
        }
    }
}
impl SyncDataSource for DataSource {
    type Action = CommandPaletteItemAction;

    fn run_query(
        &self,
        query: &Query,
        _app: &AppContext,
    ) -> Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper> {
        Ok(self
            .searcher
            .search(&query.text.trim().to_lowercase())
            .map_err(|err| {
                Box::new(DataSourceSearchError {
                    message: err.to_string(),
                }) as DataSourceRunErrorWrapper
            })?
            .into_iter()
            .map(QueryResult::from)
            .collect())
    }
}

impl Entity for DataSource {
    type Event = ();
}

trait LaunchConfigSearcher {
    fn search(&self, search_term: &str) -> anyhow::Result<Vec<SearchItem>>;

    fn refresh_search_index(&mut self, app: &AppContext);
}

#[derive(Default)]
struct FuzzyLaunchConfigSearcher {
    configs: HashMap<String, LaunchConfig>,
}

impl LaunchConfigSearcher for FuzzyLaunchConfigSearcher {
    fn search(&self, search_term: &str) -> anyhow::Result<Vec<SearchItem>> {
        Ok(self
            .configs
            .values()
            .filter_map(|launch_config| {
                let match_result =
                    match_indices_case_insensitive(&launch_config.name, search_term)?;

                Some(SearchItem::new(
                    Arc::new(launch_config.clone()),
                    match_result,
                ))
            })
            .collect())
    }

    fn refresh_search_index(&mut self, app: &AppContext) {
        self.configs = WarpConfig::as_ref(app)
            .launch_configs()
            .iter()
            .map(|config| (config.name.to_lowercase(), config.clone()))
            .collect();
    }
}

#[cfg(not(target_family = "wasm"))]
mod full_text_searcher {
    use std::collections::HashMap;
    use std::sync::Arc;

    use fuzzy_match::FuzzyMatchResult;
    use warpui::r#async::executor::Background;
    use warpui::{AppContext, SingletonEntity};

    use crate::define_search_schema;
    use crate::launch_configs::launch_config::LaunchConfig;
    use crate::search::command_palette::launch_config::data_source::LaunchConfigSearcher;
    use crate::search::command_palette::launch_config::search_item::SearchItem;
    use crate::search::searcher::{AsyncSearcher, DEFAULT_MEMORY_BUDGET, SCORE_CONVERSION_FACTOR};
    use crate::user_config::WarpConfig;

    // The name of the launch configs are duplicated to ensure that the searcher
    // hashes the name to uniquely identify the launch config.
    // Also, it makes sense from a schema POV: the name is the identifying key.
    // TODO: Add a proper Launch Config ID
    define_search_schema!(
        schema_name: LAUNCH_CONFIG_SCHEMA,
        config_name: ConfigSearcherConfig,
        search_doc: LaunchConfigDocument,
        identifying_doc: LaunchConfigIdDocument,
        search_fields: [name: 1.0],
        id_fields: [name_id: String]
    );

    pub(crate) struct FullTextLaunchConfigSearcher {
        background_executor: Arc<Background>,
        searcher: AsyncSearcher<ConfigSearcherConfig>,
        configs: HashMap<String, LaunchConfig>,
    }

    impl LaunchConfigSearcher for FullTextLaunchConfigSearcher {
        fn search(&self, search_term: &str) -> anyhow::Result<Vec<SearchItem>> {
            if search_term.is_empty() {
                return Ok(self
                    .configs
                    .values()
                    .map(|config| {
                        SearchItem::new(Arc::new(config.clone()), FuzzyMatchResult::no_match())
                    })
                    .collect());
            }

            Ok(self
                .searcher
                .search_id(search_term)?
                .into_iter()
                .filter_map(|match_result| {
                    let launch_config = self.configs.get(&match_result.values.name_id)?;
                    let match_result = FuzzyMatchResult {
                        score: (match_result.score * SCORE_CONVERSION_FACTOR) as i64,
                        matched_indices: match_result.highlights.name,
                    };

                    Some(SearchItem::new(
                        Arc::new(launch_config.clone()),
                        match_result,
                    ))
                })
                .collect())
        }

        fn refresh_search_index(&mut self, app: &AppContext) {
            self.configs = WarpConfig::as_ref(app)
                .launch_configs()
                .iter()
                .map(|config| (config.name.to_lowercase(), config.clone()))
                .collect();
            if self.rebuild_search_index().is_err() {
                log::error!("Failed to create search index writer for launch configs");
                self.clear_search_index();
            }
        }
    }

    impl FullTextLaunchConfigSearcher {
        pub(crate) fn new(background_executor: Arc<Background>) -> Self {
            Self {
                background_executor: background_executor.clone(),
                searcher: LAUNCH_CONFIG_SCHEMA
                    .create_async_searcher(DEFAULT_MEMORY_BUDGET, background_executor),
                configs: Default::default(),
            }
        }

        fn rebuild_search_index(&mut self) -> Result<(), anyhow::Error> {
            self.clear_search_index();
            let documents = self.configs.keys().map(|name| LaunchConfigDocument {
                name: name.clone(),
                name_id: name.clone(),
            });
            self.searcher.build_index_async(documents)
        }

        fn clear_search_index(&mut self) {
            if self.searcher.clear_search_index_async().is_err() {
                // As a workaround, we can create a new index and replace the old one.
                self.searcher = LAUNCH_CONFIG_SCHEMA
                    .create_async_searcher(DEFAULT_MEMORY_BUDGET, self.background_executor.clone());
            }
        }
    }
}
