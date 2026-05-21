use super::new_session_option::{Direction, NewSessionConfig};
use super::new_session_option::{NewSessionOption, NewSessionOptionId};
use super::search_item::SearchItem;
use crate::search::data_source::DataSourceSearchError;
use crate::search::{
    binding_source::BindingSource,
    command_palette::mixer::CommandPaletteItemAction,
    data_source::{Query, QueryResult},
    mixer::{DataSourceRunErrorWrapper, SyncDataSource},
};
use crate::terminal::available_shells::AvailableShells;
use fuzzy_match::{match_indices_case_insensitive, FuzzyMatchResult};
use std::collections::HashMap;
use std::sync::Arc;
use warp_core::features::FeatureFlag;
use warpui::{AppContext, Entity, ModelContext, ModelHandle, SingletonEntity};

/// Controls which kinds of new sessions the data source should surface.
#[derive(Copy, Clone, Debug)]
pub struct AllowedSessionKinds {
    pub windows: bool,
    pub tabs: bool,
    pub panes: bool,
}

impl Default for AllowedSessionKinds {
    fn default() -> Self {
        Self {
            windows: true,
            tabs: true,
            panes: true,
        }
    }
}

impl AllowedSessionKinds {
    pub fn tabs_only() -> Self {
        Self {
            windows: false,
            tabs: true,
            panes: false,
        }
    }
}

/// A data source that provides options for creating new terminal sessions
/// Gathers this data by:
/// - Listening for any binding source changes
/// - Comparing the options in binding sources (open new tab, open new window, etc.)
///   to the list of available shells, and creates an intersection of those items.
pub struct NewSessionDataSource {
    searcher: Box<dyn NewSessionSearcher>,
    allowed: AllowedSessionKinds,
}

impl NewSessionDataSource {
    #[cfg(not(target_family = "wasm"))]
    pub fn new(binding_source: ModelHandle<BindingSource>, ctx: &mut ModelContext<Self>) -> Self {
        if FeatureFlag::UseTantivySearch.is_enabled() {
            Self::new_full_text(binding_source, ctx)
        } else {
            Self::new_fuzzy(binding_source, ctx)
        }
    }

    #[cfg(target_family = "wasm")]
    pub fn new(binding_source: ModelHandle<BindingSource>, ctx: &mut ModelContext<Self>) -> Self {
        Self::new_fuzzy(binding_source, ctx)
    }

    fn new_fuzzy(binding_source: ModelHandle<BindingSource>, ctx: &mut ModelContext<Self>) -> Self {
        ctx.observe(&binding_source, Self::on_binding_source_changed);
        Self {
            searcher: Box::new(FuzzyNewSessionSearcher::default()),
            allowed: Default::default(),
        }
    }

    #[cfg(not(target_family = "wasm"))]
    fn new_full_text(
        binding_source: ModelHandle<BindingSource>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        ctx.observe(&binding_source, Self::on_binding_source_changed);
        Self {
            searcher: Box::new(full_text_searcher::FullTextNewSessionSearcher::new(
                ctx.background_executor(),
            )),
            allowed: Default::default(),
        }
    }

    pub fn with_allowed_kinds(mut self, allowed: AllowedSessionKinds) -> Self {
        self.allowed = allowed;
        self
    }

    fn on_binding_source_changed(
        &mut self,
        source: ModelHandle<BindingSource>,
        ctx: &mut ModelContext<Self>,
    ) {
        if !FeatureFlag::ShellSelector.is_enabled() {
            return;
        }

        let (window_id, view_id) = match source.as_ref(ctx) {
            BindingSource::None => return,
            BindingSource::View {
                window_id, view_id, ..
            } => (*window_id, *view_id),
        };

        let shell_id_to_options = self.searcher.bindings_mut();

        let mut has_tabs = false;
        let mut has_panes = false;
        for lens in ctx.key_bindings_for_view(window_id, view_id) {
            match lens.name {
                "workspace:new_tab" => has_tabs = true,
                "pane_group:add_down" => has_panes = true,
                _ => (),
            }
        }

        shell_id_to_options.clear();

        for shell in AvailableShells::as_ref(ctx).get_available_shells() {
            let Some(id_str) = shell.id() else { continue };

            if self.allowed.windows {
                let id = NewSessionOptionId::new(format!("new_window:{id_str}"));
                let new_option = Arc::new(NewSessionOption::new(
                    id.clone(),
                    NewSessionConfig::NewWindow(shell.clone()),
                ));
                shell_id_to_options.insert(id, new_option);
            }

            if self.allowed.tabs && has_tabs {
                let id = NewSessionOptionId::new(format!("new_tab:{id_str}"));
                let new_option = Arc::new(NewSessionOption::new(
                    id.clone(),
                    NewSessionConfig::NewTab(shell.clone()),
                ));
                shell_id_to_options.insert(id, new_option);
            }

            if self.allowed.panes && has_panes {
                for (id_str, direction) in [
                    (format!("split_down:{id_str}"), Direction::Down),
                    (format!("split_right:{id_str}"), Direction::Right),
                    (format!("split_up:{id_str}"), Direction::Up),
                    (format!("split_left:{id_str}"), Direction::Left),
                ] {
                    let id = NewSessionOptionId::new(id_str);
                    let new_option = Arc::new(NewSessionOption::new(
                        id.clone(),
                        NewSessionConfig::Split(direction, shell.clone()),
                    ));
                    shell_id_to_options.insert(id, new_option);
                }
            }
        }

        self.searcher.build_index();
    }

    pub fn query_result(
        &self,
        id: &NewSessionOptionId,
    ) -> Option<QueryResult<CommandPaletteItemAction>> {
        self.searcher
            .bindings()
            .get(id)
            .map(|option| SearchItem::new(option.clone(), FuzzyMatchResult::no_match()).into())
    }
}

impl SyncDataSource for NewSessionDataSource {
    type Action = CommandPaletteItemAction;

    /// Does a fuzzy search on the descriptions of the new session options.
    /// Logic is mostly copied from actions/data_source.rs
    fn run_query(
        &self,
        query: &Query,
        _app: &AppContext,
    ) -> Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper> {
        let search_term = query.text.as_str();
        self.searcher.search(search_term).map_err(|err| {
            let search_error = DataSourceSearchError {
                message: err.to_string(),
            };
            Box::new(search_error) as DataSourceRunErrorWrapper
        })
    }
}

impl Entity for NewSessionDataSource {
    type Event = ();
}

type SearcherAction = <NewSessionDataSource as SyncDataSource>::Action;

const SEARCHER_BASE_STRINGS: [&str; 6] = [
    "Create New Tab",
    "Create New Window",
    "Split Pane Down",
    "Split Pane Right",
    "Split Pane Up",
    "Split Pane Left",
];

trait NewSessionSearcher {
    fn search(&self, _search_term: &str) -> anyhow::Result<Vec<QueryResult<SearcherAction>>>;

    fn build_index(&mut self);

    fn bindings(&self) -> &HashMap<NewSessionOptionId, Arc<NewSessionOption>>;
    fn bindings_mut(&mut self) -> &mut HashMap<NewSessionOptionId, Arc<NewSessionOption>>;

    /// Computes the maximum match score for the given query string given
    /// the "base options". We want to make sure that the default command
    /// for any given variant is listed before the variant. Ex:
    ///   "Create New Tab" should always be ranked higher than
    ///   "Create New Tab: Zsh"
    /// This function computes the lowest possible ranking score
    /// for any base strings that match the query. All variant
    /// matches should have this value as a ceiling.
    fn compute_max_match(&self, query_str: &str) -> Option<f64>;
}
#[derive(Default)]
struct FuzzyNewSessionSearcher {
    shell_id_to_options: HashMap<NewSessionOptionId, Arc<NewSessionOption>>,
}

impl NewSessionSearcher for FuzzyNewSessionSearcher {
    fn search(&self, search_term: &str) -> anyhow::Result<Vec<QueryResult<SearcherAction>>> {
        let max_match = self.compute_max_match(search_term);

        Ok(self
            .shell_id_to_options
            .values()
            .filter_map(move |new_session_option| {
                // Binding descriptions are almost always upper case. If a user searches with
                // lowercase text, the fuzzy matcher will weight this match lower because the case
                // between the search term and the description differ. As a result, we lowercase
                // both the search term and the description to ensure that we are matching the two
                // with the same casing.
                match_indices_case_insensitive(
                    new_session_option.description().to_lowercase().as_str(),
                    search_term.to_lowercase().as_str(),
                )
                .map(|result| {
                    // If for some reason the variant (ex: "Create New Tab: Powershell") ranks higher
                    // than a match for a base string (ex: "Create New Tab"), we want to cap the score
                    // to be one less than the base string.
                    if let Some(max_match) = max_match {
                        FuzzyMatchResult {
                            score: std::cmp::min(result.score, max_match.round() as i64 - 1),
                            matched_indices: result.matched_indices,
                        }
                    } else {
                        result
                    }
                })
                .map(|result| (result, new_session_option))
            })
            .map(|(match_result, new_session_config)| {
                SearchItem::new(new_session_config.clone(), match_result).into()
            })
            .collect())
    }

    /// This method is a no-op for the fuzzy searcher since it does not maintain an index.
    fn build_index(&mut self) {}

    fn bindings(&self) -> &HashMap<NewSessionOptionId, Arc<NewSessionOption>> {
        &self.shell_id_to_options
    }

    fn bindings_mut(&mut self) -> &mut HashMap<NewSessionOptionId, Arc<NewSessionOption>> {
        &mut self.shell_id_to_options
    }

    fn compute_max_match(&self, query_str: &str) -> Option<f64> {
        SEARCHER_BASE_STRINGS
            .iter()
            .filter_map(|base| {
                match_indices_case_insensitive(
                    base.to_lowercase().as_str(),
                    query_str.to_lowercase().as_str(),
                )
                .map(|result| result.score)
            })
            .min()
            .map(|score| score as f64)
    }
}

#[cfg(not(target_family = "wasm"))]
mod full_text_searcher {
    use crate::define_search_schema;
    use crate::search::command_palette::new_session::data_source::{
        NewSessionSearcher, SearcherAction, SEARCHER_BASE_STRINGS,
    };
    use crate::search::command_palette::new_session::search_item::SearchItem;
    use crate::search::command_palette::new_session::{NewSessionOption, NewSessionOptionId};
    use crate::search::data_source::QueryResult;
    use crate::search::searcher::{
        AsyncSearcher, DEFAULT_MEMORY_BUDGET, MIN_MEMORY_BUDGET, SCORE_CONVERSION_FACTOR,
    };
    use fuzzy_match::FuzzyMatchResult;
    use std::collections::HashMap;
    use std::sync::Arc;
    use warpui::r#async::executor::Background;

    define_search_schema!(
        schema_name: NEW_SESSION_SEARCH_SCHEMA,
        config_name: NewSessionConfig,
        search_doc: NewSessionDocument,
        identifying_doc: NewSessionIdDocument,
        search_fields: [new_session_option: 1.0],
        id_fields: [id: String]
    );
    define_search_schema!(
        schema_name: BASE_TEXT_SEARCH_SCHEMA,
        config_name: BaseTextConfig,
        search_doc: BaseTextDocument,
        identifying_doc: BaseTextIdDocument,
        search_fields: [base_text: 1.0],
        id_fields: []
    );

    pub(crate) struct FullTextNewSessionSearcher {
        background_executor: Arc<Background>,
        searcher: AsyncSearcher<NewSessionConfig>,
        max_match_searcher: AsyncSearcher<BaseTextConfig>,
        shell_id_to_options: HashMap<NewSessionOptionId, Arc<NewSessionOption>>,
    }

    impl NewSessionSearcher for FullTextNewSessionSearcher {
        fn search(&self, search_term: &str) -> anyhow::Result<Vec<QueryResult<SearcherAction>>> {
            let max_match = self.compute_max_match(search_term);
            let search_result = self.searcher.search_id(search_term)?;
            Ok(search_result
                .into_iter()
                .filter_map(|result| {
                    let matched_indices = result.highlights.new_session_option;
                    let new_session_option = self
                        .shell_id_to_options
                        .get(&NewSessionOptionId(result.values.id))?;

                    // If for some reason the variant (ex: "Create New Tab: Powershell") ranks higher
                    // than a match for a base string (ex: "Create New Tab"), we want to cap the score
                    // to be one less than the base string.
                    let capped_score = Self::cap_score(result.score, max_match);

                    Some(
                        SearchItem::new(
                            new_session_option.clone(),
                            FuzzyMatchResult {
                                score: (capped_score * SCORE_CONVERSION_FACTOR) as i64,
                                matched_indices,
                            },
                        )
                        .into(),
                    )
                })
                .collect())
        }

        fn build_index(&mut self) {
            if self.rebuild_search_index().is_err() {
                log::error!("Failed to create search index writer for new session options");
                self.clear_search_index();
            }
        }

        fn bindings(&self) -> &HashMap<NewSessionOptionId, Arc<NewSessionOption>> {
            &self.shell_id_to_options
        }

        fn bindings_mut(&mut self) -> &mut HashMap<NewSessionOptionId, Arc<NewSessionOption>> {
            &mut self.shell_id_to_options
        }

        fn compute_max_match(&self, query_str: &str) -> Option<f64> {
            self.max_match_searcher
                .search_id(query_str)
                .ok()?
                .iter()
                .map(|result| result.score)
                .reduce(|min, score| if score < min { score } else { min })
        }
    }

    impl FullTextNewSessionSearcher {
        pub(crate) fn new(background_executor: Arc<Background>) -> Self {
            let searcher = NEW_SESSION_SEARCH_SCHEMA
                .create_async_searcher(DEFAULT_MEMORY_BUDGET, background_executor.clone());
            let mut max_match_searcher = BASE_TEXT_SEARCH_SCHEMA
                .create_async_searcher(MIN_MEMORY_BUDGET, background_executor.clone());
            let max_match_documents = SEARCHER_BASE_STRINGS.iter().map(|base| BaseTextDocument {
                base_text: base.to_string(),
            });
            if max_match_searcher
                .build_index_async(max_match_documents)
                .is_err()
            {
                log::error!("Failed to build search index for base text of new session search");
                if max_match_searcher.clear_search_index_async().is_err() {
                    max_match_searcher = BASE_TEXT_SEARCH_SCHEMA
                        .create_async_searcher(MIN_MEMORY_BUDGET, background_executor.clone())
                }
            }

            Self {
                background_executor,
                searcher,
                max_match_searcher,
                shell_id_to_options: HashMap::new(),
            }
        }

        fn rebuild_search_index(&mut self) -> Result<(), anyhow::Error> {
            self.clear_search_index();
            let documents = self.shell_id_to_options.iter().map(|(id, option)| {
                let binding_description = option.description().to_lowercase();

                NewSessionDocument {
                    new_session_option: binding_description.clone(),
                    id: id.0.clone(),
                }
            });
            self.searcher.build_index_async(documents)
        }

        fn clear_search_index(&mut self) {
            if self.searcher.clear_search_index_async().is_err() {
                // As a workaround, we can create a new index and replace the old one.
                self.searcher = NEW_SESSION_SEARCH_SCHEMA
                    .create_async_searcher(DEFAULT_MEMORY_BUDGET, self.background_executor.clone());
            }
        }

        fn cap_score(score: f64, max_match_score: Option<f64>) -> f64 {
            if let Some(max_match) = max_match_score {
                // The use of 0.02 comes from the fact that fuzzy search scores are reduced by 1 in this case,
                // and we boosted the Tantivy score by a factor of 50 to roughly match the fuzzy search scores.
                if score > max_match - 0.02 {
                    max_match - 0.02
                } else {
                    score
                }
            } else {
                score
            }
        }
    }
}
